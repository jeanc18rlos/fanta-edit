//! Name-based references — the authoring seam that lets `.fnx` say
//! `component="Button"` instead of a bare ULID and `"$Theme/bg"` instead of a
//! variable id.
//!
//! ## Why a table, not a resolver callback
//!
//! The codec stays model-agnostic (no fanta-doc dependency), so it cannot look
//! names up itself. Callers distill their component library / variable
//! registry into a [`RefTable`] — two string↔string bijections — and the codec
//! treats it as pure data. The same table drives both directions:
//!
//! - **parse** ([`desugar_refs`]): names → ids, run as the LAST desugar pass so
//!   the in-memory [`FnxElement`] tree stays canonical (ULIDs only), exactly
//!   like the transform/size/shape sugars in [`crate::sugar`]. Nothing
//!   downstream (sidecar fingerprints, node maps, merge) ever observes a name.
//! - **print** ([`sugar_refs`]): ids → names, applied to a clone right before
//!   rendering, and only when the caller opted in (`emit_names`) — older
//!   project layouts keep emitting raw ULIDs byte-for-byte.
//!
//! ## Ambiguity and the escape hatch
//!
//! Names are user-controlled and freely collide. The rules:
//!
//! - A raw 26-char Crockford ULID string is ALWAYS accepted verbatim — the
//!   forever escape hatch. (A legitimate ULID never starts with `$`, and no
//!   attr value legitimately starts with `$` today, so `$`-paths are
//!   unambiguous too.)
//! - Resolving an unknown name is a hard parse error with a nearest-name
//!   suggestion (edit distance ≤ 2); resolving a duplicated name is a hard
//!   error telling the author to use the id.
//! - Printing NEVER emits an ambiguous name: the id→name side of the table
//!   only contains names that map back to exactly one id, so
//!   `parse(print(x)) == x` holds by construction. A name that itself looks
//!   like a ULID is also never emitted (parse would take the escape hatch and
//!   misread it as an id).
//!
//! ## Bindings map sugar (B4)
//!
//! `CanvasNode.bindings` serializes as a pair array (`crate::binding::map_as_seq`
//! in fanta-doc): `[[{"prop":"fill_color","index":0}, "<VariableId>"], …]`.
//! Authors may instead write the readable object form
//! `bindings={{"fill_color": "$Theme/bg", "stroke_color:1": "…"}}`; parse
//! desugars it into the exact pair-array shape (see [`BOUND_PROPS`] for the
//! serde mirror), and print re-sugars a pair array into the object form iff
//! EVERY entry sugars cleanly — a half-named array would be confusing, so it
//! is strictly all-or-nothing per attribute.

use crate::convert::FnxError;
use crate::model::FnxElement;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// BoundProp mirror
// ---------------------------------------------------------------------------

/// Every `fanta_doc::BoundProp` variant as `(serde tag, carries an index)`, in
/// DECLARATION order.
///
/// Three facts are mirrored here and pinned by dev-dependency drift tests in
/// `tests.rs` (this crate deliberately has no fanta-doc dependency):
///
/// 1. the snake_case serde tag of each variant (`#[serde(tag = "prop")]`);
/// 2. which variants carry a `u16` `index` field — for those, serde ALWAYS
///    serializes the index, including `0` (no skip attribute), so the map
///    sugar must emit `"index": 0` for a bare `fill_color` key;
/// 3. the declaration order — `BoundProp` derives `Ord`, `bindings` is a
///    `BTreeMap<BoundProp, _>`, and `map_as_seq` serializes it in key order,
///    so a desugared pair array sorted by (declaration position, index) is
///    byte-identical to what the doc itself serializes.
pub(crate) const BOUND_PROPS: &[(&str, bool)] = &[
    ("fill_color", true),
    ("stroke_color", true),
    ("stroke_width", true),
    ("corner_radius", false),
    ("opacity", false),
    ("visible", false),
    ("text_content", false),
    ("text_style", false),
    ("clip_width", false),
    ("clip_height", false),
];

// ---------------------------------------------------------------------------
// RefTable
// ---------------------------------------------------------------------------

