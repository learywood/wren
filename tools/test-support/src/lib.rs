#![cfg_attr(not(windows), allow(dead_code))]
#![allow(clippy::missing_errors_doc)]

#[cfg(not(windows))]
compile_error!("wren-test-support supports Windows only");

pub mod artifacts;
pub mod environment;
pub mod install;
pub mod process;
pub mod workspace;

pub use environment::EnvironmentPolicy;
pub use install::ReleaseInstallation;
pub use process::{ProcessRequest, ProcessResult, TreeCleanup, run_process};
pub use workspace::{IsolatedWorkspace, copy_tree};
