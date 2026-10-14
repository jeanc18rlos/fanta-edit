//! Source-preserving FNX mirror.
//!
//! The semantic [`FnxElement`](crate::FnxElement) tree is the editable virtual
//! DOM. This module keeps the original source split into immutable trivia/body
//! pieces and stable opening/closing-tag slots keyed by sidecar identity.
//! Updating one node replaces only its tag slots; comments, wrapper code,
//! whitespace, and every unrelated node remain byte-for-byte unchanged.

use crate::api::FnxSidecar;
use crate::convert::FnxError;
use crate::ir::{ArtifactIr, PreparedStructuralPatch};
use crate::model::{FnxElement, SUGAR_TAGS};
use crate::print::{render_attr, render_close_tag, render_open_tag_with_children};
use crate::refs::RefTable;
use crate::sugar::shape_sugar_spelling;
use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
struct NodePieces {
    open: usize,
    close: Option<usize>,
}

/// Retained concrete source for one FNX artifact.
///
/// Rendering concatenates the stored pieces. A node-property edit changes only
/// the opening tag piece (and the closing tag when the node type changes), so
/// repeated canvas operations do not reprint the complete file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FnxSourceMirror {
    // Candidate structures share untouched tags, including large vector paths.
    pieces: Vec<Arc<str>>,
    nodes: BTreeMap<String, NodePieces>,
    /// The name↔id context this mirror re-sugars reference spellings with when
    /// a canvas patch reprints an attribute: a recolor that swaps a bound
    /// variable id prints the `$Collection/Name` path (and a component swap
    /// prints the new component's name) when the table opted into
    /// `emit_names`. Shared via `Arc` so the owning session can rebuild one
    /// table on variable changes and hand every retained mirror the same one.
    refs: Arc<RefTable>,
    original_refs: Arc<RefTable>,
    // Registry renames change future spelling without changing the meaning of
    // retained attributes. Store only rewritten references, not a registry
    // snapshot for every edited tag.
    reference_values: BTreeMap<String, BTreeMap<&'static str, serde_json::Value>>,
    mutation_token: Arc<()>,
}

/// Validate attribute JSON and check explicit identities and structural
/// fingerprints without retaining the other node attributes. A false result
/// requires normal sidecar reconciliation, including legacy fingerprints.
pub fn sidecar_matches_source(source: &str, sidecar: &FnxSidecar) -> Result<bool, FnxError> {
    let spans = SourceScanner::new(source).scan()?;
    if spans.len() != sidecar.ids.len() {
        return Ok(false);
    }
    let mut parents = Vec::<(usize, usize)>::new();
    let mut identities = std::collections::BTreeSet::new();
    for (position, (span, entry)) in spans.iter().zip(&sidecar.ids).enumerate() {
        while parents
            .last()
            .is_some_and(|(_, close)| span.open.start >= *close)
        {
            parents.pop();
        }
        let parent = parents
            .last()
            .map(|(parent, _)| u32::try_from(*parent))
            .transpose()
            .map_err(|error| FnxError::Parse(error.to_string()))?;
        let opening = source
            .get(span.open.clone())
            .ok_or_else(|| FnxError::Parse("invalid opening-tag span".into()))?;
        let layout = OpenTagLayout::parse(opening)?;
        let shape_sugar = matches!(layout.tag.as_str(), "Rect" | "Ellipse");
        let canonical_tag = if shape_sugar {
            "Vector"
        } else {
            layout.tag.as_str()
        };
        if crate::model::type_for_tag(canonical_tag).is_none()
            || entry.tag.as_deref() != Some(canonical_tag)
            || entry.parent_index != parent
            || (layout.attrs.contains_key("transform")
                && (layout.attrs.contains_key("x") || layout.attrs.contains_key("y")))
        {
            return Ok(false);
        }
        for attribute in layout.attrs.values() {
            let value = opening
                .get(attribute.value.clone())
                .ok_or_else(|| FnxError::Parse("invalid attribute span".into()))?;
            if let Some(value) = value
                .strip_prefix('{')
                .and_then(|value| value.strip_suffix('}'))
            {
                if value.contains("fnxColor") || value.contains('#') {
                    let expanded = crate::color::expand_color_literals(value);
                    serde_json::from_str::<ValidatedJson>(&expanded)
                } else {
                    serde_json::from_str::<ValidatedJson>(value)
                }
            } else {
                serde_json::from_str::<ValidatedJson>(value)
            }
            .map_err(|error| FnxError::Parse(error.to_string()))?;
        }
        if shape_sugar {
            // Shape desugaring leaves identities and structure unchanged, but
            // rejects an authored path or dimensions that are not finite numbers.
            if layout.attrs.contains_key("path") {
                return Ok(false);
            }
            for key in ["width", "height"] {
                let Some(attribute) = layout.attrs.get(key) else {
                    return Ok(false);
                };
                let value = opening
                    .get(attribute.value.clone())
                    .ok_or_else(|| FnxError::Parse("invalid attribute span".into()))?;
                let value = value
                    .strip_prefix('{')
                    .and_then(|value| value.strip_suffix('}'))
                    .unwrap_or(value);
                if !serde_json::from_str::<f64>(value).is_ok_and(f64::is_finite) {
                    return Ok(false);
                }
            }
        }
        let string_attribute = |key: &str| -> Result<Option<String>, FnxError> {
            let Some(attribute) = layout.attrs.get(key) else {
                return Ok(None);
            };
            let value = opening
                .get(attribute.value.clone())
                .ok_or_else(|| FnxError::Parse("invalid attribute span".into()))?;
            let value = value
                .strip_prefix('{')
                .and_then(|value| value.strip_suffix('}'))
                .unwrap_or(value);
            serde_json::from_str::<String>(value)
                .map(Some)
                .map_err(|error| FnxError::Parse(error.to_string()))
        };
        if string_attribute("id")?.as_deref() != Some(entry.id.as_str())
            || string_attribute("name")? != entry.name
            || !identities.insert(entry.id.as_str())
        {
            return Ok(false);
        }
        if let Some(close) = &span.close {
            parents.push((position, close.start));
        }
    }
    Ok(true)
}

/// Decode only component references and variable bindings for a checked
/// source/identity pair. Large unrelated attribute payloads remain borrowed.
pub fn source_reference_nodes_with(
    source: &str,
    sidecar: &FnxSidecar,
    refs: &RefTable,
) -> Result<Vec<(String, FnxElement)>, FnxError> {
    let spans = SourceScanner::new(source).scan()?;
    if spans.len() != sidecar.ids.len() {
        return Err(FnxError::SidecarMismatch {
            sidecar: sidecar.ids.len(),
            elements: spans.len(),
        });
    }
    let mut references = Vec::new();
    for (span, entry) in spans.iter().zip(&sidecar.ids) {
        let opening = source
            .get(span.open.clone())
            .ok_or_else(|| FnxError::Parse("invalid opening-tag span".into()))?;
        let layout = OpenTagLayout::parse(opening)?;
        let mut element = FnxElement::new(&layout.tag);
        for key in ["component", "bindings"] {
            let Some(attribute) = layout.attrs.get(key) else {
                continue;
            };
            let spelling = opening
                .get(attribute.value.clone())
                .ok_or_else(|| FnxError::Parse("reference attribute span is absent".into()))?;
            let spelling = spelling
                .strip_prefix('{')
                .and_then(|value| value.strip_suffix('}'))
                .unwrap_or(spelling);
            if key == "component" && !spelling.trim_start().starts_with('"') {
                continue;
            }
            let value: serde_json::Value = serde_json::from_str(spelling)
                .map_err(|error| FnxError::Parse(error.to_string()))?;
            if key == "component" && !value.is_string() {
                continue;
            }
            element.attrs.insert(key.into(), value);
        }
        if !element.attrs.is_empty() {
            crate::refs::desugar_refs(&mut element, refs)?;
            references.push((entry.id.clone(), element));
        }
    }
    Ok(references)
}

struct ValidatedJson;

impl<'de> serde::Deserialize<'de> for ValidatedJson {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // IgnoredAny skips numeric range and string decoding checks. Use the
        // same scalar parsing as Value without retaining arrays or objects.
        deserializer.deserialize_any(Self)
    }
}

impl<'de> serde::de::Visitor<'de> for ValidatedJson {
    type Value = Self;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E: serde::de::Error>(self, _value: bool) -> Result<Self, E> {
        Ok(Self)
    }

    fn visit_i64<E: serde::de::Error>(self, _value: i64) -> Result<Self, E> {
        Ok(Self)
    }

    fn visit_u64<E: serde::de::Error>(self, _value: u64) -> Result<Self, E> {
        Ok(Self)
    }

    fn visit_f64<E: serde::de::Error>(self, _value: f64) -> Result<Self, E> {
        Ok(Self)
    }

    fn visit_str<E: serde::de::Error>(self, _value: &str) -> Result<Self, E> {
        Ok(Self)
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<Self, E> {
        Ok(Self)
    }

    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut sequence: A) -> Result<Self, A::Error> {
        while sequence.next_element::<Self>()?.is_some() {}
        Ok(Self)
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Self, A::Error> {
        while map.next_key::<Self>()?.is_some() {
            map.next_value::<Self>()?;
        }
        Ok(Self)
    }
}

impl FnxSourceMirror {
    /// Index `source` by the stable pre-order ids in `sidecar`, with no
    /// reference context: patches print raw ULIDs, exactly as before.
    pub fn from_source(source: &str, sidecar: &FnxSidecar) -> Result<Self, FnxError> {
        Self::from_source_with(source, sidecar, Arc::new(RefTable::default()))
    }

