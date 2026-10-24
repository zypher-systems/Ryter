//! The bench inside the binary: its tasks, a runner that gives each one to
//! the build hat in a fresh copy of the task's project and scores it by the
//! hidden tests, and the report that says tasks solved per dollar.
//!
//! The fixtures are embedded at build time from `bench/` in the repository
//! (`task.toml`, `repo/`, `hidden/`; never a solution). `ryter bench` and the
//! `/benchmark` panel both run through [`run_models`]: one run per model, and
//! a comparison when there are several.

mod fixtures {
    include!(concat!(env!("OUT_DIR"), "/fixtures.rs"));
}
pub mod compare;
pub mod lead;
pub mod report;
pub mod run;
pub mod task;

pub use compare::{Comparison, ModelRow, Seat, folder, model_folders, run_models, slug};
pub use lead::{adopt_orphans, end_orphans, guard_available, lead};
pub use report::{Check, Report, Summary, TaskResult};
pub use run::{
    Cancel, Options, Progress, RESERVE_USD, allowance, check_binary, default_sandbox, fresh_dir,
    guard_note, own_binary, run,
};
pub use task::{File, Level, Task, select, tasks};