/// How one name resolves inside a [`BiLookup`].
enum Resolution<'a> {
    /// Exactly one id carries this name.
    Unique(&'a str),
    /// Several ids share this name — resolvable only via the id escape hatch.
    Ambiguous,
    /// No id carries this name.
    Unknown,
}

/// One direction pair of a name↔id mapping with ambiguity tracking.
///
/// `by_name` remembers EVERY name ever inserted (so "unknown" and "ambiguous"
/// stay distinguishable errors), but a duplicated name degrades its slot to
/// `None`. `by_id` holds ONLY unambiguous names, so the print side can never
/// emit a spelling the parse side would resolve to a different id.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct BiLookup {
    by_name: BTreeMap<String, Option<String>>,
    by_id: BTreeMap<String, String>,
}

impl BiLookup {
    fn insert(&mut self, name: &str, id: &str) {
        match self.by_name.get_mut(name) {
            None => {
                self.by_name.insert(name.to_owned(), Some(id.to_owned()));
                self.by_id.insert(id.to_owned(), name.to_owned());
            }
            Some(slot) => {
                // Re-inserting the exact same pair is idempotent, not ambiguity.
                if slot.as_deref() == Some(id) {
                    return;
                }
                // Second distinct id for this name: retire the first id's
                // reverse entry and poison the name. The new id never enters
                // `by_id` either — neither side may print this name.
                if let Some(previous) = slot.take() {
                    self.by_id.remove(&previous);
                }
            }
        }
    }

    fn resolve(&self, name: &str) -> Resolution<'_> {
        match self.by_name.get(name) {
            Some(Some(id)) => Resolution::Unique(id),
            Some(None) => Resolution::Ambiguous,
            None => Resolution::Unknown,
        }
    }

    fn name_of(&self, id: &str) -> Option<&str> {
        self.by_id.get(id).map(String::as_str)
    }

    fn names(&self) -> impl Iterator<Item = &str> {
        self.by_name.keys().map(String::as_str)
    }
}

/// The name↔id context for one parse or print: components by display name,
/// variables by `"{collection_name}/{variable_name}"` path.
///
/// [`RefTable::default()`] is the no-op table: it neither resolves nor emits,
/// so every legacy call path (`parse_doc`, `print_doc`, sidecar
/// reconciliation) behaves byte-for-byte as before this feature existed. A
/// table built with [`RefTable::new`] ALWAYS resolves on parse — even when
/// empty, so an unknown name in a project with no components errors clearly
/// instead of leaking downstream as a bogus id — and emits names on print only
/// when constructed with `emit_names` (project layout v4+).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RefTable {
    emit_names: bool,
    /// `false` only for [`RefTable::default`]: distinguishes "no reference
    /// context exists" (pass names through untouched, exactly like the codec
    /// always did) from "the context is empty" (resolution applies and every
    /// name is unknown). The derived `Default` yields `false` here, which is
    /// exactly the inert table the legacy entry points rely on.
    resolving: bool,
    components: BiLookup,
    variables: BiLookup,
}

impl RefTable {
    pub fn new(emit_names: bool) -> Self {
        Self {
            emit_names,
            resolving: true,
            components: BiLookup::default(),
            variables: BiLookup::default(),
        }
    }

    /// Whether print re-sugars ids into names (project layout v4+).
    pub fn emit_names(&self) -> bool {
        self.emit_names
    }

    /// Register a component master under its display name. `id` is the bare
    /// ULID string exactly as it serializes inside node JSON.
    pub fn insert_component(&mut self, name: &str, id: &str) {
        self.components.insert(name, id);
    }

    /// Register a design-token variable under its canonical
    /// `"{collection_name}/{variable_name}"` path (no `$` prefix — the sigil
    /// belongs to the source spelling, not the table).
    pub fn insert_variable(&mut self, path: &str, id: &str) {
        self.variables.insert(path, id);
    }
}

// ---------------------------------------------------------------------------
// ULID escape hatch
// ---------------------------------------------------------------------------

