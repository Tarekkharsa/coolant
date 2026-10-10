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
//! - [`host`] finds security software that slows every program start,
//!   several container VMs at once, and duplicate dev servers.
//! - [`crashes`] finds programs that keep crashing (and keep ReportCrash busy).
//! - [`actions`] applies a doctor fix in one step, for the app's buttons.
//!
//! Everything that changes the disk supports a dry run and refuses to touch
//! protected paths or folders a running process is using. Unix-only.
#![cfg(unix)]

pub mod actions;
pub mod config;
pub mod crashes;
pub mod discover;
pub mod doctor;
pub mod exposed;
pub mod fsutil;
pub mod host;
pub mod inuse;
pub mod prewarm;
pub mod sweep;
pub mod tune;

pub use config::Config;
pub use discover::{Worktree, worktrees};
