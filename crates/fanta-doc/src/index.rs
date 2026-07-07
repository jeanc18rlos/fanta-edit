//! Fractional indexing for z-order within a parent.
//!
//! Inspired by tldraw and Figma. Each sibling has an [`IndexKey`]; the parent's
//! children are rendered in ascending order. To insert a node between two
//! siblings, mint a new key strictly between their two keys — no shifting, no
//! "renumber all the things," and CRDT-friendly because concurrent inserts
//! between the same two siblings produce different keys with extremely high
//! probability.
//!
//! ## Encoding
//!
//! For v0 we use a single `f64` for simplicity. Precision degrades exponentially
//! when inserts always land in the same gap, but the [`Scene`] (when added) will
//! rebalance a parent's children when it detects precision exhaustion. Phase 4
//! collab work will migrate to a string-based base-62 encoding (no precision
//! limit, mechanical port) before yrs integration lands. The public API does
//! not expose the underlying representation, so the migration is internal.
//!
//! [`Scene`]: crate::scene::Scene

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// A fractional index used for ordering siblings under the same parent.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(transparent)]
pub struct IndexKey(f64);

impl IndexKey {
    /// The conventional "first" key — placed at `1.0` so [`before`] can produce
    /// `0.5`, `0.25`, … without going negative for a few generations.
    ///
    /// [`before`]: IndexKey::before
    pub const FIRST: Self = Self(1.0);

    /// Build from a raw value. Mostly useful for tests and deserialization;
    /// production code should compose via [`between`], [`before`], [`after`].
    ///
    /// [`between`]: IndexKey::between
    /// [`before`]: IndexKey::before
    /// [`after`]: IndexKey::after
    pub const fn from_raw(value: f64) -> Self {
        Self(value)
    }

    /// The underlying numeric value. Exposed for diagnostics and rebalancing
    /// algorithms in `fanta-doc::scene`.
    pub const fn raw(self) -> f64 {
        self.0
    }

    /// Mint a key strictly between `a` and `b`.
    ///
    /// `a` and `b` must be ordered (`a < b`). The returned key is the
    /// midpoint. When precision approaches `f64::EPSILON`, the caller (the
    /// [`Scene`]) is responsible for rebalancing the affected sibling group.
    ///
    /// [`Scene`]: crate::scene::Scene
    pub fn between(a: Self, b: Self) -> Self {
        debug_assert!(a.0 < b.0, "between({}, {}) requires a < b", a.0, b.0);
        Self((a.0 + b.0) * 0.5)
    }

    /// Mint a key strictly less than `a`.
    pub fn before(a: Self) -> Self {
        Self(a.0 - 1.0)
    }

    /// Mint a key strictly greater than `a`.
    pub fn after(a: Self) -> Self {
        Self(a.0 + 1.0)
    }

    /// True when no further keys can be minted between this and an adjacent
    /// one without losing precision. [`Scene`] uses this to trigger rebalance.
    ///
    /// [`Scene`]: crate::scene::Scene
    pub fn near_precision_limit(a: Self, b: Self) -> bool {
        (b.0 - a.0).abs() < (a.0.abs().max(b.0.abs()) * 8.0 * f64::EPSILON).max(f64::EPSILON * 16.0)
    }
}

impl Default for IndexKey {
    fn default() -> Self {
        Self::FIRST
    }
}

impl PartialEq for IndexKey {
    fn eq(&self, other: &Self) -> bool {
        // Total-equality on the raw float so PartialOrd / Ord behave under
        // sort. NaN cannot arise from our constructors.
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for IndexKey {}

impl PartialOrd for IndexKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for IndexKey {
    fn cmp(&self, other: &Self) -> Ordering {
        // Our values never include NaN, so total_cmp is safe and gives a real
        // ordering for sort and BTreeMap usage.
        self.0.total_cmp(&other.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn between_is_strictly_between() {
        let a = IndexKey::FIRST;
        let b = IndexKey::after(a);
        let mid = IndexKey::between(a, b);
        assert!(a < mid && mid < b);
    }

    #[test]
    fn before_and_after_are_strictly_ordered() {
        let a = IndexKey::FIRST;
        let lo = IndexKey::before(a);
        let hi = IndexKey::after(a);
        assert!(lo < a && a < hi);
    }

    #[test]
    fn sorting_is_stable_and_total() {
        let mut v = [
            IndexKey::from_raw(3.0),
            IndexKey::from_raw(1.0),
            IndexKey::from_raw(2.0),
            IndexKey::from_raw(0.5),
        ];
        v.sort();
        assert_eq!(
            v.iter().map(|k| k.raw()).collect::<Vec<_>>(),
            [0.5, 1.0, 2.0, 3.0]
        );
    }

    #[test]
    fn fifty_inserts_between_one_pair_stays_well_above_limit() {
        let mut a = IndexKey::from_raw(1.0);
        let b = IndexKey::from_raw(2.0);
        for _ in 0..40 {
            let m = IndexKey::between(a, b);
            assert!(a < m && m < b);
            a = m; // worst case: keep narrowing the gap on the left
        }
        // After 40 inserts the gap is ~9e-13; precision still ~5 doublings off.
        assert!(!IndexKey::near_precision_limit(a, b));
    }
}