    /// [`from_source`](Self::from_source) with a name↔id context retained for
    /// the lifetime of the mirror (see the `refs` field).
    pub fn from_source_with(
        source: &str,
        sidecar: &FnxSidecar,
        refs: Arc<RefTable>,
    ) -> Result<Self, FnxError> {
        let mut ids = std::collections::BTreeSet::new();
        for entry in &sidecar.ids {
            if !ids.insert(entry.id.as_str()) {
                return Err(FnxError::Parse(format!(
                    "duplicate sidecar node id {}",
                    entry.id
                )));
            }
        }
        let spans = SourceScanner::new(source).scan()?;
        if spans.len() != sidecar.ids.len() {
            return Err(FnxError::SidecarMismatch {
                sidecar: sidecar.ids.len(),
                elements: spans.len(),
            });
        }

        let mut tokens = Vec::with_capacity(spans.len() * 2);
        for (entry, span) in sidecar.ids.iter().zip(spans) {
            tokens.push(SourceToken {
                range: span.open,
                id: entry.id.clone(),
                kind: TokenKind::Open,
            });
            if let Some(close) = span.close {
                tokens.push(SourceToken {
                    range: close,
                    id: entry.id.clone(),
                    kind: TokenKind::Close,
                });
            }
        }
        tokens.sort_by_key(|token| token.range.start);

        let mut pieces = Vec::with_capacity(tokens.len() * 2 + 1);
        let mut nodes = BTreeMap::<String, NodePieces>::new();
        let mut cursor = 0;
        for token in tokens {
            if token.range.start < cursor || token.range.end > source.len() {
                return Err(FnxError::Parse(
                    "overlapping or out-of-range FNX source token".into(),
                ));
            }
            pieces.push(Arc::from(&source[cursor..token.range.start]));
            let piece = pieces.len();
            pieces.push(Arc::from(&source[token.range.clone()]));
            let node = nodes.entry(token.id).or_insert(NodePieces {
                open: usize::MAX,
                close: None,
            });
            match token.kind {
                TokenKind::Open => node.open = piece,
                TokenKind::Close => node.close = Some(piece),
            }
            cursor = token.range.end;
        }
        pieces.push(Arc::from(&source[cursor..]));
        if nodes.values().any(|node| node.open == usize::MAX) {
            return Err(FnxError::Parse(
                "FNX source index contains a node without an opening tag".into(),
            ));
        }
        Ok(Self {
            pieces,
            nodes,
            original_refs: Arc::clone(&refs),
            refs,
            reference_values: BTreeMap::new(),
            mutation_token: Arc::new(()),
        })
    }

    /// Replace the retained name↔id context. Called when the owning session's
    /// component library / variable registry changes (a rename can make a
    /// spelling ambiguous or newly available); already-written pieces are
    /// untouched — the table only shapes FUTURE patches.
    pub fn set_ref_table(&mut self, refs: Arc<RefTable>) {
        self.refs = refs;
    }

    /// Concrete-source writers and retained-reference changes replace this
    /// token; clones share it until their source actually changes.
    pub fn synchronization_token(&self) -> Arc<()> {
        Arc::clone(&self.mutation_token)
    }

    /// Reparse every retained attribute using the vocabulary that authored its
    /// reference spelling. This exceptional check also covers source-owned
    /// fields that a typed document projection intentionally omits.
    pub fn semantics_match(&self, ir: &ArtifactIr) -> Result<bool, FnxError> {
        let mut root = crate::parse::parse_doc(&self.render())?;
        let mut pending = vec![&mut root];
        let mut entries = ir.sidecar().ids.iter();
        while let Some(element) = pending.pop() {
            let Some(entry) = entries.next() else {
                return Ok(false);
            };
            element
                .attrs
                .entry("id".into())
                .or_insert_with(|| entry.id.clone().into());
            self.resolve_retained_references(&entry.id, element)?;
            pending.extend(element.children.iter_mut().rev());
        }
        Ok(entries.next().is_none() && root == *ir.root())
    }

    /// Verify that a fresh reader using today's registry will recover the
    /// retained reference ids, even when an old display name was reused.
    pub fn references_match_current_table(&self) -> Result<bool, FnxError> {
        let unchanged_vocabulary = Arc::ptr_eq(&self.refs, &self.original_refs);
        for (id, node) in &self.nodes {
            if unchanged_vocabulary && !self.reference_values.contains_key(id) {
                continue;
            }
            let opening = self
                .pieces
                .get(node.open)
                .ok_or_else(|| FnxError::Parse("retained opening is absent".into()))?;
            let layout = OpenTagLayout::parse(opening)?;
            for key in ["component", "bindings"] {
                let Some(attribute) = layout.attrs.get(key) else {
                    continue;
                };
                let spelling = opening
                    .get(attribute.value.clone())
                    .ok_or_else(|| FnxError::Parse("reference attribute span is absent".into()))?;
                let spelling = spelling
                    .strip_prefix('{')
                    .and_then(|value| value.strip_suffix('}'))
                    .unwrap_or(spelling);
                if key == "component" && !spelling.trim_start().starts_with('"') {
                    continue;
                }
                let value: serde_json::Value = serde_json::from_str(spelling)
                    .map_err(|error| FnxError::Parse(error.to_string()))?;
                if key == "component" && !value.is_string() {
                    continue;
                }
                let element = FnxElement::new(&layout.tag);
                let retained = self
                    .reference_values
                    .get(id)
                    .and_then(|values| values.get(key))
                    .cloned()
                    .map(Ok)
                    .unwrap_or_else(|| {
                        resolve_reference_value(&element, key, &value, &self.original_refs)
                    })?;
                match resolve_reference_value(&element, key, &value, &self.refs) {
                    Ok(current) if current == retained => {}
                    Ok(_) | Err(_) => return Ok(false),
                }
            }
        }
        Ok(true)
    }

    /// Check explicit identity and topology in retained tag slots. The caller
    /// must already have validated attribute semantics; this avoids decoding
    /// unchanged JSON again after a checked canvas patch.
    pub fn retained_identity_matches(&self, sidecar: &FnxSidecar) -> Result<bool, FnxError> {
        if self.nodes.len() != sidecar.ids.len() {
            return Ok(false);
        }
        let mut parents = Vec::<(usize, usize)>::new();
        let mut previous_open = None;
        for (position, entry) in sidecar.ids.iter().enumerate() {
            let Some(node) = self.nodes.get(&entry.id) else {
                return Ok(false);
            };
            if previous_open.is_some_and(|previous| node.open <= previous) {
                return Ok(false);
            }
            previous_open = Some(node.open);
            while parents.last().is_some_and(|(_, close)| node.open >= *close) {
                parents.pop();
            }
            let parent = parents
                .last()
                .map(|(parent, _)| u32::try_from(*parent))
                .transpose()
                .map_err(|error| FnxError::Parse(error.to_string()))?;
            let opening = self
                .pieces
                .get(node.open)
                .ok_or_else(|| FnxError::Parse("retained opening is absent".into()))?;
            let layout = OpenTagLayout::parse(opening)?;
            let string_attribute = |key: &str| -> Result<Option<String>, FnxError> {
                let Some(attribute) = layout.attrs.get(key) else {
                    return Ok(None);
                };
                let value = opening
                    .get(attribute.value.clone())
                    .ok_or_else(|| FnxError::Parse("retained attribute span is absent".into()))?;
                let value = value
                    .strip_prefix('{')
                    .and_then(|value| value.strip_suffix('}'))
                    .unwrap_or(value);
                let value: serde_json::Value = serde_json::from_str(value)
                    .map_err(|error| FnxError::Parse(error.to_string()))?;
                Ok(value.as_str().map(str::to_owned))
            };
            let tag = if SUGAR_TAGS.contains(&layout.tag.as_str()) {
                "Vector"
            } else {
                layout.tag.as_str()
            };
            if string_attribute("id")?.as_deref() != Some(entry.id.as_str())
                || string_attribute("name")? != entry.name
                || entry.tag.as_deref() != Some(tag)
                || entry.parent_index != parent
            {
                return Ok(false);
            }
            if let Some(close) = node.close {
                parents.push((position, close));
            }
        }
        Ok(true)
    }

    /// The element with reference ids re-sugared into names, when the retained
    /// table opted into `emit_names` — mirroring what the canonical printer
    /// would emit, so a patched tag never regresses a `$path`/name spelling
    /// back to a raw ULID.
    fn ref_shaped(&self, mut shaped: FnxElement) -> FnxElement {
        if self.refs.emit_names() {
            crate::refs::sugar_element(&mut shaped, &self.refs);
        }
        shaped
    }

    /// Materialize the current source bytes.
    pub fn render(&self) -> String {
        let capacity = self.pieces.iter().map(|piece| piece.len()).sum();
        let mut source = String::with_capacity(capacity);
        for piece in &self.pieces {
            source.push_str(piece);
        }
        source
    }

    /// Migrate legacy or newly hand-added elements to explicit source IDs by
    /// inserting only missing attributes in their existing opening tags.
    pub fn insert_missing_ids(&mut self, sidecar: &FnxSidecar) -> Result<bool, FnxError> {
        let mut changed = false;
        for entry in &sidecar.ids {
            let node = self.nodes.get(&entry.id).ok_or_else(|| {
                FnxError::Parse(format!("source mirror has no node id {}", entry.id))
            })?;
            let mut opening = self.pieces[node.open].to_string();
            let layout = OpenTagLayout::parse(&opening)?;
            if layout.attrs.contains_key("id") {
                continue;
            }
            let attribute = format!(
                " id={}",
                render_attr(&serde_json::Value::String(entry.id.clone()))
            );
            opening.insert_str(layout.insert_at, &attribute);
            self.pieces[node.open] = Arc::from(opening);
            self.mutation_token = Arc::new(());
            changed = true;
        }
        Ok(changed)
    }