/// Whether `text` reads as a bare 26-char Crockford-base32 ULID — the value
/// spelling the doc's ids serialize to.
///
/// Case-insensitive because the `ulid` crate's decoder accepts lowercase, and
/// deliberately WITHOUT the decoder's leading-digit overflow rule (`> '7'`
/// overflows 128 bits): anything that even LOOKS like an id is left verbatim
/// for the model layer to judge, so this check can only ever widen the escape
/// hatch, never mis-resolve an id as a name. The cost is that a component
/// literally named as 26 Crockford characters cannot be referenced by name —
/// the print side refuses to emit such a name for the same reason.
pub(crate) fn is_ulid_like(text: &str) -> bool {
    text.chars().count() == 26
        && text.chars().all(|c| {
            matches!(
                c.to_ascii_uppercase(),
                '0'..='9' | 'A'..='H' | 'J' | 'K' | 'M' | 'N' | 'P'..='T' | 'V'..='Z'
            )
        })
}

// ---------------------------------------------------------------------------
// Suggestions
// ---------------------------------------------------------------------------

/// Classic single-row Levenshtein distance, capped by the caller's tolerance
/// via the length pre-filter in [`nearest`]. Small inputs only (names), so the
/// O(a·b) table is fine and no new dependency is warranted.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, &ca) in a.iter().enumerate() {
        let mut previous_diagonal = row[0];
        row[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let substitution = previous_diagonal + usize::from(ca != cb);
            previous_diagonal = row[j + 1];
            row[j + 1] = substitution.min(previous_diagonal + 1).min(row[j] + 1);
        }
    }
    row[b.len()]
}

/// The known name closest to `target` within edit distance 2, ties broken
/// lexicographically so error messages are deterministic. Candidates whose
/// length differs by more than 2 cannot be within distance 2 and are skipped
/// before the quadratic distance runs.
fn nearest<'a>(candidates: impl Iterator<Item = &'a str>, target: &str) -> Option<&'a str> {
    let target_len = target.chars().count();
    let mut best: Option<(usize, &str)> = None;
    for candidate in candidates {
        if candidate.chars().count().abs_diff(target_len) > 2 {
            continue;
        }
        let distance = edit_distance(candidate, target);
        if distance > 2 {
            continue;
        }
        let better = match best {
            None => true,
            Some((best_distance, best_name)) => {
                distance < best_distance || (distance == best_distance && candidate < best_name)
            }
        };
        if better {
            best = Some((distance, candidate));
        }
    }
    best.map(|(_, name)| name)
}

/// `; did you mean "…"?` (with an optional caller-supplied prefix such as `$`),
/// or empty when nothing is close enough.
fn suggestion_suffix(candidate: Option<&str>, prefix: &str) -> String {
    match candidate {
        Some(name) => format!("; did you mean \"{prefix}{name}\"?"),
        None => String::new(),
    }
}

/// A human-readable element address for post-parse errors, mirroring
/// [`crate::sugar`]'s convention: char positions are gone once the tree is
/// built, so tag + `name` is the most precise address left.
fn element_ref(el: &FnxElement) -> String {
    match el.attrs.get("name").and_then(Value::as_str) {
        Some(name) => format!("<{} name={name:?}>", el.tag),
        None => format!("<{}>", el.tag),
    }
}

// ---------------------------------------------------------------------------
// Parse side: names → ids
// ---------------------------------------------------------------------------

/// Resolve every name-based reference in the subtree into its canonical ULID,
/// in place: `component` display names and `$Collection/Name` binding paths
/// (in both the object sugar and hand-edited pair arrays). Run as the last
/// parse-time pass. With a [`RefTable::default`] table this is a strict no-op.
pub fn desugar_refs(el: &mut FnxElement, table: &RefTable) -> Result<(), FnxError> {
    if !table.resolving {
        return Ok(());
    }
    desugar_element(el, table)?;
    for child in &mut el.children {
        desugar_refs(child, table)?;
    }
    Ok(())
}

