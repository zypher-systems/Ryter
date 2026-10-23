//! The bench inside the binary: its tasks, a runner that gives each one to
//! the build hat in a fresh copy of the task's project and scores it by the
//! hidden tests, and the report that says tasks solved per dollar.
//!
//! The fixtures are embedded at build time from `bench/` in the repository
//! (`task.toml`, `repo/`, `hidden/`; never a solution). `ryter bench` and the
//! `/benchmark` panel both run through [`run`].

mod fixtures {
    include!(concat!(env!("OUT_DIR"), "/fixtures.rs"));
}
pub mod report;
pub mod run;
pub mod task;

pub use report::{Check, Report, Summary, TaskResult};
pub use run::{Cancel, Options, Progress, RESERVE_USD, allowance, run};
pub use task::{File, Level, Task, select, tasks};
