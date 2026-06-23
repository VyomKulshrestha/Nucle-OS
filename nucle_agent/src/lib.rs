//! # nucle_agent — ReAct Agent Interface
//!
//! Takes natural-language file operations, plans across the VFS layer,
//! and executes them as multi-step pipelines.
//!
//! "Store last year's medical archive with 3x redundancy" becomes
//! a full agentic pipeline down to the encoding layer.

pub mod tools;
pub mod planner;
pub mod executor;