    /// Rebuild a changed tree while retaining byte-identical source for every
    /// unchanged subtree. This is used for canvas inserts, deletes, and
    /// reorders, where individual opening-tag patches cannot express the
    /// change but canonicalizing the whole file would erase author trivia.
    pub fn rebuild_tree(&self, previous: &ArtifactIr, next: &ArtifactIr) -> Result<Self, FnxError> {
        let mut previous_elements = Vec::new();
        flatten_elements(previous.root(), None, &mut previous_elements);
        if previous_elements.len() != previous.sidecar().ids.len() {
            return Err(FnxError::SidecarMismatch {
                sidecar: previous.sidecar().ids.len(),
                elements: previous_elements.len(),
            });
        }

        let mut patched = self.clone();
        let by_id: BTreeMap<&str, usize> = previous
            .sidecar()
            .ids
            .iter()
            .enumerate()
            .map(|(position, entry)| (entry.id.as_str(), position))
            .collect();
        let mut next_elements = Vec::new();
        flatten_elements(next.root(), None, &mut next_elements);
        for (entry, (next_element, _)) in next.sidecar().ids.iter().zip(&next_elements) {
            let Some(position) = by_id.get(entry.id.as_str()).copied() else {
                continue;
            };
            let previous_element = previous_elements[position].0;
            if previous_element.tag == next_element.tag
                && previous_element.children.is_empty() == next_element.children.is_empty()
            {
                // A changed tag is handled by the builder below. A same-tag
                // delta edits only that opening-tag slot, preserving the
                // author's attribute order and spellings before reconstruction.
                patched.patch_element_delta(&entry.id, previous_element, next_element)?;
            }
        }

        let source = patched.render();
        let spans = SourceScanner::new(&source).scan()?;
        if spans.len() != previous_elements.len() {
            return Err(FnxError::SidecarMismatch {
                sidecar: previous_elements.len(),
                elements: spans.len(),
            });
        }
        let root_span = spans
            .first()
            .ok_or_else(|| FnxError::Parse("source has no root element".into()))?;
        let root_end = span_end(root_span);
        let mut builder = StructuralSourceBuilder::new(
            &source,
            &spans,
            &previous_elements,
            &by_id,
            next,
            &self.refs,
        );
        let mut rebuilt = String::with_capacity(source.len());
        rebuilt.push_str(&source[..root_span.open.start]);
        builder.append(next.root(), 2, &mut rebuilt)?;
        rebuilt.push_str(&source[root_end..]);
        if crate::parse::parse_doc_with(&rebuilt, &self.refs)? != *next.root() {
            return Err(FnxError::Parse(
                "structural source rebuild changed the artifact semantics".into(),
            ));
        }
        let rebuilt = Self::from_source_with(&rebuilt, next.sidecar(), Arc::clone(&self.refs))?;
        if self.child_comments()? != rebuilt.child_comments()? {
            return Err(FnxError::Parse(
                "cannot preserve authored FNX comments exactly in this canvas edit; edit the FNX source instead".into(),
            ));
        }
        Ok(rebuilt)
    }

    /// The caller must establish that `previous` is this mirror's synchronized
    /// semantic tree. The identity/generation checks detect a stale patch;
    /// reparsing unchanged attributes here would defeat source-scoped adoption.
    pub fn prepare_structural_patch(
        &self,
        previous: &ArtifactIr,
        patch: &PreparedStructuralPatch,
    ) -> Result<Self, FnxError> {
        if !patch.matches(previous)
            || self.nodes.len() != previous.sidecar().ids.len()
            || previous
                .sidecar()
                .ids
                .iter()
                .any(|entry| !self.nodes.contains_key(&entry.id))
        {
            return Err(FnxError::Parse(
                "structural source baseline is stale".into(),
            ));
        }
        let mut patched = self.clone();
        for (id, element) in &patch.replacements {
            if let Some(before) = previous.element(id) {
                patched.patch_element_delta_with_children(
                    id,
                    before,
                    element,
                    !before.children.is_empty(),
                )?;
            }
        }
        let mut rebuilt = Self {
            pieces: vec![Arc::clone(
                self.pieces
                    .first()
                    .ok_or_else(|| FnxError::Parse("source wrapper is absent".into()))?,
            )],
            nodes: BTreeMap::new(),
            refs: Arc::clone(&self.refs),
            original_refs: Arc::clone(&self.original_refs),
            reference_values: BTreeMap::new(),
            mutation_token: Arc::new(()),
        };
        let root = patch
            .order
            .first()
            .ok_or_else(|| FnxError::Parse("structural source has no root".into()))?;
        let mut pending = vec![(root.as_str(), 2usize, false)];
        while let Some((id, depth, closing)) = pending.pop() {
            let element = patch
                .element(previous, id)
                .ok_or_else(|| FnxError::Parse(format!("structural source lost node {id}")))?;
            let children = patch.children.get(id).ok_or_else(|| {
                FnxError::Parse(format!("structural source lost children of {id}"))
            })?;
            let retained = patched.nodes.get(id);
            if closing {
                let (tail, close) = if let Some(close) = retained.and_then(|node| node.close) {
                    (
                        Arc::clone(patched.pieces.get(close.saturating_sub(1)).ok_or_else(
                            || FnxError::Parse("retained closing trivia is absent".into()),
                        )?),
                        Arc::clone(patched.pieces.get(close).ok_or_else(|| {
                            FnxError::Parse("retained closing tag is absent".into())
                        })?),
                    )
                } else {
                    (
                        Arc::from(format!("\n{}", "  ".repeat(depth))),
                        Arc::from(render_close_tag(element)),
                    )
                };
                rebuilt.pieces.push(tail);
                let position = rebuilt.pieces.len();
                rebuilt.pieces.push(close);
                rebuilt
                    .nodes
                    .get_mut(id)
                    .ok_or_else(|| FnxError::Parse("new closing tag lost its opening".into()))?
                    .close = Some(position);
                continue;
            }
            if id != root {
                let leading = if let Some(node) = retained {
                    Arc::clone(patched.pieces.get(node.open.saturating_sub(1)).ok_or_else(
                        || FnxError::Parse("retained leading trivia is absent".into()),
                    )?)
                } else {
                    Arc::from(format!("\n{}", "  ".repeat(depth)))
                };
                rebuilt.pieces.push(leading);
            }
            let has_children = !children.is_empty();
            let opening = if let Some(node) = retained {
                let opening = patched
                    .pieces
                    .get(node.open)
                    .ok_or_else(|| FnxError::Parse("retained opening tag is absent".into()))?;
                if node.close.is_none()
                    && has_children
                    && SUGAR_TAGS.contains(&OpenTagLayout::parse(opening)?.tag.as_str())
                {
                    let shaped = self.ref_shaped(opening_tag_element(element));
                    Arc::from(render_open_tag_with_children(&shaped, true))
                } else if node.close.is_some() == has_children {
                    Arc::clone(opening)
                } else {
                    let mut opening = opening.to_string();
                    if has_children {
                        let suffix = opening
                            .strip_suffix("/>")
                            .ok_or_else(|| {
                                FnxError::Parse("retained leaf tag is malformed".into())
                            })?
                            .len();
                        opening.replace_range(suffix.., ">");
                    } else {
                        let suffix = opening
                            .strip_suffix('>')
                            .ok_or_else(|| {
                                FnxError::Parse("retained parent tag is malformed".into())
                            })?
                            .len();
                        opening.replace_range(suffix.., "/>");
                    }
                    Arc::from(opening)
                }
            } else {
                let mut shaped = opening_tag_element(element);
                if !has_children {
                    crate::sugar::sugar_shapes(&mut shaped);
                }
                crate::sugar::sugar_transform(&mut shaped);
                crate::refs::sugar_refs(&mut shaped, &self.refs);
                Arc::from(render_open_tag_with_children(&shaped, has_children))
            };
            if retained.is_none_or(|node| !Arc::ptr_eq(&opening, &patched.pieces[node.open])) {
                let references = patched.validate_opening_semantics(
                    id,
                    &opening,
                    element,
                    has_children,
                    retained.map(|_| element),
                )?;
                if !references.is_empty() {
                    rebuilt.reference_values.insert(id.to_owned(), references);
                }
            } else if let Some(references) = patched.reference_values.get(id) {
                rebuilt
                    .reference_values
                    .insert(id.to_owned(), references.clone());
            }
            let position = rebuilt.pieces.len();
            rebuilt.pieces.push(opening);
            rebuilt.nodes.insert(
                id.to_owned(),
                NodePieces {
                    open: position,
                    close: None,
                },
            );
            if has_children {
                pending.push((id, depth, true));
                pending.extend(
                    children
                        .iter()
                        .rev()
                        .map(|child| (child.as_str(), depth + 1, false)),
                );
            }
        }
        rebuilt.pieces.push(Arc::clone(
            self.pieces
                .last()
                .ok_or_else(|| FnxError::Parse("source suffix is absent".into()))?,
        ));
        if self.child_comments()? != rebuilt.child_comments()? {
            return Err(FnxError::Parse("cannot preserve authored FNX comments exactly in this canvas edit; edit the FNX source instead".into()));
        }
        Ok(rebuilt)
    }

