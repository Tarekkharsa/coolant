//! The workspace layer: find the git worktrees agents work in and reduce the
//! heat they cause.
//!
//! - [`doctor`] checks the setup and running processes and suggests fixes.
//! - [`tune`] moves worktree folders out of Spotlight's reach (and Time
//!   Machine's) while leaving a symlink, so tools keep using the same paths.
//! - [`prewarm`] clones build artifacts (target/, node_modules, ...) from the
//!   main checkout into a new worktree with copy-on-write, so the first build
//!   is incremental instead of a full cold compile.
//! - [`sweep`] deletes build artifacts in worktrees nobody is using anymore.
//!
//! Everything that changes the disk supports a dry run and refuses to touch
//! protected paths or folders a running process is using. Unix-only.
#![cfg(unix)]

pub mod config;
pub mod discover;
pub mod doctor;
pub mod exposed;
pub mod fsutil;
pub mod inuse;
pub mod prewarm;
pub mod sweep;
pub mod tune;

pub use config::Config;
pub use discover::{Worktree, worktrees};
