//! GSB-PCB: offline Gerber/Excellon/stackup import, geometry reconstruction
//! and electrical connectivity candidate review.
//!
//! The crate is layered so business rules live outside the browser:
//! [`domain`] raw (immutable) material models, [`parse`] file parsers,
//! [`geometry`] derived reconstruction/connectivity, [`params`] versioned
//! rule parameters, [`store`] the SQLite event store and [`app`] the
//! replay/application service used by both the HTTP server and tests.

pub mod app;
pub mod domain;
pub mod geometry;
pub mod params;
pub mod parse;
pub mod store;
pub mod web;
