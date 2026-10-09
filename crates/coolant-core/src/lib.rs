//! Platform-independent core of coolant: classify processes, profile a
//! machine over time, and turn the numbers into findings.

pub mod classify;
pub mod profile;
pub mod report;

pub use coolant_platform as platform;
