//! PCB copper-layer connectivity workbench — offline, server-authoritative core.

pub mod engine;
pub mod error;
pub mod model;
pub mod parser;
pub mod case;
pub mod history;
pub mod project;
pub mod storage;
pub mod exchange;
pub mod web;

pub use error::{Diagnostic, DomainResult, Failure, FailureKind};
pub use project::ProjectService;