    fn validate_opening_semantics(
        &self,
        id: &str,
        opening: &str,
        expected: &FnxElement,
        has_children: bool,
        previous: Option<&FnxElement>,
    ) -> Result<BTreeMap<&'static str, serde_json::Value>, FnxError> {
        let source = if has_children {
            let layout = OpenTagLayout::parse(opening)?;
            format!("{opening}</{}>", layout.tag)
        } else {
            opening.to_owned()
        };
        let mut parsed = crate::parse::parse_doc(&source)?;
        let layout = OpenTagLayout::parse(opening)?;
        let retained = self
            .nodes
            .get(id)
            .and_then(|node| self.pieces.get(node.open));
        let retained_layout = retained
            .map(|opening| OpenTagLayout::parse(opening))
            .transpose()?;
        let mut references = BTreeMap::new();
        for key in ["component", "bindings"] {
            let Some(value) = parsed.attrs.get(key) else {
                continue;
            };
            if key == "component" && !value.is_string() {
                continue;
            }
            let unchanged = previous
                .is_some_and(|previous| previous.attrs.get(key) == expected.attrs.get(key))
                && retained.zip(retained_layout.as_ref()).is_some_and(
                    |(retained, retained_layout)| {
                        retained_layout
                            .attrs
                            .get(key)
                            .zip(layout.attrs.get(key))
                            .is_some_and(|(before, after)| {
                                retained.get(before.value.clone())
                                    == opening.get(after.value.clone())
                            })
                    },
                );
            let value = if unchanged {
                self.reference_values
                    .get(id)
                    .and_then(|references| references.get(key))
                    .cloned()
                    .map(Ok)
                    .unwrap_or_else(|| {
                        resolve_reference_value(&parsed, key, value, &self.original_refs)
                    })?
            } else {
                resolve_reference_value(&parsed, key, value, &self.refs)?
            };
            parsed.attrs.insert(key.into(), value.clone());
            references.insert(key, value);
        }
        if expected.attrs.contains_key("id") {
            parsed
                .attrs
                .entry("id".to_owned())
                .or_insert_with(|| id.into());
        }
        if parsed.tag != expected.tag || parsed.attrs != expected.attrs {
            return Err(FnxError::Parse(format!(
                "rewritten opening tag changed the semantics of node {id}"
            )));
        }
        Ok(references)
    }

    fn resolve_retained_references(
        &self,
        id: &str,
        element: &mut FnxElement,
    ) -> Result<(), FnxError> {
        for key in ["component", "bindings"] {
            let Some(value) = element.attrs.get(key) else {
                continue;
            };
            if key == "component" && !value.is_string() {
                continue;
            }
            let value = self
                .reference_values
                .get(id)
                .and_then(|references| references.get(key))
                .cloned()
                .map(Ok)
                .unwrap_or_else(|| {
                    resolve_reference_value(element, key, value, &self.original_refs)
                })?;
            element.attrs.insert(key.into(), value);
        }
        Ok(())
    }

    fn child_comments(&self) -> Result<BTreeMap<String, usize>, FnxError> {
        let tag_pieces: std::collections::BTreeSet<_> = self
            .nodes
            .values()
            .flat_map(|node| std::iter::once(node.open).chain(node.close))
            .collect();
        let mut comments = BTreeMap::new();
        for (position, piece) in self.pieces.iter().enumerate() {
            // Wrapper code and tag values may contain comment-like strings;
            // only scanner-validated JSX child trivia belongs to this check.
            if position == 0 || position + 1 == self.pieces.len() || tag_pieces.contains(&position)
            {
                continue;
            }
            let mut trivia = piece.trim_start();
            while !trivia.is_empty() {
                let end = trivia
                    .strip_prefix("{/*")
                    .and_then(|comment| comment.find("*/}"))
                    .map(|end| end + 6)
                    .ok_or_else(|| FnxError::Parse("invalid retained FNX child trivia".into()))?;
                *comments.entry(trivia[..end].to_owned()).or_insert(0) += 1;
                trivia = trivia[end..].trim_start();
            }
        }
        Ok(comments)
    }

    /// Replace one node's semantic tag/attributes without touching any other
    /// source span.
    ///
    /// Structural changes (switching between self-closing and parent form)
    /// require a subtree patch and are intentionally rejected here.
    pub fn patch_element(&mut self, id: &str, element: &FnxElement) -> Result<bool, FnxError> {
        self.patch_element_with_children(id, element, !element.children.is_empty())
    }

    fn patch_element_with_children(
        &mut self,
        id: &str,
        element: &FnxElement,
        has_children: bool,
    ) -> Result<bool, FnxError> {
        let node = self
            .nodes
            .get(id)
            .ok_or_else(|| FnxError::Parse(format!("source mirror has no node id {id}")))?
            .clone();
        if has_children != node.close.is_some() {
            return Err(FnxError::Parse(format!(
                "node {id} changed source structure; a tag-only patch is unsafe"
            )));
        }

        // A full-tag reprint uses canonical attribute spelling, but reference
        // spellings still re-sugar (names / `$paths`) so a fallback reprint of
        // an instance keeps the readable form the printer would emit.
        let expected = element;
        let element = self.ref_shaped(opening_tag_element(element));
        let open = render_open_tag_with_children(&element, has_children);
        let references =
            self.validate_opening_semantics(id, &open, expected, has_children, None)?;
        let mut changed = self.pieces[node.open].as_ref() != open;
        self.pieces[node.open] = Arc::from(open);
        if let Some(close_piece) = node.close {
            let close = render_close_tag(&element);
            changed |= self.pieces[close_piece].as_ref() != close;
            self.pieces[close_piece] = Arc::from(close);
        }
        if changed
            || self
                .reference_values
                .get(id)
                .map_or(!references.is_empty(), |previous| previous != &references)
        {
            self.mutation_token = Arc::new(());
        }
        if references.is_empty() {
            self.reference_values.remove(id);
        } else {
            self.reference_values.insert(id.to_owned(), references);
        }
        Ok(changed)
    }

    /// Apply a semantic before/after delta to one node while retaining the
    /// node's own untouched attribute spelling and ordering as well as all
    /// surrounding source.
    ///
    /// Existing values are replaced in place, removed attributes consume only
    /// their own leading whitespace, and newly introduced attributes are
    /// inserted immediately before the tag terminator. A node-type change
    /// falls back to [`patch_element`](Self::patch_element), since changing the
    /// tag can also change which attribute sugar is valid.
    pub fn patch_element_delta(
        &mut self,
        id: &str,
        previous: &FnxElement,
        next: &FnxElement,
    ) -> Result<bool, FnxError> {
        self.patch_element_delta_with_children(id, previous, next, !next.children.is_empty())
    }

    fn patch_element_delta_with_children(
        &mut self,
        id: &str,
        previous: &FnxElement,
        next: &FnxElement,
        has_children: bool,
    ) -> Result<bool, FnxError> {
        let canonical_previous = previous;
        let canonical_next = next;
        if previous.tag != next.tag {
            return self.patch_element_with_children(id, next, has_children);
        }
        let node = self
            .nodes
            .get(id)
            .ok_or_else(|| FnxError::Parse(format!("source mirror has no node id {id}")))?
            .clone();
        if previous.children.is_empty() == has_children || has_children != node.close.is_some() {
            return Err(FnxError::Parse(format!(
                "node {id} changed source structure; an attribute patch is unsafe"
            )));
        }
        if previous.attrs == next.attrs {
            return Ok(false);
        }

        let layout = OpenTagLayout::parse(&self.pieces[node.open])?;
        // The author spelled this node with shape sugar (`<Rect>`/`<Ellipse>`).
        // Patch in sugar space while the canonical element still round-trips
        // through the sugar (a resize just updates `width`/`height` in place,
        // keeping the author's spelling); the moment the shape stops matching
        // its generator (edited into a free path), reprint this one tag
        // canonically as `<Vector …/>` — mirroring the tag-change fallback
        // above. Vectors are childless, so there is no open/close pair to
        // keep in sync on that fallback.
        let (previous, next) = if SUGAR_TAGS.contains(&layout.tag.as_str()) {
            match (
                shape_sugar_spelling(previous, &layout.tag),
                shape_sugar_spelling(next, &layout.tag),
            ) {
                (Some(previous), Some(next)) => (previous, next),
                _ => return self.patch_element_with_children(id, next, has_children),
            }
        } else {
            (opening_tag_element(previous), opening_tag_element(next))
        };
        // Re-sugar reference spellings on BOTH sides before diffing: an
        // unchanged reference then compares equal in whichever spelling the
        // author used (no spurious edit), and a genuinely changed one is
        // rewritten in the printer's readable form — e.g. binding a new
        // variable prints its `$Collection/Name` path, not a raw ULID.
        let previous = self.ref_shaped(previous);
        let next = self.ref_shaped(next);
        let opening = &self.pieces[node.open];
        let previous = source_shaped(previous, &layout);
        let next = source_shaped(next, &layout);
        let mut edits = Vec::<TextEdit>::new();
        let mut inserted = String::new();

        for key in previous
            .attrs
            .keys()
            .chain(next.attrs.keys())
            .collect::<std::collections::BTreeSet<_>>()
        {
            let old = previous.attrs.get(key);
            let new = next.attrs.get(key);
            if old == new {
                continue;
            }
            match (layout.attrs.get(key.as_str()), new) {
                (Some(span), Some(value)) => edits.push(TextEdit {
                    range: span.value.clone(),
                    replacement: render_attr(value),
                }),
                (Some(span), None) => edits.push(TextEdit {
                    range: span.whole.clone(),
                    replacement: String::new(),
                }),
                (None, Some(value)) => {
                    inserted.push(' ');
                    inserted.push_str(key);
                    inserted.push('=');
                    inserted.push_str(&render_attr(value));
                }
                (None, None) => {}
            }
        }
        if !inserted.is_empty() {
            edits.push(TextEdit {
                range: layout.insert_at..layout.insert_at,
                replacement: inserted,
            });
        }
        if edits.is_empty() {
            return Ok(false);
        }
        edits.sort_unstable_by_key(|edit| std::cmp::Reverse(edit.range.start));
        let mut patched = opening.to_string();
        for edit in edits {
            patched.replace_range(edit.range, &edit.replacement);
        }
        let changed = patched != opening.as_ref();
        let references = self.validate_opening_semantics(
            id,
            &patched,
            canonical_next,
            has_children,
            Some(canonical_previous),
        )?;
        self.pieces[node.open] = Arc::from(patched);
        if changed
            || self
                .reference_values
                .get(id)
                .map_or(!references.is_empty(), |previous| previous != &references)
        {
            self.mutation_token = Arc::new(());
        }
        if references.is_empty() {
            self.reference_values.remove(id);
        } else {
            self.reference_values.insert(id.to_owned(), references);
        }
        Ok(changed)
    }
}

#[derive(Debug)]
struct TextEdit {
    range: Range<usize>,
    replacement: String,
}

