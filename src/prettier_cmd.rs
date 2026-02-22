use crate::tracking;
use crate::utils::package_manager_exec;
use anyhow::{Context, Result};

pub fn run(args: &[String], verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    let mut cmd = package_manager_exec("prettier");

    // Add user arguments
    for arg in args {
        cmd.arg(arg);
    }

    if verbose > 0 {
        eprintln!("Running: prettier {}", args.join(" "));
    }

    let output = cmd
        .output()
        .context("Failed to run prettier (try: npm install -g prettier)")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let raw = format!("{}\n{}", stdout, stderr);

    let mut filtered = filter_prettier_output(&raw);
    let exit_code = output
        .status
        .code()
        .unwrap_or(if output.status.success() { 0 } else { 1 });
    crate::utils::ensure_failure_visibility(&mut filtered, exit_code, &stderr);

    println!("{}", filtered);

    timer.track(
        &format!("prettier {}", args.join(" ")),
        &format!("rtk prettier {}", args.join(" ")),
        &raw,
        &filtered,
    );

    // Preserve exit code for CI/CD
    if !output.status.success() {
        std::process::exit(output.status.code().unwrap_or(1));
    }

    Ok(())
}

/// Filter Prettier output - show only files that need formatting
/// Fixed: Parse [warn] and [error] prefixes instead of stripping them
pub fn filter_prettier_output(output: &str) -> String {
    let mut files_to_format: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut files_checked = 0;
    let mut is_check_mode = true;

    for line in output.lines() {
        let trimmed = line.trim();

        // Parse [warn] <filepath> lines - these are files needing formatting
        // Only treat as a file if it looks like a file path (contains path separator or extension)
        // This excludes summary lines like "[warn] Code style issues found..."
        if let Some(path) = trimmed.strip_prefix("[warn] ") {
            // Only treat as a file if it looks like a file path:
            // - contains path separator (/ or \)
            // - OR ends with a dot followed by 2-10 alphanumeric chars (file extension)
            let is_file_path = path.contains('/')
                || path.contains("\\")
                || path
                    .rsplit('.')
                    .next()
                    .map(|ext| {
                        ext.len() <= 10
                            && ext.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                    })
                    .unwrap_or(false);
            if is_file_path {
                files_to_format.push(path.to_string());
            }
        }
        // Parse [error] <message> lines - these are syntax errors
        else if let Some(err) = trimmed.strip_prefix("[error] ") {
            errors.push(err.to_string());
        }
        // Detect check mode vs write mode
        else if trimmed.contains("Checking formatting") {
            is_check_mode = true;
        }
        // Count total files checked
        else if trimmed.contains("All matched files use Prettier") {
            if let Some(count_str) = trimmed.split_whitespace().next() {
                if let Ok(count) = count_str.parse::<usize>() {
                    files_checked = count;
                }
            }
        }
    }

    // Only claim success if explicitly confirmed AND no issues found
    // Must have "All matched files use Prettier" AND no files_to_format AND no errors
    if files_to_format.is_empty()
        && errors.is_empty()
        && output.contains("All matched files use Prettier")
    {
        return "✓ Prettier: All files formatted correctly".to_string();
    }

    // Check if files were written (write mode)
    if output.contains("modified") || output.contains("formatted") {
        is_check_mode = false;
    }

    let mut result = String::new();

    // Show errors first (syntax errors are more critical)
    if !errors.is_empty() {
        result.push_str(&format!("Prettier: {} errors\n", errors.len()));
        result.push_str("═══════════════════════════════════════\n");
        for err in errors.iter().take(10) {
            result.push_str(&format!("- {}\n", err));
        }
        if errors.len() > 10 {
            result.push_str(&format!("... +{} more errors\n", errors.len() - 10));
        }
        result.push('\n');
    }

    if is_check_mode {
        // Check mode: show files that need formatting
        // Skip this block if we have errors (already reported above)
        // and files_to_format is empty - don't show misleading "0 files need formatting"
        if !files_to_format.is_empty() {
            result.push_str(&format!(
                "Prettier: {} files need formatting\n",
                files_to_format.len()
            ));
            result.push_str("═══════════════════════════════════════\n");

            for (i, file) in files_to_format.iter().take(10).enumerate() {
                result.push_str(&format!("{}. {}\n", i + 1, file));
            }

            if files_to_format.len() > 10 {
                result.push_str(&format!(
                    "\n... +{} more files\n",
                    files_to_format.len() - 10
                ));
            }

            if files_checked > 0 {
                result.push_str(&format!(
                    "\n✓ {} files already formatted\n",
                    files_checked - files_to_format.len()
                ));
            }
        } else if errors.is_empty() {
            // Only show "all formatted" if there were no errors either
            if output.contains("All matched files use Prettier") {
                result.push_str("✓ Prettier: All files formatted correctly\n");
            }
        }
    } else {
        // Write mode: show what was formatted
        result.push_str(&format!(
            "✓ Prettier: {} files formatted\n",
            files_to_format.len()
        ));
    }

    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_all_formatted() {
        let output = r#"
Checking formatting...
All matched files use Prettier code style!
        "#;
        let result = filter_prettier_output(output);
        assert!(result.contains("✓ Prettier"));
        assert!(result.contains("All files formatted correctly"));
    }

    #[test]
    fn test_filter_files_need_formatting() {
        let output = r#"
Checking formatting...
[warn] src/components/ui/button.tsx
[warn] src/lib/auth/session.ts
[warn] src/pages/dashboard.tsx
Code style issues found in the above file(s). Forgot to run Prettier?
        "#;
        let result = filter_prettier_output(output);
        assert!(result.contains("3 files need formatting"));
        assert!(result.contains("button.tsx"));
        assert!(result.contains("session.ts"));
    }

    #[test]
    fn test_filter_many_files() {
        let mut output = String::from("Checking formatting...\n");
        for i in 0..15 {
            output.push_str(&format!("[warn] src/file{}.ts\n", i));
        }
        let result = filter_prettier_output(&output);
        assert!(result.contains("15 files need formatting"));
        assert!(result.contains("... +5 more files"));
    }

    #[test]
    fn test_filter_warn_files_detected() {
        let output = include_str!("../tests/fixtures/prettier_check_failure.txt");
        let result = filter_prettier_output(output);
        // Must NOT claim success
        assert!(
            !result.contains("✓"),
            "Failure output must not contain success marker"
        );
        assert!(!result.contains("All files formatted correctly"));
        // Must show file count
        assert!(result.contains("4 files need formatting"));
        // Must show all files including non-hardcoded extensions
        assert!(result.contains("Button.tsx"));
        assert!(result.contains("utils.ts"));
        assert!(result.contains("index.vue"));
        assert!(result.contains("config.yaml"));
    }

    #[test]
    fn test_filter_error_lines_detected() {
        let output = include_str!("../tests/fixtures/prettier_syntax_error.txt");
        let result = filter_prettier_output(output);
        // Must NOT claim success
        assert!(
            !result.contains("✓"),
            "Error output must not contain success marker"
        );
        assert!(!result.contains("All files formatted correctly"));
        // Must show errors
        assert!(result.contains("error") || result.contains("Error"));
        assert!(result.contains("broken.ts"));
        // Must also show the warn file
        assert!(result.contains("messy.css"));
    }

    #[test]
    fn test_filter_empty_output_no_false_success() {
        // Empty or unrecognized output should NOT claim success
        let result = filter_prettier_output("");
        assert!(!result.contains("✓ Prettier: All files formatted correctly"));
    }

    #[test]
    fn test_filter_excludes_warn_summary_lines() {
        // Prettier can output "[warn] Code style issues found..." - should not be treated as a file
        let output = r#"
Checking formatting...
[warn] Code style issues found in the above file(s). Forgot to run Prettier?
        "#;
        let result = filter_prettier_output(output);
        // Should not claim success or show file count
        assert!(!result.contains("files need formatting"));
        assert!(!result.contains("All files formatted correctly"));
    }

    #[test]
    fn test_filter_error_only_no_false_zero_count() {
        // Syntax errors only (no [warn] files) should not show "0 files need formatting"
        let output = r#"
Checking formatting...
[error] src/broken.ts: SyntaxError: Unexpected token (3:1)
        "#;
        let result = filter_prettier_output(output);
        // Must show errors
        assert!(result.contains("error") || result.contains("Error"));
        assert!(result.contains("broken.ts"));
        // Must NOT show "0 files need formatting"
        assert!(!result.contains("0 files need formatting"));
    }
}
