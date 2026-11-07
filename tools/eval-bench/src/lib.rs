//! Scenario benchmarks of the production evaluator using compiler-checked IR.
//! This driver is deliberately separate from both compiler and evaluator crates.
#[cfg(feature = "alloc-metrics")]
mod allocation;
mod fixtures;
mod runner;

pub fn main_entry() -> std::process::ExitCode {
    match runner::run(std::env::args().skip(1)) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("eval-bench: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
