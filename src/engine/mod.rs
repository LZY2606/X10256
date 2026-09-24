//! Geometry / connectivity engine.
//!
//! The engine is pure and directly callable from tests. Everything works on
//! normalized nanometre coordinates; original representations live on the
//! immutable materials.

pub mod raster;
pub mod rules;
pub mod state;
pub mod compute;

pub use compute::compute_state;
pub use rules::{RuleParams, RuleVersion, TANGENT_CANDIDATE, TANGENT_CONNECTED, TANGENT_GAP};
pub use state::{
    Ambiguity, CandidateSolution, ComponentNode, ConnectionKind, LinkEdge, PaintRender,
    RecomputedState, RenderAperture, RenderHit, RenderLayer, RenderOp,
};
