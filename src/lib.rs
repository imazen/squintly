//! Squintly — psychovisual data collection for zensim.
//!
//! See `SPEC.md` for design rationale. Core flow:
//!
//! 1. Frontend (phone-first TS) calibrates viewing conditions, then asks for trials.
//! 2. [`coefficient`] supplies image manifests + raw bytes (HTTP or filesystem).
//! 3. [`sampling`] picks the next trial, balancing pair/single trial types and
//!    advancing per-source staircases.
//! 4. Responses persist to SQLite via [`db`].
//! 5. [`bt`] fits Bradley–Terry-Davidson scores; [`export`] writes zenanalyze TSVs.

pub mod asap;
pub mod auth;
pub mod bt;
pub mod coefficient;
pub mod content_class;
pub mod crowd_bt;
pub mod curator;
pub mod db;
pub mod db_health;
pub mod disposition;
pub mod exclusion;
pub mod export;
pub mod grading;
pub mod handle;
pub mod handlers;
pub mod jpeg_q;
pub mod licensing;
pub mod metrics;
pub mod metrics_api;
pub mod sampling;
pub mod source_label;
pub mod staircase;
pub mod stats;
pub mod streaks;
pub mod studies;
pub mod suggestion_store;
pub mod suggestions;
pub mod unified;
pub mod variant_gen;

pub use coefficient::{Coefficient, CoefficientSource};