fn resolve_reference_value(
    element: &FnxElement,
    key: &str,
    value: &serde_json::Value,
    refs: &RefTable,
) -> Result<serde_json::Value, FnxError> {
    let mut reference = FnxElement::new(&element.tag);
    if let Some(name) = element.attrs.get("name") {
        reference.attrs.insert("name".into(), name.clone());
    }
    reference.attrs.insert(key.into(), value.clone());
    crate::refs::desugar_refs(&mut reference, refs)?;
    reference
        .attrs
        .remove(key)
        .ok_or_else(|| FnxError::Parse(format!("reference resolution lost attribute {key}")))
}

#[derive(Debug)]
struct AttrSpan {
    whole: Range<usize>,
    value: Range<usize>,
}

#[derive(Debug)]
struct OpenTagLayout {
    /// The tag ident as SPELLED in source — a sugar tag (`Rect`/`Ellipse`)
    /// when the author used shape sugar, unlike the canonical semantic
    /// element's tag (always `Vector` post-parse). The delta patcher keys on
    /// this to patch in sugar space and keep the author's spelling.
    tag: String,
    attrs: BTreeMap<String, AttrSpan>,
    insert_at: usize,
}

impl OpenTagLayout {
    fn parse(source: &str) -> Result<Self, FnxError> {
        let mut scanner = SourceScanner::new(source);
        scanner.expect(b'<')?;
        let tag = scanner.ident()?;
        let mut attrs = BTreeMap::new();
        loop {
            let whitespace = scanner.cursor;
            scanner.skip_ws();
            match (scanner.peek(), scanner.peek_at(1)) {
                (Some(b'/'), Some(b'>')) | (Some(b'>'), _) => {
                    return Ok(Self {
                        tag,
                        attrs,
                        insert_at: whitespace,
                    });
                }
                (Some(_), _) => {
                    let key = scanner.ident()?;
                    scanner.skip_ws();
                    scanner.expect(b'=')?;
                    scanner.skip_ws();
                    let value_start = scanner.cursor;
                    scanner.attr_value()?;
                    attrs.insert(
                        key,
                        AttrSpan {
                            whole: whitespace..scanner.cursor,
                            value: value_start..scanner.cursor,
                        },
                    );
                }
                (None, _) => return Err(scanner.error("unterminated opening tag")),
            }
        }
    }
}

fn opening_tag_element(element: &FnxElement) -> FnxElement {
    // Each child owns a separate source span. Tag shaping must not repeatedly
    // clone or traverse their subtrees while rebuilding a parent tag.
    FnxElement {
        tag: element.tag.clone(),
        attrs: element.attrs.clone(),
        children: Vec::new(),
    }
}

/// Re-apply the author's attribute sugar spelling (x/y position, width/height
/// size) to a canonical element so the delta patcher compares like with like.
/// Shape sugar is handled by the caller BEFORE this runs (it changes the tag,
/// which decides the fallback), so `element` here is either canonical or
/// already sugar-spelled.
fn source_shaped(element: FnxElement, layout: &OpenTagLayout) -> FnxElement {
    let mut shaped = element;
    if layout.attrs.contains_key("x") || layout.attrs.contains_key("y") {
        crate::sugar::sugar_transform(&mut shaped);
    }
    let size_key = match shaped.tag.as_str() {
        "Frame" => Some("clip_size"),
        "Vector" => None,
        _ => Some("local_size"),
    };
    if (layout.attrs.contains_key("width") || layout.attrs.contains_key("height"))
        && let Some(size_key) = size_key
        && let Some(size) = shaped
            .attrs
            .get(size_key)
            .and_then(serde_json::Value::as_array)
        && size.len() == 2
    {
        let width = size[0].clone();
        let height = size[1].clone();
        shaped.attrs.remove(size_key);
        shaped.attrs.insert("width".into(), width);
        shaped.attrs.insert("height".into(), height);
    }
    shaped
}

#[derive(Debug)]
struct ElementSpans {
    open: Range<usize>,
    close: Option<Range<usize>>,
}

fn span_end(span: &ElementSpans) -> usize {
    span.close.as_ref().map_or(span.open.end, |close| close.end)
}

fn flatten_elements<'a>(
    element: &'a FnxElement,
    parent: Option<usize>,
    elements: &mut Vec<(&'a FnxElement, Option<usize>)>,
) {
    let position = elements.len();
    elements.push((element, parent));
    for child in &element.children {
        flatten_elements(child, Some(position), elements);
    }
}

fn element_count(element: &FnxElement) -> usize {
    1 + element.children.iter().map(element_count).sum::<usize>()
}

struct StructuralSourceBuilder<'a> {
    source: &'a str,
    spans: &'a [ElementSpans],
    previous_elements: &'a [(&'a FnxElement, Option<usize>)],
    by_id: BTreeMap<String, usize>,
    children: Vec<Vec<usize>>,
    leading: Vec<Option<Range<usize>>>,
    next: &'a ArtifactIr,
    refs: &'a RefTable,
    cursor: usize,
}

impl<'a> StructuralSourceBuilder<'a> {
    fn new(
        source: &'a str,
        spans: &'a [ElementSpans],
        previous_elements: &'a [(&'a FnxElement, Option<usize>)],
        by_id: &BTreeMap<&str, usize>,
        next: &'a ArtifactIr,
        refs: &'a RefTable,
    ) -> Self {
        let mut children = vec![Vec::new(); previous_elements.len()];
        for (position, (_, parent)) in previous_elements.iter().enumerate() {
            if let Some(parent) = parent {
                children[*parent].push(position);
            }
        }
        let mut leading = vec![None; previous_elements.len()];
        for (parent, siblings) in children.iter().enumerate() {
            let mut start = spans[parent].open.end;
            for &child in siblings {
                leading[child] = Some(start..spans[child].open.start);
                start = span_end(&spans[child]);
            }
        }
        Self {
            source,
            spans,
            previous_elements,
            by_id: by_id
                .iter()
                .map(|(id, position)| ((*id).to_owned(), *position))
                .collect(),
            children,
            leading,
            next,
            refs,
            cursor: 0,
        }
    }

    fn append(
        &mut self,
        element: &FnxElement,
        depth: usize,
        out: &mut String,
    ) -> Result<(), FnxError> {
        let entry = self
            .next
            .sidecar()
            .ids
            .get(self.cursor)
            .ok_or_else(|| FnxError::Parse("new tree exceeds sidecar".into()))?;
        self.cursor += 1;
        let old_position = self.by_id.get(&entry.id).copied();
        if let Some(position) = old_position {
            let old_element = self.previous_elements[position].0;
            if old_element == element {
                let span = &self.spans[position];
                out.push_str(&self.source[span.open.start..span_end(span)]);
                self.cursor += element_count(element) - 1;
                return Ok(());
            }
        }

        let reusable = old_position.filter(|position| {
            let old_element = self.previous_elements[*position].0;
            old_element.tag == element.tag
                && old_element.children.is_empty() == element.children.is_empty()
        });
        if let Some(position) = reusable {
            let span = &self.spans[position];
            out.push_str(&self.source[span.open.clone()]);
        } else {
            let mut shaped = opening_tag_element(element);
            // Only childless vectors qualify for shape sugar. Shallow tag
            // shaping must preserve the original node's eligibility.
            if element.children.is_empty() {
                crate::sugar::sugar_shapes(&mut shaped);
            }
            crate::sugar::sugar_transform(&mut shaped);
            crate::refs::sugar_refs(&mut shaped, self.refs);
            out.push_str(&render_open_tag_with_children(
                &shaped,
                !element.children.is_empty(),
            ));
        }
        if element.children.is_empty() {
            return Ok(());
        }

        for child in &element.children {
            let child_id = self
                .next
                .sidecar()
                .ids
                .get(self.cursor)
                .ok_or_else(|| FnxError::Parse("new child exceeds sidecar".into()))?
                .id
                .as_str();
            let child_old_position = self.by_id.get(child_id).copied();
            // Leading child trivia follows its stable identity on reparent;
            // the old parent no longer emits that child's interval.
            let old_leading = child_old_position.and_then(|child| self.leading[child].as_ref());
            if let Some(leading) = old_leading {
                out.push_str(&self.source[leading.clone()]);
            } else {
                out.push('\n');
                out.push_str(&"  ".repeat(depth + 1));
            }
            self.append(child, depth + 1, out)?;
        }

        if let Some(position) = old_position
            && let Some(close) = &self.spans[position].close
        {
            let tail_start = self.children[position]
                .last()
                .map_or(self.spans[position].open.end, |last| {
                    span_end(&self.spans[*last])
                });
            out.push_str(&self.source[tail_start..close.start]);
            if self.previous_elements[position].0.tag == element.tag {
                out.push_str(&self.source[close.clone()]);
            } else {
                out.push_str(&render_close_tag(element));
            }
        } else {
            out.push('\n');
            out.push_str(&"  ".repeat(depth));
            out.push_str(&render_close_tag(element));
        }
        Ok(())
    }
}

#[derive(Debug)]
struct SourceToken {
    range: Range<usize>,
    id: String,
    kind: TokenKind,
}

#[derive(Debug, Clone, Copy)]
enum TokenKind {
    Open,
    Close,
}

