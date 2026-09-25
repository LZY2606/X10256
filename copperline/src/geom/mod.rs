//! Geometry reconstruction and connectivity evaluation.
//!
//! Units are millimetres on the derived layer; the source layer keeps raw
//! tokens. Polarity is superposed in strict operation order (dark adds, clear
//! subtracts). Contact tolerance is a versioned parameter supplied by the
//! caller, never a hard-coded constant inside the predicates.

pub mod polygon;
pub mod build;
pub mod connect;

pub use build::{reconstruct, ReconstructedLayer, PlatedBarrel, Budget};
pub use connect::{connectivity, Link, LinkKind, Component, ConnectivityResult, Proximity};
pub use polygon::{polygon_area, polygons_touch, polygons_distance, Polygon};
