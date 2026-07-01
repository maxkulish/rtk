use crate::tracking;
use anyhow::{bail, Context, Result};
use std::io::{self, Read};

/// Read stdin and apply a named filter, for shell pipe chaining.
///
/// Lets a raw command be piped through an RTK filter without RTK spawning the
/// command itself, e.g.:
///   cargo test 2>&1 | rtk pipe cargo-test
///   pytest | rtk pipe pytest
pub fn run(filter_name: &str, verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    if verbose > 0 {
        eprintln!("pipe: applying '{filter_name}' filter to stdin");
    }

    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .context("failed to read stdin for rtk pipe")?;

    let filtered = apply_named_filter(filter_name, &input)?;
    // Never emit more than the raw input (RTK must not be worse than a plain pipe).
    let output = crate::guard::never_worse(&input, &filtered);
    print!("{output}");

    timer.track(
        &format!("<stdin> | {filter_name}"),
        &format!("rtk pipe {filter_name}"),
        &input,
        output,
    );

    Ok(())
}

/// The set of filter names `rtk pipe` accepts, for help text and errors.
pub const PIPE_FILTERS: &[&str] = &[
    "cargo-test",
    "pytest",
    "tsc",
    "go-test",
    "mypy",
    "ruff-check",
    "ruff-format",
    "log",
];

/// Dispatch stdin content to the named filter. Pure (string in, string out) so it
/// is directly unit-testable; unknown names are a hard error listing the options.
fn apply_named_filter(name: &str, input: &str) -> Result<String> {
    let out = match name {
        "cargo-test" => crate::cargo_cmd::filter_cargo_test(input),
        "pytest" => crate::pytest_cmd::filter_pytest_output(input),
        "tsc" => crate::tsc_cmd::filter_tsc_output(input),
        "go-test" => crate::go_cmd::filter_go_test_json(input),
        "mypy" => crate::mypy_cmd::filter_mypy_output(input),
        "ruff-check" => crate::ruff_cmd::filter_ruff_check_json(input),
        "ruff-format" => crate::ruff_cmd::filter_ruff_format(input),
        "log" => crate::log_cmd::run_stdin_str(input),
        other => bail!(
            "unknown pipe filter '{}'. Available: {}",
            other,
            PIPE_FILTERS.join(", ")
        ),
    };
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_named_filter_unknown_errors() {
        let err = apply_named_filter("nope", "some input").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unknown pipe filter 'nope'"));
        assert!(
            msg.contains("cargo-test"),
            "error should list available filters"
        );
    }

    #[test]
    fn test_apply_named_filter_all_known_names_route() {
        // Every advertised filter name must resolve (empty input is fine).
        for name in PIPE_FILTERS {
            let result = apply_named_filter(name, "");
            assert!(result.is_ok(), "filter '{name}' should be routable");
        }
    }

    #[test]
    fn test_apply_named_filter_log_routes() {
        // Route to the log filter and confirm it produced output from the input.
        // (Size is not asserted here; the runtime never_worse guard in run() caps it.)
        let input = "some log line\nanother line\n";
        let out = apply_named_filter("log", input).unwrap();
        assert!(!out.is_empty(), "log filter should produce output");
    }

    #[test]
    fn test_pipe_filters_list_nonempty_and_unique() {
        assert!(!PIPE_FILTERS.is_empty());
        let mut seen = std::collections::HashSet::new();
        for f in PIPE_FILTERS {
            assert!(seen.insert(*f), "duplicate pipe filter '{f}'");
        }
    }
}
