//! Copperline — offline PCB/Gerber/geometric connectivity review workbench.

pub mod model;
pub mod id;
pub mod parse;
pub mod geom;
pub mod params;
pub mod cases;
pub mod pipeline;
pub mod store;
pub mod export;
pub mod web;

pub use pipeline::{Engine, EventRecord, ViewState, Decision};
pub use params::Params;