fn desugar_element(el: &mut FnxElement, table: &RefTable) -> Result<(), FnxError> {
    // Resolve BEFORE mutating so error messages can address the element.
    let address = element_ref(el);

    if let Some(text) = el.attrs.get("component").and_then(Value::as_str) {
        if !is_ulid_like(text) {
            let id = match table.components.resolve(text) {
                Resolution::Unique(id) => id.to_owned(),
                Resolution::Ambiguous => {
                    return Err(FnxError::Parse(format!(
                        "ambiguous component name {text:?} on {address}: multiple components \
                         share this name; use the component id"
                    )));
                }
                Resolution::Unknown => {
                    return Err(FnxError::Parse(format!(
                        "unknown component {text:?} on {address}{}",
                        suggestion_suffix(nearest(table.components.names(), text), "")
                    )));
                }
            };
            el.attrs.insert("component".to_owned(), Value::String(id));
        }
    }

    let Some(bindings) = el.attrs.get("bindings") else {
        return Ok(());
    };
    match bindings {
        // B4 map sugar: `{"fill_color": "$Theme/bg", "stroke_color:1": "…"}`
        // → the canonical pair array, sorted the way the doc's BTreeMap
        // serializes (see [`BOUND_PROPS`]).
        Value::Object(map) => {
            let mut pairs: Vec<(usize, u64, Value)> = Vec::with_capacity(map.len());
            for (key, value) in map {
                let (position, index) = parse_binding_key(key, &address)?;
                let variable = resolve_binding_value(value, table, &address)?;
                let (name, indexed) = BOUND_PROPS[position];
                // `json!` preserves the written key order (serde_json's
                // preserve_order feature), matching serde's tag-then-fields
                // output for the internally tagged enum.
                let prop = if indexed {
                    json!({ "prop": name, "index": index })
                } else {
                    json!({ "prop": name })
                };
                pairs.push((position, index, json!([prop, variable])));
            }
            pairs.sort_by_key(|(position, index, _)| (*position, *index));
            if let Some(window) = pairs
                .windows(2)
                .find(|window| (window[0].0, window[0].1) == (window[1].0, window[1].1))
            {
                let (name, indexed) = BOUND_PROPS[window[0].0];
                let spelled = if indexed && window[0].1 != 0 {
                    format!("{name}:{}", window[0].1)
                } else {
                    name.to_owned()
                };
                return Err(FnxError::Parse(format!(
                    "duplicate binding property \"{spelled}\" on {address} (a bare name and \
                     \":0\" address the same slot)"
                )));
            }
            let array = Value::Array(pairs.into_iter().map(|(_, _, pair)| pair).collect());
            el.attrs.insert("bindings".to_owned(), array);
        }
        // Already the pair array (machine-written or hand-edited): still
        // resolve `$…` path strings inside it, so authors can token-ify one
        // entry without rewriting the attribute into the object form.
        Value::Array(_) => {
            // Two passes to keep the borrow local: resolve into replacements,
            // then write them back.
            let mut resolved: Vec<(usize, Value)> = Vec::new();
            if let Some(entries) = el.attrs.get("bindings").and_then(Value::as_array) {
                for (position, entry) in entries.iter().enumerate() {
                    let Some(items) = entry.as_array() else {
                        continue;
                    };
                    if let Some(value) = items.get(1)
                        && value.as_str().is_some_and(|text| text.starts_with('$'))
                    {
                        resolved.push((position, resolve_binding_value(value, table, &address)?));
                    }
                }
            }
            if !resolved.is_empty()
                && let Some(entries) = el.attrs.get_mut("bindings").and_then(Value::as_array_mut)
            {
                for (position, value) in resolved {
                    if let Some(slot) = entries
                        .get_mut(position)
                        .and_then(Value::as_array_mut)
                        .and_then(|items| items.get_mut(1))
                    {
                        *slot = value;
                    }
                }
            }
        }
        // Any other shape is not ours to police — the model layer reports it.
        _ => {}
    }
    Ok(())
}

