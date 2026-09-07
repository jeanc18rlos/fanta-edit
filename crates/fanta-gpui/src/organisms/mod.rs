//! Organisms: the host-facing feature surfaces (ARCHITECTURE.md §16).
//!
//! Each organism is a controlled component following the engine seam (§2-§4):
//! hosts supply immutable view data and apply the typed intents it emits.

pub mod assets;
pub mod design;
pub mod layers;
pub mod pages;
pub mod prototype;
pub mod timeline;
pub mod toolbar;
pub mod variables;