struct SourceScanner<'a> {
    source: &'a str,
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> SourceScanner<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            bytes: source.as_bytes(),
            cursor: 0,
        }
    }

    fn scan(mut self) -> Result<Vec<ElementSpans>, FnxError> {
        self.cursor = self
            .bytes
            .iter()
            .position(|byte| *byte == b'<')
            .ok_or_else(|| FnxError::Parse("no element found".into()))?;
        let mut spans = Vec::new();
        self.element(&mut spans)?;
        Ok(spans)
    }

    fn element(&mut self, spans: &mut Vec<ElementSpans>) -> Result<(), FnxError> {
        self.skip_ws();
        let open_start = self.cursor;
        self.expect(b'<')?;
        if self.peek() == Some(b'/') {
            return Err(self.error("unexpected closing tag"));
        }
        let tag = self.ident()?;
        let span_index = spans.len();
        spans.push(ElementSpans {
            open: open_start..open_start,
            close: None,
        });
        let mut attrs = std::collections::BTreeSet::new();

        loop {
            self.skip_ws();
            match (self.peek(), self.peek_at(1)) {
                (Some(b'/'), Some(b'>')) => {
                    self.cursor += 2;
                    spans[span_index].open = open_start..self.cursor;
                    return Ok(());
                }
                (Some(b'>'), _) => {
                    self.cursor += 1;
                    spans[span_index].open = open_start..self.cursor;
                    break;
                }
                (Some(_), _) => {
                    let key = self.ident()?;
                    if !attrs.insert(key.clone()) {
                        return Err(self.error(format!("duplicate attribute {key}")));
                    }
                    self.skip_ws();
                    self.expect(b'=')?;
                    self.skip_ws();
                    self.attr_value()?;
                }
                (None, _) => return Err(self.error("unterminated opening tag")),
            }
        }

        loop {
            self.skip_child_trivia()?;
            if self.peek() == Some(b'<') && self.peek_at(1) == Some(b'/') {
                let close_start = self.cursor;
                self.cursor += 2;
                let close = self.ident()?;
                if close != tag {
                    return Err(self.error(format!("</{close}> closing <{tag}>")));
                }
                self.skip_ws();
                self.expect(b'>')?;
                spans[span_index].close = Some(close_start..self.cursor);
                return Ok(());
            }
            if self.peek().is_none() {
                return Err(self.error(format!("unterminated <{tag}>")));
            }
            self.element(spans)?;
        }
    }

    fn skip_child_trivia(&mut self) -> Result<(), FnxError> {
        loop {
            self.skip_ws();
            if self.bytes.get(self.cursor..self.cursor + 3) != Some(b"{/*") {
                return Ok(());
            }
            self.cursor += 3;
            while self.cursor + 2 < self.bytes.len()
                && self.bytes.get(self.cursor..self.cursor + 3) != Some(b"*/}")
            {
                self.cursor += 1;
            }
            if self.cursor + 2 >= self.bytes.len() {
                return Err(self.error("unterminated JSX comment"));
            }
            self.cursor += 3;
        }
    }

    fn attr_value(&mut self) -> Result<(), FnxError> {
        match self.peek() {
            Some(b'"') => self.string(),
            Some(b'{') => self.braced(),
            _ => Err(self.error("expected attribute value")),
        }
    }

    fn string(&mut self) -> Result<(), FnxError> {
        self.expect(b'"')?;
        loop {
            match self.bump() {
                Some(b'\\') => {
                    if self.bump().is_none() {
                        return Err(self.error("unterminated string escape"));
                    }
                }
                Some(b'"') => return Ok(()),
                Some(_) => {}
                None => return Err(self.error("unterminated string")),
            }
        }
    }

    fn braced(&mut self) -> Result<(), FnxError> {
        self.expect(b'{')?;
        let mut depth = 1usize;
        let mut in_string = false;
        while let Some(byte) = self.bump() {
            if in_string {
                match byte {
                    b'\\' => {
                        if self.bump().is_none() {
                            return Err(self.error("unterminated string escape"));
                        }
                    }
                    b'"' => in_string = false,
                    _ => {}
                }
                continue;
            }
            match byte {
                b'"' => in_string = true,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(());
                    }
                }
                _ => {}
            }
        }
        Err(self.error("unterminated braced attribute"))
    }

    fn ident(&mut self) -> Result<String, FnxError> {
        let start = self.cursor;
        while self
            .peek()
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            self.cursor += 1;
        }
        if start == self.cursor {
            return Err(self.error("expected identifier"));
        }
        Ok(self.source[start..self.cursor].to_owned())
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.cursor += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), FnxError> {
        if self.peek() == Some(byte) {
            self.cursor += 1;
            Ok(())
        } else {
            Err(self.error(format!("expected byte {:?}", char::from(byte))))
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.cursor).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.cursor + offset).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.cursor += 1;
        Some(byte)
    }

    fn error(&self, message: impl Into<String>) -> FnxError {
        FnxError::Parse(format!("at byte {}: {}", self.cursor, message.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ArtifactIr, ArtifactKind, IdEntry, parse_doc};
    use serde_json::Value;

    fn sidecar() -> FnxSidecar {
        FnxSidecar {
            root_parent: None,
            ids: vec![
                IdEntry {
                    id: "root".into(),
                    index: Value::from(1),
                    tag: Some("Frame".into()),
                    name: Some("Root".into()),
                    parent_index: None,
                },
                IdEntry {
                    id: "child".into(),
                    index: Value::from(1),
                    tag: Some("Text".into()),
                    name: Some("Label".into()),
                    parent_index: Some(0),
                },
            ],
        }
    }

    #[test]
    fn sidecar_witness_checks_full_json_without_retaining_node_payloads() {
        let source = r##"<Frame id="root" name="Root" x={12.5} background={{"color": fnxColor("#FFFFFF"), "kind": "solid"}}><Text id={"child"} name="Label" content="héllo" future_hint={{"segments": [[1, 2.5, -3e2]], "quoted": "a\"b", "enabled": true, "empty": null}} /></Frame>"##;
        assert!(parse_doc(source).is_ok());
        assert!(sidecar_matches_source(source, &sidecar()).expect("coherent modern source"));
        assert_eq!(
            crate::reconcile_sidecar(source, &sidecar(), || "unexpected".into())
                .expect("existing reconciliation"),
            sidecar()
        );
    }

    #[test]
    fn sidecar_witness_rejects_stale_identity_and_structural_fingerprints() {
        let source = r#"<Frame id="root" name="Root"><Text id="child" name="Label" /></Frame>"#;
        let mut stale = sidecar();
        stale.ids[0].id = "other".into();
        assert!(!sidecar_matches_source(source, &stale).expect("stale root"));
        let mut stale = sidecar();
        stale.ids[1].id = "other".into();
        assert!(!sidecar_matches_source(source, &stale).expect("stale descendant"));
        let mut stale = sidecar();
        stale.ids[1].parent_index = None;
        assert!(!sidecar_matches_source(source, &stale).expect("stale parent"));
        let mut stale = sidecar();
        stale.ids[1].name = Some("Previous".into());
        assert!(!sidecar_matches_source(source, &stale).expect("stale name"));
        let mut stale = sidecar();
        stale.ids[1].tag = None;
        assert!(!sidecar_matches_source(source, &stale).expect("legacy fingerprint"));
        let mut stale = sidecar();
        stale.ids.swap(0, 1);
        assert!(!sidecar_matches_source(source, &stale).expect("stale order"));
        let mut stale = sidecar();
        stale.ids.pop();
        assert!(!sidecar_matches_source(source, &stale).expect("missing entry"));
        assert!(
            !sidecar_matches_source(&source.replace(" id=\"child\"", ""), &sidecar())
                .expect("legacy source uses normal reconciliation")
        );
        let reparented = r#"<Frame id="root" name="Root"><Text id="child" name="Label"><Text id="grandchild" name="Nested" /></Text></Frame>"#;
        let mut stale = sidecar();
        stale.ids.push(IdEntry {
            id: "grandchild".into(),
            index: Value::from(2.0),
            tag: Some("Text".into()),
            name: Some("Nested".into()),
            parent_index: Some(0),
        });
        assert!(!sidecar_matches_source(reparented, &stale).expect("reparented source"));
    }

    #[test]
    fn sidecar_witness_preserves_invalid_attribute_errors() {
        for attribute in [
            "opacity={oops}",
            "local_size={[broken]}",
            r#"future_hint="invalid\q""#,
            "future_hint={1e400}",
            r#"future_hint={{"nested": [1e400]}}"#,
            r##"background={{"color": fnxColor("#INVALID")}}"##,
            "x={1} transform={[1, 0, 0, 1, 0, 0]}",
        ] {
            let source = format!(
                "<Frame id=\"root\" name=\"Root\" {attribute}><Text id=\"child\" name=\"Label\" /></Frame>"
            );
            assert!(parse_doc(&source).is_err(), "{attribute}");
            assert!(
                !matches!(sidecar_matches_source(&source, &sidecar()), Ok(true)),
                "{attribute}"
            );
        }
    }

    #[test]
    fn sidecar_witness_checks_canonical_shape_fingerprints_and_dimensions() {
        let mut canonical = sidecar();
        canonical.ids[1].tag = Some("Vector".into());
        for tag in ["Rect", "Ellipse"] {
            for dimensions in [
                "width={10} height={20}",
                "width={0} height={0}",
                "width={-10} height={-20}",
                "width={1e308} height={1e308}",
            ] {
                let source = format!(
                    "<Frame id=\"root\" name=\"Root\"><{tag} id=\"child\" name=\"Label\" {dimensions} /></Frame>"
                );
                assert!(parse_doc(&source).is_ok(), "{tag} {dimensions}");
                assert!(
                    sidecar_matches_source(&source, &canonical).expect("canonical shape"),
                    "{tag} {dimensions}"
                );
                assert_eq!(
                    crate::reconcile_sidecar(&source, &canonical, || "unexpected".into())
                        .expect("existing reconciliation"),
                    canonical
                );
            }
            for attributes in [
                "width={10}",
                "height={20}",
                r#"width={"10"} height={20}"#,
                r#"width={10} height="20""#,
                "width={10} height={null}",
                "width={1e400} height={20}",
                "width={10} height={20} path={null}",
                "width={10} height={20} x={1} transform={[1, 0, 0, 1, 0, 0]}",
            ] {
                let source = format!(
                    "<Frame id=\"root\" name=\"Root\"><{tag} id=\"child\" name=\"Label\" {attributes} /></Frame>"
                );
                assert!(parse_doc(&source).is_err(), "{tag} {attributes}");
                assert!(
                    !matches!(sidecar_matches_source(&source, &canonical), Ok(true)),
                    "{tag} {attributes}"
                );
            }
        }
    }

    #[test]
    fn synchronization_tokens_cover_source_edits_references_and_identity_migration() {
        let source = r#"<Frame id="root" name="Root"><Text id="child" name="Label" /></Frame>"#;
        let mut mirror = FnxSourceMirror::from_source(source, &sidecar()).expect("source");
        let original = mirror.synchronization_token();
        assert!(Arc::ptr_eq(
            &original,
            &mirror.clone().synchronization_token()
        ));
        let root = parse_doc(source).expect("tree");
        let previous = root.children.first().expect("child");
        let mut next = previous.clone();
        next.attrs.insert("opacity".into(), serde_json::json!(0.5));
        mirror
            .patch_element_delta("child", previous, &next)
            .expect("delta");
        assert!(!Arc::ptr_eq(&original, &mirror.synchronization_token()));
        let changed = mirror.synchronization_token();
        let before = mirror.render();
        let mut stale = next.clone();
        stale
            .attrs
            .insert("name".into(), "Different baseline".into());
        let mut candidate = stale.clone();
        candidate
            .attrs
            .insert("opacity".into(), serde_json::json!(0.25));
        assert!(
            mirror
                .patch_element_delta("child", &stale, &candidate)
                .is_err()
        );
        assert_eq!(mirror.render(), before);
        assert!(Arc::ptr_eq(&changed, &mirror.synchronization_token()));
        next.attrs.insert("name".into(), "Reprinted".into());
        mirror.patch_element("child", &next).expect("full tag");
        assert!(!Arc::ptr_eq(&changed, &mirror.synchronization_token()));
        let reprinted = mirror.synchronization_token();
        mirror.set_ref_table(Arc::new(RefTable::default()));
        assert!(Arc::ptr_eq(&reprinted, &mirror.synchronization_token()));
        let mut legacy = FnxSourceMirror::from_source(
            r#"<Frame name="Root"><Text name="Label" /></Frame>"#,
            &sidecar(),
        )
        .expect("legacy");
        let before_ids = legacy.synchronization_token();
        assert!(legacy.insert_missing_ids(&sidecar()).expect("migrate"));
        assert!(!Arc::ptr_eq(&before_ids, &legacy.synchronization_token()));
        assert!(sidecar_matches_source(&legacy.render(), &sidecar()).expect("identity witness"));
        assert!(
            legacy
                .retained_identity_matches(&sidecar())
                .expect("retained identity")
        );
    }

    #[test]
    fn retained_reference_attributes_survive_renames_and_reused_names() {
        let first = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
        let second = "01ARZ3NDEKTSV4RRFFQ69G5FAW";
        let mut original = RefTable::new(true);
        original.insert_component("Original", first);
        original.insert_variable("Theme/Opacity", first);
        let source = r#"<Frame id="root" name="Root"><Instance id="child" name="Placement" component="Original" bindings={{"opacity":"$Theme/Opacity"}} /></Frame>"#;
        let mut identities = sidecar();
        identities.ids[1].tag = Some("Instance".into());
        identities.ids[1].name = Some("Placement".into());
        let mut ir = ArtifactIr::from_source_with(
            crate::ArtifactKind::Page,
            "References",
            source,
            identities.clone(),
            &original,
        )
        .expect("IR");
        let mut mirror = FnxSourceMirror::from_source_with(source, &identities, Arc::new(original))
            .expect("retained source");
        let token = mirror.synchronization_token();
        let mut renamed = RefTable::new(true);
        renamed.insert_component("Renamed", first);
        renamed.insert_component("Original", second);
        renamed.insert_variable("Theme/Renamed", first);
        renamed.insert_variable("Theme/Opacity", second);
        mirror.set_ref_table(Arc::new(renamed));
        assert!(Arc::ptr_eq(&token, &mirror.synchronization_token()));
        assert!(
            !mirror
                .references_match_current_table()
                .expect("reused names")
        );
        assert!(
            mirror
                .semantics_match(&ir)
                .expect("original reference meaning")
        );
        let previous = ir.element("child").expect("instance").clone();
        let mut next = previous.clone();
        next.attrs.insert("name".into(), "After rename".into());
        mirror
            .patch_element_delta("child", &previous, &next)
            .expect("unrelated edit");
        let mut value = ir.node_value("child").expect("node");
        value["name"] = "After rename".into();
        ir.patch_node_value("child", &value).expect("IR edit");
        assert!(mirror.render().contains("component=\"Original\""));
        assert!(mirror.render().contains("$Theme/Opacity"));
        assert!(mirror.semantics_match(&ir).expect("retained references"));
        let previous = next.clone();
        next.attrs.insert("component".into(), second.into());
        mirror
            .patch_element_delta("child", &previous, &next)
            .expect("component swap");
        value["component"] = second.into();
        ir.patch_node_value("child", &value)
            .expect("IR component swap");
        assert!(
            mirror
                .semantics_match(&ir)
                .expect("mixed reference vocabularies")
        );
        let previous = next.clone();
        next.attrs.insert(
            "bindings".into(),
            serde_json::json!([[{"prop":"opacity"},second]]),
        );
        mirror
            .patch_element_delta("child", &previous, &next)
            .expect("binding swap");
        value["bindings"] = next.attrs["bindings"].clone();
        ir.patch_node_value("child", &value)
            .expect("IR binding swap");
        assert!(
            mirror
                .semantics_match(&ir)
                .expect("updated reference meanings")
        );
        assert!(
            mirror
                .references_match_current_table()
                .expect("current names")
        );
    }

    #[test]
    fn retained_identity_requires_explicit_ids_and_current_fingerprints() {
        let source = r#"<Frame id="root" name="Root"><Text id="child" name="Label" /></Frame>"#;
        let mirror = FnxSourceMirror::from_source(source, &sidecar()).expect("source");
        assert!(
            mirror
                .retained_identity_matches(&sidecar())
                .expect("identity")
        );
        for position in 0..2 {
            let mut stale = sidecar();
            stale.ids[position].name = Some("Renamed".into());
            assert!(
                !mirror
                    .retained_identity_matches(&stale)
                    .expect("stale name")
            );
            stale = sidecar();
            stale.ids[position].tag = Some("Vector".into());
            assert!(!mirror.retained_identity_matches(&stale).expect("stale tag"));
        }
        let mut stale = sidecar();
        stale.ids[1].parent_index = None;
        assert!(
            !mirror
                .retained_identity_matches(&stale)
                .expect("stale topology")
        );
        let legacy = FnxSourceMirror::from_source(
            r#"<Frame name="Root"><Text name="Label" /></Frame>"#,
            &sidecar(),
        )
        .expect("legacy source");
        assert!(
            !legacy
                .retained_identity_matches(&sidecar())
                .expect("legacy identity")
        );
    }

    #[test]
    fn patching_one_element_preserves_all_unrelated_source_bytes() {
        let source = "// custom wrapper stays\nexport default () => (\n  <Frame   name=\"Root\">\n\t<Text name=\"Label\" content=\"héllo\" />\n  </Frame>\n);\n";
        let mut mirror = FnxSourceMirror::from_source(source, &sidecar()).unwrap();
        assert_eq!(mirror.render(), source);

        let mut root = parse_doc(source).unwrap();
        let previous = root.children[0].clone();
        root.children[0]
            .attrs
            .insert("content".into(), Value::String("changed".into()));
        mirror
            .patch_element_delta("child", &previous, &root.children[0])
            .unwrap();
        let patched = mirror.render();

        assert!(patched.starts_with("// custom wrapper stays\nexport default () => (\n"));
        assert!(patched.contains("<Frame   name=\"Root\">"));
        assert!(patched.contains("<Text name=\"Label\" content=\"changed\" />"));
        assert_eq!(patched.matches("<Text").count(), 1);
    }

    #[test]
    fn parent_tag_patches_shape_only_their_own_attributes() {
        let mut root = parse_doc(
            r#"<Rect id="root" name="Root" width={10} height={20}><Text id="child" name="Label" content="Keep this child" /></Rect>"#,
        )
        .expect("canonical vector parent");
        let source = crate::print_doc("Parent", &root);
        let mut identities = sidecar();
        identities.ids[0].tag = Some("Vector".into());
        let mut mirror = FnxSourceMirror::from_source(&source, &identities).expect("mirror");
        let original_child = root.children.first().expect("child").clone();
        let tag = opening_tag_element(&root);
        assert_eq!(tag.attrs, root.attrs);
        assert!(
            tag.children.is_empty(),
            "tag shaping must not copy descendants"
        );

        let previous = root.clone();
        root.attrs.insert("name".into(), Value::from("Renamed"));
        assert!(
            mirror
                .patch_element_delta("root", &previous, &root)
                .expect("parent attribute delta")
        );
        root.attrs.insert("opacity".into(), Value::from(0.5));
        assert!(
            mirror
                .patch_element("root", &root)
                .expect("parent tag patch")
        );
        let patched = mirror.render();
        assert!(patched.contains("<Vector ") && patched.contains("</Vector>"));
        assert!(!patched.contains("<Rect "));
        let parsed = parse_doc(&patched).expect("preserved parent form");
        assert_eq!(parsed, root);
        assert_eq!(parsed.children.first(), Some(&original_child));
    }

    #[test]
    fn structural_parent_insertions_preserve_child_presence_and_vector_tag() {
        for populated in [false, true] {
            let source = if populated {
                r#"<Frame id="root" name="Root"><Text id="child" name="Label" /></Frame>"#
            } else {
                r#"<Frame id="root" name="Root" />"#
            };
            let mut identities = sidecar();
            if !populated {
                identities.ids.pop();
            }
            let previous = ArtifactIr::from_source(ArtifactKind::Page, "Root", source, identities)
                .expect("previous IR");
            let mut next = previous.clone();
            let mut root = next.root().clone();
            root.children.push(
                parse_doc(
                    r#"<Rect id="vector" name="Parent" width={10} height={20}><Text id="nested" name="Nested" /></Rect>"#,
                )
                .expect("inserted vector parent"),
            );
            let mut identities = next.sidecar().clone();
            let vector_position = u32::try_from(identities.ids.len()).expect("small fixture");
            identities.ids.extend([
                IdEntry {
                    id: "vector".into(),
                    index: Value::from(2.0),
                    tag: Some("Vector".into()),
                    name: Some("Parent".into()),
                    parent_index: Some(0),
                },
                IdEntry {
                    id: "nested".into(),
                    index: Value::from(1.0),
                    tag: Some("Text".into()),
                    name: Some("Nested".into()),
                    parent_index: Some(vector_position),
                },
            ]);
            next.replace_structure(root, identities).expect("next IR");
            let mirror = FnxSourceMirror::from_source(source, previous.sidecar()).expect("mirror");
            let rebuilt = mirror
                .rebuild_tree(&previous, &next)
                .expect("insert parent")
                .render();
            assert!(rebuilt.contains("<Vector ") && rebuilt.contains("</Vector>"));
            assert!(!rebuilt.contains("<Rect "));
            assert!(rebuilt.contains("</Frame>"));
            assert_eq!(
                parse_doc(&rebuilt).expect("preserved parent forms"),
                *next.root()
            );
        }
    }

    #[test]
    fn duplicate_ids_and_attributes_are_rejected_before_indexing() {
        let mut duplicate_ids = sidecar();
        duplicate_ids.ids[1].id = duplicate_ids.ids[0].id.clone();
        assert!(FnxSourceMirror::from_source("<Frame><Text /></Frame>", &duplicate_ids).is_err());

        let duplicate_attr = "<Frame name=\"A\" name=\"B\"><Text /></Frame>";
        assert!(FnxSourceMirror::from_source(duplicate_attr, &sidecar()).is_err());
        assert!(parse_doc(duplicate_attr).is_err());
    }

    #[test]
    fn structural_insert_preserves_existing_subtree_and_jsx_comment() {
        let source = "// authored wrapper\nexport default () => (\n  <Frame id=\"root\" name=\"Root\">\n    {/* keep this explanation */}\n    <Text id=\"child\" name=\"Label\" content=\"héllo\" />\n  </Frame>\n);\n";
        let previous = ArtifactIr::from_source(ArtifactKind::Page, "Root", source, sidecar())
            .expect("previous IR");
        let mut next = previous.clone();
        let mut root = next.root().clone();
        let mut inserted = FnxElement::new("Text");
        inserted.attrs.insert("id".into(), Value::from("new-child"));
        inserted.attrs.insert("name".into(), Value::from("New"));
        root.children.push(inserted);
        let mut ids = next.sidecar().clone();
        ids.ids.push(IdEntry {
            id: "new-child".into(),
            index: Value::from(2),
            tag: Some("Text".into()),
            name: Some("New".into()),
            parent_index: Some(0),
        });
        next.replace_structure(root, ids).expect("next IR");
        let mirror = FnxSourceMirror::from_source(source, previous.sidecar()).expect("mirror");
        let rebuilt = mirror
            .rebuild_tree(&previous, &next)
            .expect("rebuild")
            .render();
        assert!(rebuilt.starts_with("// authored wrapper\n"));
        assert!(rebuilt.contains("{/* keep this explanation */}"));
        assert!(rebuilt.contains("<Text id=\"child\" name=\"Label\" content=\"héllo\" />"));
        assert!(rebuilt.contains("id=\"new-child\""));
        assert_eq!(parse_doc(&rebuilt).expect("parse"), *next.root());
    }

    #[test]
    fn incremental_structure_preserves_payloads_source_slots_comments_and_external_parent() {
        use crate::ir::StructuralNodeChange;
        use serde_json::json;

        let nodes = vec![
            json!({"id":"root","type":"group","parent":"outside","index":1,"name":"Root"}),
            json!({"id":"left","type":"group","parent":"root","index":1,"name":"Left"}),
            json!({"id":"right","type":"group","parent":"root","index":2,"name":"Right"}),
            json!({"id":"vector","type":"vector","parent":"left","index":1,"name":"Vector","future_geometry":vec![3; 8192]}),
            json!({"id":"sibling","type":"text","parent":"root","index":3,"name":"Sibling","opacity":0.4,"future_hint":"retain"}),
        ];
        let canonical =
            ArtifactIr::from_nodes(crate::ArtifactKind::Page, "Page", &nodes).expect("IR");
        let source = canonical.print();
        let identity = "id=\"vector\"";
        let opening = source[..source.find(identity).expect("vector identity")]
            .rfind('<')
            .expect("vector opening");
        let mut source = source;
        source.insert_str(opening, "{/* pinned vector comment */}\n      ");
        source.insert_str(0, "// exact wrapper\n");
        let mut ir = ArtifactIr::from_source(
            crate::ArtifactKind::Page,
            "Page",
            &source,
            canonical.sidecar().clone(),
        )
        .expect("authored IR");
        let mut mirror = FnxSourceMirror::from_source(&source, ir.sidecar()).expect("mirror");
        let vector_slot =
            Arc::clone(&mirror.pieces[mirror.nodes.get("vector").expect("vector slot").open]);
        let geometry_pointer = ir
            .element("vector")
            .expect("vector")
            .attrs
            .get("future_geometry")
            .and_then(Value::as_array)
            .expect("geometry")
            .as_ptr();
        let value = |ir: &ArtifactIr, id: &str| {
            ir.to_nodes()
                .expect("nodes")
                .into_iter()
                .find(|node| node.get("id").and_then(Value::as_str) == Some(id))
                .expect("node")
        };
        for step in 0..4 {
            let changes = match step {
                0 => {
                    let before = value(&ir, "vector");
                    let mut after = before.clone();
                    after["parent"] = json!("right");
                    vec![StructuralNodeChange {
                        id: "vector".into(),
                        before: Some(before),
                        after: Some(after),
                    }]
                }
                1 => {
                    let before = value(&ir, "sibling");
                    let mut after = before.clone();
                    after
                        .as_object_mut()
                        .expect("node object")
                        .remove("opacity");
                    vec![
                        StructuralNodeChange {
                            id: "inserted".into(),
                            before: None,
                            after: Some(
                                json!({"id":"inserted","type":"text","parent":"left","index":1,"name":"Inserted"}),
                            ),
                        },
                        StructuralNodeChange {
                            id: "sibling".into(),
                            before: Some(before),
                            after: Some(after),
                        },
                    ]
                }
                2 => {
                    let before = value(&ir, "right");
                    let mut after = before.clone();
                    after["index"] = json!(0.5);
                    vec![StructuralNodeChange {
                        id: "right".into(),
                        before: Some(before),
                        after: Some(after),
                    }]
                }
                _ => vec![StructuralNodeChange {
                    id: "inserted".into(),
                    before: Some(value(&ir, "inserted")),
                    after: None,
                }],
            };
            let patch = ir.prepare_structural_patch(&changes).expect("prepare");
            let next = mirror
                .prepare_structural_patch(&ir, &patch)
                .expect("prepare source");
            ir.commit_structural_patch(patch).expect("commit");
            mirror = next;
            let rendered = mirror.render();
            assert_eq!(parse_doc(&rendered).expect("source parse"), *ir.root());
            assert_eq!(
                crate::decode_subtree(&rendered, ir.sidecar()).expect("decode"),
                ir.to_nodes().expect("IR nodes")
            );
            assert!(rendered.starts_with("// exact wrapper\n"));
            assert_eq!(rendered.matches("{/* pinned vector comment */}").count(), 1);
            assert_eq!(ir.sidecar().root_parent.as_deref(), Some("outside"));
            assert!(
                Arc::ptr_eq(
                    &vector_slot,
                    &mirror.pieces[mirror.nodes.get("vector").expect("vector slot").open]
                ),
                "the unchanged large source tag must retain its allocation"
            );
            assert_eq!(
                geometry_pointer,
                ir.element("vector")
                    .expect("vector")
                    .attrs
                    .get("future_geometry")
                    .and_then(Value::as_array)
                    .expect("geometry")
                    .as_ptr(),
                "the unchanged semantic payload must move without deep cloning"
            );
        }
        assert_eq!(
            ir.root()
                .children
                .first()
                .and_then(|element| element.attrs.get("id")),
            Some(&json!("right"))
        );
        assert!(!mirror.render().contains("opacity={0.4}"));
        assert_eq!(
            ir.element("sibling")
                .expect("sibling")
                .attrs
                .get("future_hint"),
            Some(&json!("retain"))
        );
        let before = mirror.render();
        let patch = ir
            .prepare_structural_patch(&[StructuralNodeChange {
                id: "vector".into(),
                before: Some(value(&ir, "vector")),
                after: None,
            }])
            .expect("valid topology");
        assert!(
            mirror
                .prepare_structural_patch(&ir, &patch)
                .expect_err("deletion must retain authored comments")
                .to_string()
                .contains("comment")
        );
        assert_eq!(mirror.render(), before);
        assert!(ir.element("vector").is_some());
    }

    #[test]
    fn legacy_source_gets_explicit_ids_without_reformatting() {
        let source = "<Frame   name=\"Root\">\n <Text name=\"Label\" />\n</Frame>";
        let mut mirror = FnxSourceMirror::from_source(source, &sidecar()).expect("mirror");
        assert!(mirror.insert_missing_ids(&sidecar()).expect("insert IDs"));
        let rendered = mirror.render();
        assert!(rendered.contains("<Frame   name=\"Root\" id=\"root\">"));
        assert!(rendered.contains("<Text name=\"Label\" id=\"child\" />"));
    }
}
