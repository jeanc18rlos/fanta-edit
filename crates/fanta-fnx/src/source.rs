//! Source-preserving FNX mirror.
//!
//! The semantic [`FnxElement`](crate::FnxElement) tree is the editable virtual
//! DOM. This module keeps the original source split into immutable trivia/body
//! pieces and stable opening/closing-tag slots keyed by sidecar identity.
//! Updating one node replaces only its tag slots; comments, wrapper code,
//! whitespace, and every unrelated node remain byte-for-byte unchanged.

use crate::api::FnxSidecar;
use crate::convert::FnxError;
use crate::ir::ArtifactIr;
use crate::model::{FnxElement, SUGAR_TAGS};
use crate::print::{render_attr, render_close_tag, render_open_tag};
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
    pieces: Vec<String>,
    nodes: BTreeMap<String, NodePieces>,
    /// The name↔id context this mirror re-sugars reference spellings with when
    /// a canvas patch reprints an attribute: a recolor that swaps a bound
    /// variable id prints the `$Collection/Name` path (and a component swap
    /// prints the new component's name) when the table opted into
    /// `emit_names`. Shared via `Arc` so the owning session can rebuild one
    /// table on variable changes and hand every retained mirror the same one.
    refs: Arc<RefTable>,
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
            pieces.push(source[cursor..token.range.start].to_owned());
            let piece = pieces.len();
            pieces.push(source[token.range.clone()].to_owned());
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
        pieces.push(source[cursor..].to_owned());
        if nodes.values().any(|node| node.open == usize::MAX) {
            return Err(FnxError::Parse(
                "FNX source index contains a node without an opening tag".into(),
            ));
        }
        Ok(Self {
            pieces,
            nodes,
            refs,
        })
    }

    /// Replace the retained name↔id context. Called when the owning session's
    /// component library / variable registry changes (a rename can make a
    /// spelling ambiguous or newly available); already-written pieces are
    /// untouched — the table only shapes FUTURE patches.
    pub fn set_ref_table(&mut self, refs: Arc<RefTable>) {
        self.refs = refs;
    }

    /// The element with reference ids re-sugared into names, when the retained
    /// table opted into `emit_names` — mirroring what the canonical printer
    /// would emit, so a patched tag never regresses a `$path`/name spelling
    /// back to a raw ULID.
    fn ref_shaped(&self, element: &FnxElement) -> FnxElement {
        let mut shaped = element.clone();
        if self.refs.emit_names() {
            crate::refs::sugar_element(&mut shaped, &self.refs);
        }
        shaped
    }

    /// Materialize the current source bytes.
    pub fn render(&self) -> String {
        let capacity = self.pieces.iter().map(String::len).sum();
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
            let opening = &mut self.pieces[node.open];
            let layout = OpenTagLayout::parse(opening)?;
            if layout.attrs.contains_key("id") {
                continue;
            }
            let attribute = format!(
                " id={}",
                render_attr(&serde_json::Value::String(entry.id.clone()))
            );
            opening.insert_str(layout.insert_at, &attribute);
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
        Self::from_source_with(&rebuilt, next.sidecar(), Arc::clone(&self.refs))
    }

    /// Replace one node's semantic tag/attributes without touching any other
    /// source span.
    ///
    /// Structural changes (switching between self-closing and parent form)
    /// require a subtree patch and are intentionally rejected here.
    pub fn patch_element(&mut self, id: &str, element: &FnxElement) -> Result<bool, FnxError> {
        let node = self
            .nodes
            .get(id)
            .ok_or_else(|| FnxError::Parse(format!("source mirror has no node id {id}")))?
            .clone();
        let has_children = !element.children.is_empty();
        if has_children != node.close.is_some() {
            return Err(FnxError::Parse(format!(
                "node {id} changed source structure; a tag-only patch is unsafe"
            )));
        }

        // A full-tag reprint uses canonical attribute spelling, but reference
        // spellings still re-sugar (names / `$paths`) so a fallback reprint of
        // an instance keeps the readable form the printer would emit.
        let element = self.ref_shaped(element);
        let open = render_open_tag(&element);
        let mut changed = self.pieces[node.open] != open;
        self.pieces[node.open] = open;
        if let Some(close_piece) = node.close {
            let close = render_close_tag(&element);
            changed |= self.pieces[close_piece] != close;
            self.pieces[close_piece] = close;
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
        if previous.tag != next.tag {
            return self.patch_element(id, next);
        }
        let node = self
            .nodes
            .get(id)
            .ok_or_else(|| FnxError::Parse(format!("source mirror has no node id {id}")))?
            .clone();
        if previous.children.is_empty() != next.children.is_empty()
            || next.children.is_empty() != node.close.is_none()
        {
            return Err(FnxError::Parse(format!(
                "node {id} changed source structure; an attribute patch is unsafe"
            )));
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
                _ => return self.patch_element(id, next),
            }
        } else {
            (previous.clone(), next.clone())
        };
        // Re-sugar reference spellings on BOTH sides before diffing: an
        // unchanged reference then compares equal in whichever spelling the
        // author used (no spurious edit), and a genuinely changed one is
        // rewritten in the printer's readable form — e.g. binding a new
        // variable prints its `$Collection/Name` path, not a raw ULID.
        let previous = self.ref_shaped(&previous);
        let next = self.ref_shaped(&next);
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
        let mut patched = opening.clone();
        for edit in edits {
            patched.replace_range(edit.range, &edit.replacement);
        }
        let changed = patched != *opening;
        self.pieces[node.open] = patched;
        Ok(changed)
    }
}

#[derive(Debug)]
struct TextEdit {
    range: Range<usize>,
    replacement: String,
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

        let mut shaped = element.clone();
        crate::sugar::sugar_shapes(&mut shaped);
        crate::sugar::sugar_transform(&mut shaped);
        crate::refs::sugar_refs(&mut shaped, self.refs);
        let reusable = old_position.filter(|position| {
            let old_element = self.previous_elements[*position].0;
            old_element.tag == element.tag
                && old_element.children.is_empty() == element.children.is_empty()
        });
        if let Some(position) = reusable {
            let span = &self.spans[position];
            out.push_str(&self.source[span.open.clone()]);
        } else {
            out.push_str(&render_open_tag(&shaped));
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
            let old_leading = old_position
                .zip(child_old_position)
                .filter(|(parent, child)| self.previous_elements[*child].1 == Some(*parent))
                .and_then(|(_, child)| self.leading[child].as_ref());
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
                out.push_str(&render_close_tag(&shaped));
            }
        } else {
            out.push('\n');
            out.push_str(&"  ".repeat(depth));
            out.push_str(&render_close_tag(&shaped));
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
    fn legacy_source_gets_explicit_ids_without_reformatting() {
        let source = "<Frame   name=\"Root\">\n <Text name=\"Label\" />\n</Frame>";
        let mut mirror = FnxSourceMirror::from_source(source, &sidecar()).expect("mirror");
        assert!(mirror.insert_missing_ids(&sidecar()).expect("insert IDs"));
        let rendered = mirror.render();
        assert!(rendered.contains("<Frame   name=\"Root\" id=\"root\">"));
        assert!(rendered.contains("<Text name=\"Label\" id=\"child\" />"));
    }
}
