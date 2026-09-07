#![allow(deprecated)]

#[cfg(test)]
use gpui::SharedString;

mod action;
mod component;
mod effect;
mod export;
mod layout;
mod local_styles;
mod media;
mod node;
mod paint;
mod property;
mod selection;
mod shape;
mod stroke;
mod typography;
mod variables;
mod viewer;
mod workspace;

pub use action::*;
pub use component::*;
pub use effect::*;
pub use export::*;
pub use layout::*;
pub use local_styles::*;
pub use media::*;
pub use node::*;
pub use paint::*;
pub use property::*;
pub use selection::*;
pub use shape::*;
pub use stroke::*;
pub use typography::*;
pub use variables::*;
pub use viewer::*;
pub use workspace::*;

#[cfg(test)]
mod tests;