/// Parse a map-sugar key — `prop_name` or `prop_name:N` — into the property's
/// [`BOUND_PROPS`] position and effective index (index 0 implied for indexed
/// properties spelled bare).
fn parse_binding_key(key: &str, address: &str) -> Result<(usize, u64), FnxError> {
    let (name, spelled_index) = match key.split_once(':') {
        Some((name, index)) => (name, Some(index)),
        None => (key, None),
    };
    let Some(position) = BOUND_PROPS.iter().position(|(known, _)| *known == name) else {
        return Err(FnxError::Parse(format!(
            "unknown binding property {name:?} on {address}{}",
            suggestion_suffix(
                nearest(BOUND_PROPS.iter().map(|(known, _)| *known), name),
                ""
            )
        )));
    };
    let indexed = BOUND_PROPS[position].1;
    let index = match (indexed, spelled_index) {
        (true, None) => 0,
        (true, Some(raw)) => raw.parse::<u16>().map_err(|_| {
            FnxError::Parse(format!(
                "binding property key {key:?} on {address}: the index after ':' must be an \
                 integer in 0..={}",
                u16::MAX
            ))
        })? as u64,
        (false, None) => 0,
        (false, Some(_)) => {
            return Err(FnxError::Parse(format!(
                "binding property \"{name}\" on {address} does not take an index"
            )));
        }
    };
    Ok((position, index))
}

/// Resolve one binding VALUE: a `$Collection/Name` path via the table, a raw
/// ULID verbatim; anything else is an authoring error (better caught here with
/// a suggestion than downstream as an opaque id-parse failure).
fn resolve_binding_value(
    value: &Value,
    table: &RefTable,
    address: &str,
) -> Result<Value, FnxError> {
    let Some(text) = value.as_str() else {
        return Err(FnxError::Parse(format!(
            "binding value on {address} must be a variable id string or a \
             \"$Collection/Name\" path"
        )));
    };
    if let Some(path) = text.strip_prefix('$') {
        return match table.variables.resolve(path) {
            Resolution::Unique(id) => Ok(Value::String(id.to_owned())),
            Resolution::Ambiguous => Err(FnxError::Parse(format!(
                "ambiguous variable path {text:?} on {address}: multiple variables share this \
                 path; use the variable id"
            ))),
            Resolution::Unknown => Err(FnxError::Parse(format!(
                "unknown variable {text:?} on {address}{}",
                suggestion_suffix(nearest(table.variables.names(), path), "$")
            ))),
        };
    }
    if is_ulid_like(text) {
        return Ok(value.clone());
    }
    Err(FnxError::Parse(format!(
        "binding value {text:?} on {address} must be a variable id or a \
         \"$Collection/Name\" path{}",
        suggestion_suffix(nearest(table.variables.names(), text), "$")
    )))
}

// ---------------------------------------------------------------------------
// Print side: ids → names
// ---------------------------------------------------------------------------

/// Re-spell canonical references as names over the whole subtree, in place, on
/// a clone right before printing — and only when the table opted into
/// `emit_names`. Strictly lossless by construction: only unambiguous,
/// non-ULID-shaped names are emitted, and a bindings pair array becomes the
/// object form iff every entry sugars (all-or-nothing, so a partially named
/// array can never appear).
pub fn sugar_refs(el: &mut FnxElement, table: &RefTable) {
    if !table.emit_names {
        return;
    }
    sugar_tree(el, table);
}

fn sugar_tree(el: &mut FnxElement, table: &RefTable) {
    sugar_element(el, table);
    for child in &mut el.children {
        sugar_tree(child, table);
    }
}

/// The single-element sugar step, exposed to [`crate::source::FnxSourceMirror`]
/// so canvas patches re-sugar reference spellings without touching children
/// (each child owns its own source span). Callers gate on
/// [`RefTable::emit_names`] themselves.
pub(crate) fn sugar_element(el: &mut FnxElement, table: &RefTable) {
    if let Some(id) = el.attrs.get("component").and_then(Value::as_str)
        && let Some(name) = table.components.name_of(id)
        // A ULID-shaped name would take the parse-time escape hatch and be
        // misread as an id; a `$`-leading name would be misread as a path
        // sigil if the component grammar ever grows one. Neither may print.
        && !is_ulid_like(name)
        && !name.starts_with('$')
    {
        let name = name.to_owned();
        el.attrs.insert("component".to_owned(), Value::String(name));
    }

    if let Some(pairs) = el.attrs.get("bindings").and_then(Value::as_array)
        && let Some(object) = sugared_bindings(pairs, table)
    {
        el.attrs
            .insert("bindings".to_owned(), Value::Object(object));
    }
}

