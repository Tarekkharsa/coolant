//! Runs agent-style workloads (many small files, git, process spawns,
//! dependency copies) while profiling the whole machine, to measure how
//! much side work macOS does in reaction: Spotlight (mds_stores, mdworker),
//! code-signing and malware checks (syspolicyd, XProtect, amfid), fseventsd
//! and friends.
//!
//! Every case is run in two locations that differ only in whether Spotlight
//! indexes them: a plain directory under the home folder, and the same layout
//! under a directory whose name ends in ".noindex", which Spotlight skips.
//!
//! Unix-only for now: the workloads use getrusage, cp -c and Unix file modes.
#![cfg(unix)]

pub mod config;
pub mod markdown;
pub mod probe;
pub mod run;
pub mod scenarios;
mod util;

pub use config::*;
pub use markdown::markdown;
pub use probe::{SpotlightProbe, probe_markdown, probe_spotlight};
pub use run::{BenchResult, Metrics, ProcCpu, run, run_partial};
pub use util::Canceled;

#[cfg(test)]
mod tests;
