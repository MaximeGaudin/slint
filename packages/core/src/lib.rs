//! slint — the linter for Agent Skills.
//!
//! A skill is an instruction document an agent selects from a description and then follows without
//! being able to ask a question. Two things go wrong with them, and neither is visible in a diff:
//! the skill is never selected, or it is selected and followed badly. This is a linter for both.
//!
//! Everything that can be answered from the text is answered from the text — no network, no tokens,
//! no waiting. What genuinely needs a reader gets one model call per skill, from whichever provider
//! the project configured, and the report always says which half ran.
//!
//! The crate is a library first so the CLI, the editor integration and the tests all drive the same
//! code. `engine::run` is the whole tool in one call.

pub mod config;
pub mod diagnostics;
pub mod engine;
pub mod fix;
pub mod llm;
pub mod plugin;
pub mod report;
pub mod rules;
pub mod skill;

pub use diagnostics::{Message, Report, Severity, SkillReport};
pub use engine::{Passes, run};
