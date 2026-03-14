//! Multi-agent email triage and queueing for MarianneAI.
//!
//! This crate mirrors the supervisor-worker pattern used in the referenced
//! `industrial_doc_analyzer` project, but applies it to Gmail intake:
//!
//! 1. Read recent Gmail messages through `gws`
//! 2. Classify whether they relate to MarianneAI
//! 3. Extract user intent from relevant emails
//! 4. Validate whether the request is clear enough to become a queued task
//! 5. Persist valid tasks to Postgres on GCP
//!
//! Exported modules intentionally stay small to allow replacing individual stages
//! without touching the orchestrator contract.

pub mod agentic;
pub mod compat;
pub mod gmail;
pub mod queue;
pub mod types;

pub use agentic::{AgentError, Orchestrator, Specialist, TurnResult};