/// The object form of a bindings pair array, or `None` unless EVERY entry
/// sugars cleanly (recognized prop shape, unambiguous variable path, no
/// duplicate keys). `None` leaves the array verbatim — including its ULIDs;
/// arrays never carry `$` names.
fn sugared_bindings(pairs: &[Value], table: &RefTable) -> Option<Map<String, Value>> {
    let mut object = Map::new();
    for pair in pairs {
        let items = pair.as_array()?;
        let [prop, variable] = items.as_slice() else {
            return None;
        };
        let key = binding_key_of(prop)?;
        let id = variable.as_str()?;
        let path = table.variables.name_of(id)?;
        // Sugar and desugar must be inverses: refuse a path the parse side
        // would not resolve back (a poisoned or sigil-leading spelling can't
        // occur — paths in the table resolve by construction — but a path is
        // still free to contain anything, so nothing further to check).
        let previous = object.insert(key, Value::String(format!("${path}")));
        if previous.is_some() {
            // Duplicate BoundProp in a hand-edited array — the object form
            // would silently drop one side.
            return None;
        }
    }
    Some(object)
}

/// The map-sugar key for one serialized `BoundProp`, or `None` when the shape
/// is not the exact serde projection this crate mirrors (unknown prop, extra
/// fields, index on a non-indexed prop, missing/oversized index) — those must
/// keep the verbatim array so a newer doc's data round-trips untouched.
fn binding_key_of(prop: &Value) -> Option<String> {
    let object = prop.as_object()?;
    let name = object.get("prop")?.as_str()?;
    let (_, indexed) = BOUND_PROPS
        .iter()
        .find(|(known, _)| *known == name)
        .copied()?;
    if indexed {
        if object.len() != 2 {
            return None;
        }
        let index = object.get("index")?.as_u64()?;
        if index > u64::from(u16::MAX) {
            return None;
        }
        // Index 0 is implied by the bare key; other indices spell `name:N`.
        Some(if index == 0 {
            name.to_owned()
        } else {
            format!("{name}:{index}")
        })
    } else {
        (object.len() == 1).then(|| name.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ulid_detection_accepts_both_cases_and_rejects_excluded_letters() {
        assert!(is_ulid_like("01ARZ3NDEKTSV4RRFFQ69G5FAV"));
        assert!(is_ulid_like("01arz3ndektsv4rrffq69g5fav"));
        assert!(!is_ulid_like("01ARZ3NDEKTSV4RRFFQ69G5FA")); // 25 chars
        assert!(!is_ulid_like("01ARZ3NDEKTSV4RRFFQ69G5FAI")); // 'I' excluded
        assert!(!is_ulid_like("Button"));
        assert!(!is_ulid_like("$Theme/bg"));
    }

    #[test]
    fn duplicate_names_are_poisoned_in_both_directions() {
        let mut lookup = BiLookup::default();
        lookup.insert("Button", "01AAAAAAAAAAAAAAAAAAAAAAAA");
        lookup.insert("Button", "01BBBBBBBBBBBBBBBBBBBBBBBB");
        assert!(matches!(lookup.resolve("Button"), Resolution::Ambiguous));
        assert_eq!(lookup.name_of("01AAAAAAAAAAAAAAAAAAAAAAAA"), None);
        assert_eq!(lookup.name_of("01BBBBBBBBBBBBBBBBBBBBBBBB"), None);
        // Idempotent re-insert of the same pair does not poison.
        let mut lookup = BiLookup::default();
        lookup.insert("Chip", "01AAAAAAAAAAAAAAAAAAAAAAAA");
        lookup.insert("Chip", "01AAAAAAAAAAAAAAAAAAAAAAAA");
        assert!(matches!(lookup.resolve("Chip"), Resolution::Unique(_)));
    }

    #[test]
    fn nearest_prefers_smallest_distance_then_lexicographic() {
        let names = ["Button", "Buttons", "Banner"];
        assert_eq!(nearest(names.iter().copied(), "Buton"), Some("Button"));
        // Distance ties ("Card" vs "Cart" from "Carb") break lexicographically.
        let names = ["Cart", "Card"];
        assert_eq!(nearest(names.iter().copied(), "Carb"), Some("Card"));
        assert_eq!(nearest(names.iter().copied(), "Sidebar"), None);
    }
}
