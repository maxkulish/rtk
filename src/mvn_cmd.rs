// Filter for Apache Maven (mvn) command output.
//
// NOTE: All filters in this module were validated against realistic fixtures
// embedded in the test module below; Apache Maven is not installed on the
// development machine where this was written.
//
// Filtering goals (target: 80%+ token reduction):
//   - Strip download/progress noise and plugin execution banners ([INFO] ---)
//   - For `mvn test` / `mvn verify`: surface only failing test classes/methods
//     with their error details, plus the aggregate "Tests run:" summary
//   - For `mvn compile` / `mvn install`: surface compile [ERROR] lines plus
//     BUILD SUCCESS / BUILD FAILURE and total time
//   - Always include reactor summary (multi-module builds)

use crate::tracking;
use anyhow::{Context, Result};
use std::process::Command;

/// Parser state for the Maven output state machine.
#[derive(Debug, PartialEq)]
enum ParseState {
    /// Normal scanning - collecting build-level events, skipping download noise
    Scanning,
    /// Inside a Surefire / Failsafe "T E S T S" block
    TestSection,
    /// Inside an individual test method failure block (collecting raw lines)
    MethodFailure,
    /// Inside the [INFO] Reactor Summary section (multi-module builds)
    ReactorSummary,
}

pub fn run(args: &[String], verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    let mut cmd = Command::new("mvn");
    for arg in args {
        cmd.arg(arg);
    }

    if verbose > 0 {
        eprintln!("Running: mvn {}", args.join(" "));
    }

    let output = cmd.output().context(
        "Failed to run mvn. Is Apache Maven installed? \
         See https://maven.apache.org/install.html",
    )?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Maven writes everything to stdout; stderr is JVM errors etc.
    let raw = if stderr.trim().is_empty() {
        stdout.to_string()
    } else {
        format!("{}{}", stdout, stderr)
    };

    let compacted = filter_mvn_output(&stdout);
    let mut filtered = crate::guard::never_worse(&raw, &compacted).to_string();

    let exit_code = output
        .status
        .code()
        .unwrap_or(if output.status.success() { 0 } else { 1 });

    crate::utils::ensure_failure_visibility(&mut filtered, exit_code, &stderr);

    if let Some(hint) = crate::tee::tee_and_hint(&raw, "mvn", exit_code) {
        println!("{}\n{}", filtered, hint);
    } else {
        println!("{}", filtered);
    }

    // Emit stderr (JVM errors, etc.) unchanged
    if !stderr.trim().is_empty() {
        eprintln!("{}", stderr.trim());
    }

    let original_cmd = format!("mvn {}", args.join(" "));
    let rtk_cmd = format!("rtk mvn {}", args.join(" "));
    timer.track(&original_cmd, &rtk_cmd, &raw, &filtered);

    // Preserve exit code for CI/CD pipelines
    if !output.status.success() {
        std::process::exit(exit_code);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns true if this [INFO] line content is pure noise.
///
/// Called only in the `Scanning` state path (most [INFO] lines in other states
/// are handled by explicit state-machine logic before reaching this function).
fn is_info_noise(content: &str) -> bool {
    if content.is_empty() {
        return true;
    }
    // Download / progress lines
    if content.starts_with("Downloading from ")
        || content.starts_with("Downloaded from ")
        || content.starts_with("Progress (")
    {
        return true;
    }
    // Plugin execution banners and separator bars
    if content.starts_with("--- ")
        || content.starts_with("---")
        || content.starts_with("========")
        || content.starts_with("--------")
    {
        return true;
    }
    // Surefire "T E S T S" banner (state transition handled separately)
    if content.trim() == "T E S T S" {
        return true;
    }
    // Maven startup / project metadata
    if content.starts_with("Scanning for projects")
        || content.starts_with("Building ")
        || content.starts_with("Running ")
        || content == "Results:"
        || content.starts_with("Surefire report directory:")
        || content.starts_with("Failsafe report directory:")
        || content.starts_with("skip non existing")
        || content.starts_with("Nothing to compile")
        || content.starts_with("Changes detected")
        || content.starts_with("Compiling ")
        || content.starts_with("Using auto detected provider")
        || content.starts_with("Installing ")
        || content.starts_with("Building jar:")
    {
        return true;
    }
    // Passing per-class test summaries have "Time elapsed:" - noise
    // (failing ones go through [ERROR] and are handled separately)
    if content.starts_with("Tests run:") && content.contains("Time elapsed:") {
        return true;
    }
    false
}

/// Returns true if an [ERROR] line content is low-value noise.
fn is_error_noise(content: &str) -> bool {
    content.is_empty()
        || content.starts_with("-> [Help")
        || content == "------------------------------------------------------------------------"
        // "Failed to execute goal" restates errors already shown inline above the BUILD FAILURE
        // line; filtering it avoids duplicating compile/test error messages in the output.
        || content.starts_with("Failed to execute goal")
        // Maven verbose "how to debug" hints - not useful for LLM context
        || content.starts_with("Please refer to ")
        || content.starts_with("To see the full stack trace")
        || content.starts_with("Re-run Maven")
        || content.starts_with("For more information about the errors")
}

/// Returns true if this is a per-method failure/error line (not the class summary).
///
/// Class-level line (keep separately):
///   `[ERROR] Tests run: 3, ... <<< FAILURE! - in com.example.MyTest`
/// Method-level line (triggers raw-line capture mode):
///   `[ERROR] shouldMultiply  Time elapsed: 0.012 s  <<< FAILURE!`
fn is_method_failure_line(content: &str) -> bool {
    (content.contains("<<< FAILURE!") || content.contains("<<< ERROR!"))
        && !content.starts_with("Tests run:")
}

// ---------------------------------------------------------------------------
// Core filter
// ---------------------------------------------------------------------------

/// Parse Maven output and return a compact summary.
///
/// Designed for 80%+ token reduction over typical Maven verbose output.
/// Validated against realistic inline fixtures (Maven not installed on dev machine).
pub(crate) fn filter_mvn_output(output: &str) -> String {
    let mut state = ParseState::Scanning;
    let mut kept: Vec<String> = Vec::new();
    // How many more raw (non-prefixed exception/location) lines to capture
    let mut method_raw_lines_left: usize = 0;

    for line in output.lines() {
        // ----------------------------------------------------------------
        // [INFO] lines
        // ----------------------------------------------------------------
        let info_content = if line == "[INFO]" {
            Some("")
        } else {
            line.strip_prefix("[INFO] ")
        };

        if let Some(content) = info_content {
            method_raw_lines_left = 0; // any prefixed line ends raw capture

            // High-priority state transitions that apply regardless of state
            match content.trim() {
                "T E S T S" => {
                    state = ParseState::TestSection;
                    continue;
                }
                "BUILD SUCCESS" => {
                    kept.push("✓ BUILD SUCCESS".to_string());
                    state = ParseState::Scanning;
                    continue;
                }
                "BUILD FAILURE" => {
                    kept.push("✗ BUILD FAILURE".to_string());
                    state = ParseState::Scanning;
                    continue;
                }
                _ => {}
            }

            if content.starts_with("Total time:") {
                kept.push(format!(
                    "  Total time: {}",
                    content.trim_start_matches("Total time:").trim()
                ));
                continue;
            }

            if content.starts_with("Reactor Summary") {
                state = ParseState::ReactorSummary;
                kept.push(content.to_string());
                continue;
            }

            // State-specific [INFO] handling
            match state {
                ParseState::ReactorSummary => {
                    // Collect module result lines within the reactor block
                    if content.contains("SUCCESS")
                        || content.contains("FAILURE")
                        || content.contains("SKIPPED")
                    {
                        kept.push(format!("  {}", compact_reactor_line(content)));
                    }
                    // Blank lines and separators within reactor block: skip silently
                }

                ParseState::TestSection | ParseState::MethodFailure => {
                    // Keep the aggregate "Tests run:" line only when it shows failures
                    // or errors. Passing aggregates ("Failures: 0, Errors: 0") are
                    // noise - they appear in multi-module builds for every passing
                    // module and add clutter without signal.
                    // Per-class summaries carry "Time elapsed:" and are already
                    // dropped by is_info_noise before reaching this branch.
                    if content.starts_with("Tests run:")
                        && !content.contains("Time elapsed:")
                        && !content.contains("- in ")
                        && !content.contains("Failures: 0")
                    {
                        state = ParseState::TestSection;
                        kept.push(content.to_string());
                    }
                    // Everything else in the test section (Running X, Results:, etc.)
                    // is noise - skip.
                }

                ParseState::Scanning => {
                    // Only keep non-noise info lines in scanning state.
                    // Most important signals (compile counts) come through here.
                    if !is_info_noise(content) {
                        // "2 errors" / "N warnings" compile summaries
                        if (content.ends_with("errors") || content.ends_with("error"))
                            && content.chars().next().map_or(false, |c| c.is_ascii_digit())
                        {
                            kept.push(format!("[info] {}", content));
                        }
                    }
                }
            }
            continue;
        }

        // ----------------------------------------------------------------
        // [ERROR] lines
        // ----------------------------------------------------------------
        let error_content = if line == "[ERROR]" {
            Some("")
        } else {
            line.strip_prefix("[ERROR] ")
        };

        if let Some(content) = error_content {
            method_raw_lines_left = 0; // prefixed line ends raw capture

            if is_error_noise(content) {
                continue;
            }

            // Safety fallback: some Failsafe configurations use [ERROR] for the banner
            if content.trim() == "T E S T S" {
                state = ParseState::TestSection;
                continue;
            }

            if content == "BUILD SUCCESS" {
                kept.push("✓ BUILD SUCCESS".to_string());
                state = ParseState::Scanning;
                continue;
            }
            if content == "BUILD FAILURE" {
                kept.push("✗ BUILD FAILURE".to_string());
                state = ParseState::Scanning;
                continue;
            }

            // Per-method failure line: triggers raw-line capture for exception detail
            if is_method_failure_line(content) {
                state = ParseState::MethodFailure;
                // Strip the "Time elapsed: X s" suffix to save tokens
                let display = if let Some(idx) = content.find("  Time elapsed:") {
                    content[..idx].trim()
                } else {
                    content.trim()
                };
                kept.push(format!("  ❌ {}", display));
                method_raw_lines_left = 5;
                continue;
            }

            // Class-level failure summary or Results-section aggregate
            if content.starts_with("Tests run:") {
                kept.push(content.to_string());
                continue;
            }

            // Everything else on [ERROR]: compile errors, failure details, etc.
            kept.push(format!("[ERROR] {}", content));
            continue;
        }

        // ----------------------------------------------------------------
        // [WARNING] lines: skip entirely (reduce noise)
        // ----------------------------------------------------------------
        if line.starts_with("[WARNING]") {
            method_raw_lines_left = 0;
            continue;
        }

        // ----------------------------------------------------------------
        // Non-prefixed lines: raw stack trace / exception content
        // ----------------------------------------------------------------
        if method_raw_lines_left > 0 {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                // Skip deep JVM stack frames; keep exception type/location
                if !trimmed.starts_with("at ") && !trimmed.starts_with("... ") {
                    kept.push(format!("    {}", trimmed));
                    method_raw_lines_left = method_raw_lines_left.saturating_sub(1);
                }
            }
        }
    }

    if kept.is_empty() {
        return String::new();
    }

    // Deduplicate while preserving order. Maven's "Failed to execute goal" footer
    // often reiterates the same [ERROR] file:line messages already shown inline
    // during the build. Removing exact duplicates keeps output clean.
    let mut seen = std::collections::HashSet::new();
    kept.retain(|line| seen.insert(line.clone()));

    kept.join("\n")
}

/// Replace long runs of dots in a reactor summary line with a single space.
///
/// Example: `"module-a ........... SUCCESS [0.2 s]"` -> `"module-a SUCCESS [0.2 s]"`
fn compact_reactor_line(line: &str) -> String {
    // Collapse the dotted alignment padding (a run of >=3 dots, e.g.
    // `module-a ....... SUCCESS`) into a single space, absorbing the spaces on
    // either side of the padding. Single dots (version numbers, timings like
    // `0.234 s`) and bracketed timing spacing are preserved.
    let mut result = String::with_capacity(line.len());
    let mut dot_run = 0usize;
    let mut skip_next_space = false;
    for ch in line.chars() {
        if ch == '.' {
            dot_run += 1;
            continue;
        }
        if dot_run >= 3 {
            if result.ends_with(' ') {
                result.pop();
            }
            result.push(' ');
            skip_next_space = true;
        } else {
            for _ in 0..dot_run {
                result.push('.');
            }
        }
        dot_run = 0;
        if ch == ' ' && skip_next_space {
            skip_next_space = false;
            continue;
        }
        skip_next_space = false;
        result.push(ch);
    }
    if dot_run >= 3 {
        if result.ends_with(' ') {
            result.pop();
        }
        result.push(' ');
    } else {
        for _ in 0..dot_run {
            result.push('.');
        }
    }
    result.trim().to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Shared token-count helper: whitespace-delimited words (matches guard::count_tokens)
    fn count_tokens(text: &str) -> usize {
        text.split_whitespace().count()
    }

    // -----------------------------------------------------------------------
    // Fixtures - realistic Maven output.
    // NOTE: Fixtures are inlined; mvn is not installed on this machine.
    // Download noise is included to reflect real build verbosity.
    // -----------------------------------------------------------------------

    /// `mvn test` with 1 failing and 2 passing tests.
    const MVN_TEST_FAILURE: &str = "\
[INFO] Scanning for projects...
[INFO]
[INFO] ----------------------< com.example:myapp >-----------------------
[INFO] Building myapp 1.0.0-SNAPSHOT
[INFO] --------------------------------[ jar ]---------------------------------
[INFO]
[INFO] Downloading from central: https://repo.maven.apache.org/maven2/org/apache/maven/plugins/maven-surefire-plugin/3.2.3/maven-surefire-plugin-3.2.3.pom
[INFO] Downloaded from central: https://repo.maven.apache.org/maven2/org/apache/maven/plugins/maven-surefire-plugin/3.2.3/maven-surefire-plugin-3.2.3.pom (3 kB at 450 kB/s)
[INFO] Downloading from central: https://repo.maven.apache.org/maven2/junit/junit/4.13.2/junit-4.13.2.pom
[INFO] Downloaded from central: https://repo.maven.apache.org/maven2/junit/junit/4.13.2/junit-4.13.2.pom (2 kB at 380 kB/s)
[INFO] Downloading from central: https://repo.maven.apache.org/maven2/junit/junit/4.13.2/junit-4.13.2.jar
[INFO] Downloaded from central: https://repo.maven.apache.org/maven2/junit/junit/4.13.2/junit-4.13.2.jar (378 kB at 1.2 MB/s)
[INFO]
[INFO] --- maven-resources-plugin:3.3.1:resources (default-resources) @ myapp ---
[INFO] skip non existing resourceDirectory /home/user/myapp/src/main/resources
[INFO]
[INFO] --- maven-compiler-plugin:3.11.0:compile (default-compile) @ myapp ---
[INFO] Nothing to compile - all classes are up to date.
[INFO]
[INFO] --- maven-resources-plugin:3.3.1:testResources (default-testResources) @ myapp ---
[INFO] skip non existing resourceDirectory /home/user/myapp/src/test/resources
[INFO]
[INFO] --- maven-compiler-plugin:3.11.0:testCompile (default-testCompile) @ myapp ---
[INFO] Nothing to compile - all classes are up to date.
[INFO]
[INFO] --- maven-surefire-plugin:3.2.3:test (default-test) @ myapp ---
[INFO] Using auto detected provider org.apache.maven.surefire.junit4.JUnit4Provider
[INFO]
[INFO] -------------------------------------------------------
[INFO]  T E S T S
[INFO] -------------------------------------------------------
[INFO] Running com.example.CalculatorTest
[ERROR] Tests run: 3, Failures: 1, Errors: 0, Skipped: 0, Time elapsed: 0.234 s <<< FAILURE! - in com.example.CalculatorTest
[ERROR] shouldMultiply  Time elapsed: 0.012 s  <<< FAILURE!
com.example.CalculatorTest.shouldMultiply:45
java.lang.AssertionError: expected:<10> but was:<15>
\tat org.junit.Assert.fail(Assert.java:89)
\tat org.junit.Assert.failNotEquals(Assert.java:835)
\tat org.junit.Assert.assertEquals(Assert.java:120)
\tat com.example.CalculatorTest.shouldMultiply(CalculatorTest.java:45)
[INFO] Running com.example.StringUtilTest
[INFO] Tests run: 5, Failures: 0, Errors: 0, Skipped: 0, Time elapsed: 0.045 s
[INFO] Running com.example.UserServiceTest
[INFO] Tests run: 4, Failures: 0, Errors: 0, Skipped: 0, Time elapsed: 0.078 s
[INFO]
[INFO] Results:
[INFO]
[ERROR] Failures:
[ERROR]   com.example.CalculatorTest.shouldMultiply:45
[ERROR]     AssertionError: expected:<10> but was:<15>
[INFO]
[ERROR] Tests run: 12, Failures: 1, Errors: 0, Skipped: 0
[INFO]
[INFO] ------------------------------------------------------------------------
[INFO] BUILD FAILURE
[INFO] ------------------------------------------------------------------------
[INFO] Total time:  2.345 s
[INFO] Finished at: 2024-01-15T10:30:00Z
[INFO] ------------------------------------------------------------------------
[ERROR] Failed to execute goal org.apache.maven.plugins:maven-surefire-plugin:3.2.3:test (default-test) on project myapp: There are test failures.
[ERROR]
[ERROR] Please refer to /home/user/myapp/target/surefire-reports for the individual test results.
[ERROR] Please refer to dump files (if any exist) [date].dump, [date]-jvmRun[N].dump and [date].dumpstream.
[ERROR] -> [Help 1]
[ERROR]
[ERROR] To see the full stack trace of the errors, re-run Maven with the -e switch.
[ERROR] Re-run Maven using the -X switch to enable full debug logging.
[ERROR]
[ERROR] For more information about the errors and possible solutions, please read the following articles:
[ERROR] -> [Help 1]
";

    /// `mvn install` - all tests pass.
    const MVN_INSTALL_SUCCESS: &str = "\
[INFO] Scanning for projects...
[INFO]
[INFO] ----------------------< com.example:myapp >-----------------------
[INFO] Building myapp 1.0.0-SNAPSHOT
[INFO] --------------------------------[ jar ]---------------------------------
[INFO]
[INFO] Downloading from central: https://repo.maven.apache.org/maven2/org/apache/maven/plugins/maven-compiler-plugin/3.11.0/maven-compiler-plugin-3.11.0.pom
[INFO] Downloaded from central: https://repo.maven.apache.org/maven2/org/apache/maven/plugins/maven-compiler-plugin/3.11.0/maven-compiler-plugin-3.11.0.pom (12 kB at 560 kB/s)
[INFO] Downloading from central: https://repo.maven.apache.org/maven2/org/apache/maven/plugins/maven-jar-plugin/3.3.0/maven-jar-plugin-3.3.0.pom
[INFO] Downloaded from central: https://repo.maven.apache.org/maven2/org/apache/maven/plugins/maven-jar-plugin/3.3.0/maven-jar-plugin-3.3.0.pom (8 kB at 620 kB/s)
[INFO] Downloading from central: https://repo.maven.apache.org/maven2/org/apache/maven/plugins/maven-install-plugin/3.1.1/maven-install-plugin-3.1.1.pom
[INFO] Downloaded from central: https://repo.maven.apache.org/maven2/org/apache/maven/plugins/maven-install-plugin/3.1.1/maven-install-plugin-3.1.1.pom (6 kB at 490 kB/s)
[INFO]
[INFO] --- maven-resources-plugin:3.3.1:resources (default-resources) @ myapp ---
[INFO] skip non existing resourceDirectory /home/user/myapp/src/main/resources
[INFO]
[INFO] --- maven-compiler-plugin:3.11.0:compile (default-compile) @ myapp ---
[INFO] Changes detected - recompiling the module! :source
[INFO] Compiling 3 source files with javac [debug release 17] to target/classes
[INFO]
[INFO] --- maven-resources-plugin:3.3.1:testResources (default-testResources) @ myapp ---
[INFO] skip non existing resourceDirectory /home/user/myapp/src/test/resources
[INFO]
[INFO] --- maven-compiler-plugin:3.11.0:testCompile (default-testCompile) @ myapp ---
[INFO] Changes detected - recompiling the module! :source
[INFO] Compiling 3 source files with javac [debug release 17] to target/test-classes
[INFO]
[INFO] --- maven-surefire-plugin:3.2.3:test (default-test) @ myapp ---
[INFO] Using auto detected provider org.apache.maven.surefire.junit4.JUnit4Provider
[INFO]
[INFO] -------------------------------------------------------
[INFO]  T E S T S
[INFO] -------------------------------------------------------
[INFO] Running com.example.CalculatorTest
[INFO] Tests run: 3, Failures: 0, Errors: 0, Skipped: 0, Time elapsed: 0.234 s
[INFO] Running com.example.StringUtilTest
[INFO] Tests run: 5, Failures: 0, Errors: 0, Skipped: 0, Time elapsed: 0.045 s
[INFO] Running com.example.UserServiceTest
[INFO] Tests run: 4, Failures: 0, Errors: 0, Skipped: 0, Time elapsed: 0.078 s
[INFO]
[INFO] Results:
[INFO]
[INFO] Tests run: 12, Failures: 0, Errors: 0, Skipped: 0
[INFO]
[INFO] --- maven-jar-plugin:3.3.0:jar (default-jar) @ myapp ---
[INFO] Building jar: /home/user/myapp/target/myapp-1.0.0-SNAPSHOT.jar
[INFO]
[INFO] --- maven-install-plugin:3.1.1:install (default-install) @ myapp ---
[INFO] Installing /home/user/myapp/target/myapp-1.0.0-SNAPSHOT.jar to /home/user/.m2/repository/com/example/myapp/1.0.0-SNAPSHOT/myapp-1.0.0-SNAPSHOT.jar
[INFO] Installing /home/user/myapp/pom.xml to /home/user/.m2/repository/com/example/myapp/1.0.0-SNAPSHOT/myapp-1.0.0-SNAPSHOT.pom
[INFO] ------------------------------------------------------------------------
[INFO] BUILD SUCCESS
[INFO] ------------------------------------------------------------------------
[INFO] Total time:  3.456 s
[INFO] Finished at: 2024-01-15T10:31:00Z
[INFO] ------------------------------------------------------------------------
";

    /// `mvn compile` - compilation errors.
    const MVN_COMPILE_ERROR: &str = "\
[INFO] Scanning for projects...
[INFO]
[INFO] ----------------------< com.example:myapp >-----------------------
[INFO] Building myapp 1.0.0-SNAPSHOT
[INFO] --------------------------------[ jar ]---------------------------------
[INFO]
[INFO] Downloading from central: https://repo.maven.apache.org/maven2/org/apache/maven/plugins/maven-compiler-plugin/3.11.0/maven-compiler-plugin-3.11.0.pom
[INFO] Downloaded from central: https://repo.maven.apache.org/maven2/org/apache/maven/plugins/maven-compiler-plugin/3.11.0/maven-compiler-plugin-3.11.0.pom (12 kB at 560 kB/s)
[INFO] Downloading from central: https://repo.maven.apache.org/maven2/org/codehaus/plexus/plexus-compiler-javac/2.13.0/plexus-compiler-javac-2.13.0.pom
[INFO] Downloaded from central: https://repo.maven.apache.org/maven2/org/codehaus/plexus/plexus-compiler-javac/2.13.0/plexus-compiler-javac-2.13.0.pom (5 kB at 340 kB/s)
[INFO] Downloading from central: https://repo.maven.apache.org/maven2/org/codehaus/plexus/plexus-compiler-javac/2.13.0/plexus-compiler-javac-2.13.0.jar
[INFO] Downloaded from central: https://repo.maven.apache.org/maven2/org/codehaus/plexus/plexus-compiler-javac/2.13.0/plexus-compiler-javac-2.13.0.jar (58 kB at 870 kB/s)
[INFO]
[INFO] --- maven-resources-plugin:3.3.1:resources (default-resources) @ myapp ---
[INFO] skip non existing resourceDirectory /home/user/myapp/src/main/resources
[INFO]
[INFO] --- maven-compiler-plugin:3.11.0:compile (default-compile) @ myapp ---
[INFO] Changes detected - recompiling the module! :source
[INFO] Compiling 3 source files with javac [debug release 17] to target/classes
[ERROR] COMPILATION ERROR :
[INFO] -------------------------------------------------------------
[ERROR] /home/user/myapp/src/main/java/com/example/Calculator.java:[42,17] ';' expected
[ERROR] /home/user/myapp/src/main/java/com/example/Calculator.java:[45,5] reached end of file while parsing
[INFO] 2 errors
[INFO] -------------------------------------------------------------
[INFO] ------------------------------------------------------------------------
[INFO] BUILD FAILURE
[INFO] ------------------------------------------------------------------------
[INFO] Total time:  1.234 s
[INFO] Finished at: 2024-01-15T10:29:00Z
[INFO] ------------------------------------------------------------------------
[ERROR] Failed to execute goal org.apache.maven.plugins:maven-compiler-plugin:3.11.0:compile (default-compile) on project myapp: Compilation failure: Compilation failure:
[ERROR] /home/user/myapp/src/main/java/com/example/Calculator.java:[42,17] ';' expected
[ERROR] /home/user/myapp/src/main/java/com/example/Calculator.java:[45,5] reached end of file while parsing
[ERROR] -> [Help 1]
[ERROR]
[ERROR] To see the full stack trace of the errors, re-run Maven with the -e switch.
[ERROR] Re-run Maven using the -X switch to enable full debug logging.
[ERROR]
[ERROR] For more information about the errors and possible solutions, please read the following articles:
[ERROR] -> [Help 1]
";

    /// Multi-module `mvn install` with one failing module.
    const MVN_MULTIMODULE_FAILURE: &str = "\
[INFO] Scanning for projects...
[INFO]
[INFO] ------------------< com.example:myapp-parent >------------------
[INFO] Building myapp-parent 1.0.0-SNAPSHOT                          [1/3]
[INFO] --------------------------------[ pom ]---------------------------------
[INFO]
[INFO] Downloading from central: https://repo.maven.apache.org/maven2/org/apache/maven/plugins/maven-surefire-plugin/3.2.3/maven-surefire-plugin-3.2.3.pom
[INFO] Downloaded from central: https://repo.maven.apache.org/maven2/org/apache/maven/plugins/maven-surefire-plugin/3.2.3/maven-surefire-plugin-3.2.3.pom (3 kB at 450 kB/s)
[INFO] Downloading from central: https://repo.maven.apache.org/maven2/junit/junit/4.13.2/junit-4.13.2.pom
[INFO] Downloaded from central: https://repo.maven.apache.org/maven2/junit/junit/4.13.2/junit-4.13.2.pom (2 kB at 380 kB/s)
[INFO] Downloading from central: https://repo.maven.apache.org/maven2/junit/junit/4.13.2/junit-4.13.2.jar
[INFO] Downloaded from central: https://repo.maven.apache.org/maven2/junit/junit/4.13.2/junit-4.13.2.jar (378 kB at 1.2 MB/s)
[INFO] Downloading from central: https://repo.maven.apache.org/maven2/org/apache/maven/plugins/maven-compiler-plugin/3.11.0/maven-compiler-plugin-3.11.0.pom
[INFO] Downloaded from central: https://repo.maven.apache.org/maven2/org/apache/maven/plugins/maven-compiler-plugin/3.11.0/maven-compiler-plugin-3.11.0.pom (12 kB at 560 kB/s)
[INFO] Downloading from central: https://repo.maven.apache.org/maven2/org/apache/maven/plugins/maven-jar-plugin/3.3.0/maven-jar-plugin-3.3.0.pom
[INFO] Downloaded from central: https://repo.maven.apache.org/maven2/org/apache/maven/plugins/maven-jar-plugin/3.3.0/maven-jar-plugin-3.3.0.pom (8 kB at 620 kB/s)
[INFO] Downloading from central: https://repo.maven.apache.org/maven2/org/apache/maven/plugins/maven-install-plugin/3.1.1/maven-install-plugin-3.1.1.pom
[INFO] Downloaded from central: https://repo.maven.apache.org/maven2/org/apache/maven/plugins/maven-install-plugin/3.1.1/maven-install-plugin-3.1.1.pom (6 kB at 490 kB/s)
[INFO] Downloading from central: https://repo.maven.apache.org/maven2/org/codehaus/plexus/plexus-utils/3.5.1/plexus-utils-3.5.1.pom
[INFO] Downloaded from central: https://repo.maven.apache.org/maven2/org/codehaus/plexus/plexus-utils/3.5.1/plexus-utils-3.5.1.pom (4 kB at 310 kB/s)
[INFO] Downloading from central: https://repo.maven.apache.org/maven2/org/codehaus/plexus/plexus-utils/3.5.1/plexus-utils-3.5.1.jar
[INFO] Downloaded from central: https://repo.maven.apache.org/maven2/org/codehaus/plexus/plexus-utils/3.5.1/plexus-utils-3.5.1.jar (264 kB at 1.8 MB/s)
[INFO] Downloading from central: https://repo.maven.apache.org/maven2/org/apache/maven/surefire/surefire-junit4/3.2.3/surefire-junit4-3.2.3.jar
[INFO] Downloaded from central: https://repo.maven.apache.org/maven2/org/apache/maven/surefire/surefire-junit4/3.2.3/surefire-junit4-3.2.3.jar (95 kB at 2.3 MB/s)
[INFO] Downloading from central: https://repo.maven.apache.org/maven2/org/apache/maven/surefire/common-java5/3.2.3/common-java5-3.2.3.jar
[INFO] Downloaded from central: https://repo.maven.apache.org/maven2/org/apache/maven/surefire/common-java5/3.2.3/common-java5-3.2.3.jar (18 kB at 650 kB/s)
[INFO]
[INFO] ------------------< com.example:module-core >------------------
[INFO] Building module-core 1.0.0-SNAPSHOT                           [2/3]
[INFO] --------------------------------[ jar ]---------------------------------
[INFO]
[INFO] --- maven-surefire-plugin:3.2.3:test (default-test) @ module-core ---
[INFO] -------------------------------------------------------
[INFO]  T E S T S
[INFO] -------------------------------------------------------
[INFO] Running com.example.core.CoreServiceTest
[INFO] Tests run: 8, Failures: 0, Errors: 0, Skipped: 0, Time elapsed: 0.312 s
[INFO]
[INFO] Results:
[INFO] Tests run: 8, Failures: 0, Errors: 0, Skipped: 0
[INFO]
[INFO] ------------------< com.example:module-web >-------------------
[INFO] Building module-web 1.0.0-SNAPSHOT                            [3/3]
[INFO] --------------------------------[ jar ]---------------------------------
[INFO]
[INFO] --- maven-surefire-plugin:3.2.3:test (default-test) @ module-web ---
[INFO] -------------------------------------------------------
[INFO]  T E S T S
[INFO] -------------------------------------------------------
[INFO] Running com.example.web.ControllerTest
[ERROR] Tests run: 2, Failures: 1, Errors: 0, Skipped: 0, Time elapsed: 0.198 s <<< FAILURE! - in com.example.web.ControllerTest
[ERROR] testGetUser  Time elapsed: 0.034 s  <<< FAILURE!
com.example.web.ControllerTest.testGetUser:88
java.lang.NullPointerException: user service returned null
\tat com.example.web.ControllerTest.testGetUser(ControllerTest.java:88)
[INFO]
[INFO] Results:
[INFO]
[ERROR] Failures:
[ERROR]   com.example.web.ControllerTest.testGetUser:88
[ERROR]     NullPointerException: user service returned null
[INFO]
[ERROR] Tests run: 2, Failures: 1, Errors: 0, Skipped: 0
[INFO]
[INFO] Reactor Summary for myapp-parent 1.0.0-SNAPSHOT:
[INFO]
[INFO] myapp-parent ....................................... SUCCESS [  0.045 s]
[INFO] module-core ........................................ SUCCESS [  1.234 s]
[INFO] module-web ......................................... FAILURE [  0.987 s]
[INFO]
[INFO] ------------------------------------------------------------------------
[INFO] BUILD FAILURE
[INFO] ------------------------------------------------------------------------
[INFO] Total time:  3.456 s
[INFO] Finished at: 2024-01-15T10:32:00Z
[INFO] ------------------------------------------------------------------------
[ERROR] Failed to execute goal org.apache.maven.plugins:maven-surefire-plugin:3.2.3:test (default-test) on project module-web: There are test failures.
[ERROR] -> [Help 1]
[ERROR]
[ERROR] To see the full stack trace of the errors, re-run Maven with the -e switch.
[ERROR] Re-run Maven using the -X switch to enable full debug logging.
[ERROR]
[ERROR] For more information about the errors and possible solutions, please read the following articles:
[ERROR] -> [Help 1]
";

    // -----------------------------------------------------------------------
    // Parser tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_failing_test_shows_class_and_method() {
        let result = filter_mvn_output(MVN_TEST_FAILURE);
        assert!(
            result.contains("CalculatorTest"),
            "Expected CalculatorTest in output, got:\n{result}"
        );
        assert!(
            result.contains("shouldMultiply"),
            "Expected shouldMultiply in output, got:\n{result}"
        );
    }

    #[test]
    fn test_failing_test_shows_error_detail() {
        let result = filter_mvn_output(MVN_TEST_FAILURE);
        // Assertion message from raw (non-prefixed) line
        assert!(
            result.contains("expected:<10>") || result.contains("AssertionError"),
            "Expected assertion error in output, got:\n{result}"
        );
    }

    #[test]
    fn test_failing_test_suppresses_passing_class_noise() {
        let result = filter_mvn_output(MVN_TEST_FAILURE);
        assert!(
            !result.contains("StringUtilTest"),
            "StringUtilTest (passing) should not appear:\n{result}"
        );
        assert!(
            !result.contains("UserServiceTest"),
            "UserServiceTest (passing) should not appear:\n{result}"
        );
    }

    #[test]
    fn test_failing_test_shows_aggregate_summary() {
        let result = filter_mvn_output(MVN_TEST_FAILURE);
        assert!(
            result.contains("Tests run: 12"),
            "Expected aggregate test summary in output, got:\n{result}"
        );
    }

    #[test]
    fn test_failing_test_shows_build_failure_and_time() {
        let result = filter_mvn_output(MVN_TEST_FAILURE);
        assert!(
            result.contains("BUILD FAILURE"),
            "Expected BUILD FAILURE:\n{result}"
        );
        assert!(
            result.contains("Total time:"),
            "Expected Total time:\n{result}"
        );
    }

    #[test]
    fn test_install_success_shows_build_success() {
        let result = filter_mvn_output(MVN_INSTALL_SUCCESS);
        assert!(
            result.contains("BUILD SUCCESS"),
            "Expected BUILD SUCCESS:\n{result}"
        );
        assert!(
            result.contains("Total time:"),
            "Expected Total time:\n{result}"
        );
    }

    #[test]
    fn test_install_success_suppresses_noise() {
        let result = filter_mvn_output(MVN_INSTALL_SUCCESS);
        assert!(
            !result.contains("CalculatorTest"),
            "Passing class should be stripped:\n{result}"
        );
        assert!(
            !result.contains("Scanning for projects"),
            "Noise should be stripped:\n{result}"
        );
        assert!(
            !result.contains("Downloading from"),
            "Download noise should be stripped:\n{result}"
        );
    }

    #[test]
    fn test_compile_error_shows_file_and_message() {
        let result = filter_mvn_output(MVN_COMPILE_ERROR);
        assert!(
            result.contains("Calculator.java"),
            "Expected source file path in output, got:\n{result}"
        );
        assert!(
            result.contains("';' expected") || result.contains("expected"),
            "Expected compile error message in output, got:\n{result}"
        );
    }

    #[test]
    fn test_compile_error_shows_build_failure() {
        let result = filter_mvn_output(MVN_COMPILE_ERROR);
        assert!(
            result.contains("BUILD FAILURE"),
            "Expected BUILD FAILURE:\n{result}"
        );
    }

    #[test]
    fn test_compile_error_suppresses_noise() {
        let result = filter_mvn_output(MVN_COMPILE_ERROR);
        assert!(
            !result.contains("Scanning for projects"),
            "Noise should be stripped:\n{result}"
        );
        assert!(
            !result.contains("-> [Help 1]"),
            "Help link noise should be stripped:\n{result}"
        );
        assert!(
            !result.contains("To see the full stack trace"),
            "Verbose hint should be stripped:\n{result}"
        );
    }

    #[test]
    fn test_multimodule_shows_reactor_summary() {
        let result = filter_mvn_output(MVN_MULTIMODULE_FAILURE);
        assert!(
            result.contains("Reactor Summary"),
            "Expected Reactor Summary in output, got:\n{result}"
        );
        assert!(
            result.contains("module-web"),
            "Expected failing module in reactor summary:\n{result}"
        );
        assert!(
            result.contains("FAILURE"),
            "Expected FAILURE status:\n{result}"
        );
    }

    #[test]
    fn test_multimodule_shows_failing_test() {
        let result = filter_mvn_output(MVN_MULTIMODULE_FAILURE);
        assert!(
            result.contains("testGetUser"),
            "Expected failing test method in output, got:\n{result}"
        );
        assert!(
            result.contains("NullPointerException") || result.contains("null"),
            "Expected exception detail in output, got:\n{result}"
        );
    }

    // -----------------------------------------------------------------------
    // Token-savings tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_token_savings_failing_test() {
        let input_tokens = count_tokens(MVN_TEST_FAILURE);
        let filtered = filter_mvn_output(MVN_TEST_FAILURE);
        let output_tokens = count_tokens(&filtered);
        let savings_pct = 100.0 - (output_tokens as f64 / input_tokens as f64 * 100.0);
        assert!(
            savings_pct >= 80.0,
            "Expected >=80% savings on test failure, got {savings_pct:.1}% \
             ({input_tokens} -> {output_tokens} tokens)\nFiltered:\n{filtered}"
        );
    }

    #[test]
    fn test_token_savings_install_success() {
        let input_tokens = count_tokens(MVN_INSTALL_SUCCESS);
        let filtered = filter_mvn_output(MVN_INSTALL_SUCCESS);
        let output_tokens = count_tokens(&filtered);
        let savings_pct = 100.0 - (output_tokens as f64 / input_tokens as f64 * 100.0);
        assert!(
            savings_pct >= 80.0,
            "Expected >=80% savings on install success, got {savings_pct:.1}% \
             ({input_tokens} -> {output_tokens} tokens)\nFiltered:\n{filtered}"
        );
    }

    #[test]
    fn test_token_savings_compile_error() {
        let input_tokens = count_tokens(MVN_COMPILE_ERROR);
        let filtered = filter_mvn_output(MVN_COMPILE_ERROR);
        let output_tokens = count_tokens(&filtered);
        let savings_pct = 100.0 - (output_tokens as f64 / input_tokens as f64 * 100.0);
        assert!(
            savings_pct >= 80.0,
            "Expected >=80% savings on compile error, got {savings_pct:.1}% \
             ({input_tokens} -> {output_tokens} tokens)\nFiltered:\n{filtered}"
        );
    }

    #[test]
    fn test_token_savings_multimodule() {
        let input_tokens = count_tokens(MVN_MULTIMODULE_FAILURE);
        let filtered = filter_mvn_output(MVN_MULTIMODULE_FAILURE);
        let output_tokens = count_tokens(&filtered);
        let savings_pct = 100.0 - (output_tokens as f64 / input_tokens as f64 * 100.0);
        assert!(
            savings_pct >= 80.0,
            "Expected >=80% savings on multimodule, got {savings_pct:.1}% \
             ({input_tokens} -> {output_tokens} tokens)\nFiltered:\n{filtered}"
        );
    }

    // -----------------------------------------------------------------------
    // Edge-case tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_empty_output_returns_empty() {
        assert_eq!(filter_mvn_output(""), "");
    }

    #[test]
    fn test_no_tests_build_success() {
        let input = "\
[INFO] Scanning for projects...
[INFO]
[INFO] --- maven-compiler-plugin:3.11.0:compile (default-compile) @ myapp ---
[INFO] Nothing to compile - all classes are up to date.
[INFO]
[INFO] --- maven-surefire-plugin:3.2.3:test (default-test) @ myapp ---
[INFO] No tests to run.
[INFO]
[INFO] BUILD SUCCESS
[INFO] ------------------------------------------------------------------------
[INFO] Total time:  0.567 s
[INFO] Finished at: 2024-01-15T10:33:00Z
[INFO] ------------------------------------------------------------------------
";
        let result = filter_mvn_output(input);
        assert!(
            result.contains("BUILD SUCCESS"),
            "BUILD SUCCESS expected:\n{result}"
        );
        assert!(
            result.contains("Total time:"),
            "Total time expected:\n{result}"
        );
        assert!(
            !result.contains("Scanning for projects"),
            "Noise should be stripped:\n{result}"
        );
    }

    #[test]
    fn test_build_success_with_warnings() {
        let input = "\
[INFO] Scanning for projects...
[WARNING] The POM for foo:bar:jar:1.0 is missing, no dependency information available
[WARNING] 'build.plugins.plugin.version' for org.apache.maven.plugins:maven-compiler-plugin is missing.
[INFO]
[INFO] BUILD SUCCESS
[INFO] ------------------------------------------------------------------------
[INFO] Total time:  1.234 s
[INFO] Finished at: 2024-01-15T10:34:00Z
[INFO] ------------------------------------------------------------------------
";
        let result = filter_mvn_output(input);
        // Warnings are skipped
        assert!(
            !result.contains("[WARNING]"),
            "Warnings should be stripped:\n{result}"
        );
        assert!(
            result.contains("BUILD SUCCESS"),
            "BUILD SUCCESS expected:\n{result}"
        );
    }

    #[test]
    fn test_compact_reactor_line_strips_dots() {
        assert_eq!(
            compact_reactor_line(
                "module-a .......................................... SUCCESS [  0.234 s]"
            ),
            "module-a SUCCESS [  0.234 s]"
        );
        assert_eq!(
            compact_reactor_line("module-b FAILURE [  1.234 s]"),
            "module-b FAILURE [  1.234 s]"
        );
        // Two dots kept (below threshold of 3)
        assert_eq!(compact_reactor_line("a..b"), "a..b");
    }

    #[test]
    fn test_is_info_noise_rejects_downloads() {
        assert!(is_info_noise(
            "Downloading from central: https://repo.maven.apache.org/maven2/junit/junit/4.13.2/junit-4.13.2.jar"
        ));
        assert!(is_info_noise(
            "Downloaded from central: https://repo.maven.apache.org/maven2/junit/junit/4.13.2/junit-4.13.2.jar (378 kB at 1.2 MB/s)"
        ));
    }

    #[test]
    fn test_is_info_noise_rejects_passing_class_summary() {
        // Has "Time elapsed:" -> per-class summary -> noise
        assert!(is_info_noise(
            "Tests run: 3, Failures: 0, Errors: 0, Skipped: 0, Time elapsed: 0.234 s"
        ));
        // No "Time elapsed:" -> aggregate -> NOT noise
        assert!(!is_info_noise(
            "Tests run: 12, Failures: 0, Errors: 0, Skipped: 0"
        ));
    }

    #[test]
    fn test_is_error_noise_filters_hints() {
        assert!(is_error_noise("-> [Help 1]"));
        assert!(is_error_noise(
            "Please refer to /tmp/surefire-reports for test results."
        ));
        assert!(is_error_noise(
            "To see the full stack trace of the errors, re-run Maven with the -e switch."
        ));
        assert!(is_error_noise(
            "Re-run Maven using the -X switch to enable full debug logging."
        ));
        assert!(is_error_noise(
            "For more information about the errors and possible solutions, please read the following articles:"
        ));
        // "Failed to execute goal" footer is a verbose restatement of errors already shown
        assert!(is_error_noise(
            "Failed to execute goal org.apache.maven.plugins:maven-surefire-plugin:3.2.3:test \
             (default-test) on project myapp: There are test failures."
        ));
        assert!(is_error_noise(""));
        // Real errors are kept
        assert!(!is_error_noise("/home/user/File.java:[42,17] ';' expected"));
        assert!(!is_error_noise(
            "Tests run: 12, Failures: 1, Errors: 0, Skipped: 0"
        ));
    }

    #[test]
    fn test_is_method_failure_line_predicate() {
        assert!(is_method_failure_line(
            "shouldMultiply  Time elapsed: 0.012 s  <<< FAILURE!"
        ));
        assert!(is_method_failure_line(
            "testThing  Time elapsed: 0.001 s  <<< ERROR!"
        ));
        // Class-level summary is NOT a method failure line
        assert!(!is_method_failure_line(
            "Tests run: 2, Failures: 1, Errors: 0, Skipped: 0, Time elapsed: 0.234 s <<< FAILURE! - in com.example.Foo"
        ));
    }
}
