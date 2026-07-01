// Filter for Terragrunt (and the underlying Terraform / OpenTofu) command output.
//
// NOTE: The fixtures used to validate this filter were captured from a real
// Terragrunt 1.0.8 run against a cloud-free `null_resource` project (see
// tests/fixtures/terragrunt_*.txt for the real plan/error captures). The
// apply/no-change/update fixtures are synthesized from the same confirmed line
// format because running `terragrunt apply` is gated by the safety rules.
//
// Real Terragrunt output is heavily ANSI-colored and every line is wrapped in a
// log prefix:
//
//     <ts> STDOUT terraform: <plan body>       # forwarded terraform stdout
//     <ts> INFO   terraform: Initializing ...   # terragrunt/terraform progress
//     <ts> STDERR terraform: │ Error: ...       # terraform error box
//     <ts> ERROR  error occurred: ...           # terragrunt-level failure
//
// Older Terragrunt used `time=... level=... msg=...`; both formats are handled.
//
// Filtering goals (target: 80%+ token reduction on plan/apply):
//   - Strip ANSI, then the terragrunt log prefix, to expose the real content
//   - Drop INFO/DEBUG/TRACE progress (downloads, provider install, init, refresh)
//   - Drop the per-resource attribute dumps in a plan (dozens of `+`/`-` lines) -
//     the `# ... will be ...` header already summarizes the action
//   - KEEP: resource action headers (`# X will be ...`), in-place attribute
//     changes (`~ attr = old -> new`), the `Plan:` / `Apply complete!` /
//     `No changes` summaries, `Changes to Outputs` and post-apply `Outputs`,
//     and all error/warning content (incl. the `╷ │ ╵` box framing)

use crate::tracking;
use anyhow::{Context, Result};
use lazy_static::lazy_static;
use regex::Regex;
use std::process::Command;

lazy_static! {
    /// Modern Terragrunt log prefix: `11:25:57.495 STDOUT ...` / `11:25:39.603 ERROR ...`.
    static ref TG_LOG_PREFIX: Regex =
        Regex::new(r"^\d{2}:\d{2}:\d{2}\.\d{3}\s+(\w+)\s*(.*)$").unwrap();
    /// Legacy Terragrunt log prefix: `time=... level=info msg=...`.
    static ref TG_OLD_PREFIX: Regex = Regex::new(r"^time=\S+\s+level=(\w+)\s+msg=(.*)$").unwrap();
}

/// How a (prefix-stripped) line should be treated.
#[derive(Debug, PartialEq)]
enum LineClass {
    /// Progress noise (INFO/DEBUG/TRACE) - always dropped.
    Noise,
    /// Error/warning content - always kept.
    Error,
    /// Terraform stdout / unprefixed content - runs through the signal filter.
    Body,
}

/// Parser state for the Terraform plan/apply body.
#[derive(Debug, PartialEq)]
enum ParseState {
    /// Normal scanning - plan headers, summaries.
    Scanning,
    /// Inside the `Changes to Outputs:` block of a plan.
    OutputChanges,
    /// Inside the post-apply `Outputs:` block.
    Outputs,
}

pub fn run(args: &[String], verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    let mut cmd = Command::new("terragrunt");
    for arg in args {
        cmd.arg(arg);
    }

    if verbose > 0 {
        eprintln!("Running: terragrunt {}", args.join(" "));
    }

    let output = cmd.output().context(
        "Failed to run terragrunt. Is Terragrunt installed? \
         See https://terragrunt.gruntwork.io/docs/getting-started/install/",
    )?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Terragrunt forwards the terraform plan/apply body on stdout and writes its
    // own logs + error boxes on stderr; filter the combined stream so errors
    // survive. Global dedup in the filter absorbs any cross-stream repetition.
    let raw = if stderr.trim().is_empty() {
        stdout.to_string()
    } else {
        format!("{}{}", stdout, stderr)
    };

    let compacted = filter_terragrunt_output(&raw);
    let mut filtered = crate::guard::never_worse(&raw, &compacted).to_string();

    // Safety net: never swallow a command's entire output. If the filter matched
    // nothing (unrecognized format) but the command produced output, fall back to
    // raw so failures are always visible.
    if filtered.trim().is_empty() && !raw.trim().is_empty() {
        filtered = raw.to_string();
    }

    let exit_code = output
        .status
        .code()
        .unwrap_or(if output.status.success() { 0 } else { 1 });

    crate::utils::ensure_failure_visibility(&mut filtered, exit_code, &stderr);

    if let Some(hint) = crate::tee::tee_and_hint(&raw, "terragrunt", exit_code) {
        println!("{}\n{}", filtered, hint);
    } else {
        println!("{}", filtered);
    }

    let original_cmd = format!("terragrunt {}", args.join(" "));
    let rtk_cmd = format!("rtk terragrunt {}", args.join(" "));
    timer.track(&original_cmd, &rtk_cmd, &raw, &filtered);

    // Preserve exit code for CI/CD pipelines
    if !output.status.success() {
        std::process::exit(exit_code);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Core filter
// ---------------------------------------------------------------------------

/// Parse Terragrunt/Terraform output and return a compact summary.
///
/// Designed for 80%+ token reduction over typical plan/apply verbosity.
pub(crate) fn filter_terragrunt_output(output: &str) -> String {
    let mut state = ParseState::Scanning;
    let mut kept: Vec<String> = Vec::new();

    for raw_line in output.lines() {
        let clean = crate::utils::strip_ansi(raw_line);
        let (class, content) = classify_line(&clean);
        let trimmed = content.trim();

        match class {
            LineClass::Noise => continue,
            LineClass::Error => {
                // Errors/warnings (incl. box framing) break any open section and
                // are always kept.
                state = ParseState::Scanning;
                if !trimmed.is_empty() {
                    kept.push(trimmed.to_string());
                }
                continue;
            }
            LineClass::Body => {}
        }

        // Box framing occasionally reaches us as body - keep it.
        if is_box_line(trimmed) {
            state = ParseState::Scanning;
            kept.push(trimmed.to_string());
            continue;
        }

        // Stateful output blocks.
        match state {
            ParseState::OutputChanges => {
                if trimmed.is_empty() {
                    state = ParseState::Scanning;
                } else if starts_with_change_symbol(trimmed) {
                    kept.push(format!("  {}", trimmed));
                    continue;
                } else {
                    state = ParseState::Scanning;
                }
            }
            ParseState::Outputs => {
                // Terraform prints a blank line between the `Outputs:` header and
                // the first value, so blanks are skipped rather than ending the
                // block; a non-empty line without ` = ` ends it. This section is
                // terminal in practice, so EOF closes it naturally.
                if trimmed.is_empty() {
                    continue;
                } else if trimmed.contains(" = ") {
                    kept.push(format!("  {}", trimmed));
                    continue;
                } else {
                    state = ParseState::Scanning;
                }
            }
            ParseState::Scanning => {}
        }

        // Section transitions.
        if trimmed == "Changes to Outputs:" {
            kept.push(trimmed.to_string());
            state = ParseState::OutputChanges;
            continue;
        }
        if trimmed == "Outputs:" {
            kept.push(trimmed.to_string());
            state = ParseState::Outputs;
            continue;
        }

        // Scanning keep-list.
        if is_signal_line(trimmed) {
            kept.push(trimmed.to_string());
        }
    }

    if kept.is_empty() {
        return String::new();
    }

    // Deduplicate ONLY error/warning/box lines: Terragrunt prints the terraform
    // error box once as STDERR and again in its own `error occurred:` footer, and
    // those collapse to identical lines after prefix-stripping. Plan body lines are
    // left alone - different resources can legitimately share an identical change
    // (e.g. the same `~ instance_type = "t3.micro" -> "t3.small"` across a fleet),
    // and deduping those would drop real changes.
    let mut seen = std::collections::HashSet::new();
    kept.retain(|line| {
        let t = line.trim();
        if is_box_line(t) || t.starts_with("Error:") || t.starts_with("Warning:") {
            seen.insert(line.clone())
        } else {
            true
        }
    });

    kept.join("\n")
}

/// Strip the Terragrunt log prefix from an ANSI-cleaned line and classify it.
///
/// Returns the treatment class and the content (with any `terraform:`/`tofu:`
/// sub-prefix removed).
fn classify_line(clean: &str) -> (LineClass, String) {
    if let Some(caps) = TG_LOG_PREFIX.captures(clean) {
        let level = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        let content = strip_tf_prefix(caps.get(2).map(|m| m.as_str()).unwrap_or(""));
        return (classify_level(level), content);
    }
    if let Some(caps) = TG_OLD_PREFIX.captures(clean) {
        let level = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        let content = strip_tf_prefix(caps.get(2).map(|m| m.as_str()).unwrap_or(""));
        return (classify_level(level), content);
    }
    // No log prefix - raw terraform/terragrunt content.
    (LineClass::Body, clean.to_string())
}

/// Map a Terragrunt log level to a treatment class (case-insensitive).
fn classify_level(level: &str) -> LineClass {
    match level.to_ascii_uppercase().as_str() {
        "INFO" | "DEBUG" | "TRACE" => LineClass::Noise,
        "ERROR" | "STDERR" | "WARN" | "WARNING" => LineClass::Error,
        // STDOUT and anything unrecognized are treated as body.
        _ => LineClass::Body,
    }
}

/// Remove a leading `terraform:` / `tofu:` sub-prefix if present.
///
/// Tries the space-suffixed form first so ordinary content keeps its indentation,
/// then the bare-colon form so a blank forwarded line (`terraform:`) collapses to
/// empty instead of leaking the tag.
fn strip_tf_prefix(s: &str) -> String {
    for p in ["terraform: ", "tofu: ", "terraform:", "tofu:"] {
        if let Some(rest) = s.strip_prefix(p) {
            return rest.to_string();
        }
    }
    s.to_string()
}

/// True if a trimmed line is part of a Terraform error/warning box (`╷ │ ╵`).
fn is_box_line(trimmed: &str) -> bool {
    matches!(trimmed.chars().next(), Some('│') | Some('╷') | Some('╵'))
}

/// True if a trimmed line begins with a Terraform plan change symbol
/// (`+`, `~`, or `-`) followed by a space.
fn starts_with_change_symbol(trimmed: &str) -> bool {
    trimmed.starts_with("+ ") || trimmed.starts_with("~ ") || trimmed.starts_with("- ")
}

/// True if a trimmed scanning-state line carries plan/apply signal worth keeping.
fn is_signal_line(trimmed: &str) -> bool {
    if trimmed.is_empty() {
        return false;
    }

    // Resource action headers: `# aws_instance.web will be updated in-place`
    if trimmed.starts_with("# ") && trimmed.contains(" will be ") {
        return true;
    }

    // In-place attribute changes: `~ instance_type = "t3.micro" -> "t3.small"`.
    // The block-opener line (`~ resource "aws_instance" "web" {`) has no ` = `,
    // so it is excluded - the `#` header above already names the resource.
    if trimmed.starts_with("~ ") && trimmed.contains(" = ") {
        return true;
    }

    // Plan / apply / destroy summaries and the no-op result.
    if trimmed.starts_with("Plan:")
        || trimmed.starts_with("Apply complete!")
        || trimmed.starts_with("Destroy complete!")
        || trimmed.starts_with("No changes.")
        || trimmed.starts_with("Your infrastructure matches the configuration")
    {
        return true;
    }

    // Errors and warnings outside the box framing.
    if trimmed.starts_with("Error:") || trimmed.starts_with("Warning:") {
        return true;
    }

    false
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

    // Real captures from Terragrunt 1.0.8 (ANSI-colored, log-prefixed).
    const TG_PLAN_REAL: &str = include_str!("../tests/fixtures/terragrunt_plan.txt");
    const TG_ERROR_REAL: &str = include_str!("../tests/fixtures/terragrunt_error.txt");

    // Synthesized from the confirmed line format (apply is safety-gated; these
    // reproduce the real `<ts> LEVEL terraform: <content>` shape).
    const TG_APPLY: &str = "\
11:30:00.000 INFO   terraform: Initializing the backend...
11:30:00.100 STDOUT terraform: null_resource.example: Creating...
11:30:00.200 STDOUT terraform: null_resource.example: Creation complete after 0s [id=1234567890]
11:30:00.300 STDOUT terraform: null_resource.second: Creating...
11:30:00.400 STDOUT terraform: null_resource.second: Creation complete after 0s [id=9876543210]
11:30:00.500 STDOUT terraform:
11:30:00.500 STDOUT terraform: Apply complete! Resources: 2 added, 0 changed, 0 destroyed.
11:30:00.500 STDOUT terraform:
11:30:00.500 STDOUT terraform: Outputs:
11:30:00.500 STDOUT terraform:
11:30:00.500 STDOUT terraform: example_id = \"1234567890\"
";

    const TG_NO_CHANGES: &str = "\
11:31:00.000 INFO   terraform: Initializing the backend...
11:31:00.100 STDOUT terraform: null_resource.example: Refreshing state... [id=1234567890]
11:31:00.200 STDOUT terraform: null_resource.second: Refreshing state... [id=9876543210]
11:31:00.300 STDOUT terraform:
11:31:00.300 STDOUT terraform: No changes. Your infrastructure matches the configuration.
11:31:00.300 STDOUT terraform:
11:31:00.300 STDOUT terraform: Terraform has compared your real infrastructure against your configuration and found no differences, so no changes are needed.
";

    const TG_UPDATE: &str = "\
11:32:00.000 STDOUT terraform:   # null_resource.example will be updated in-place
11:32:00.000 STDOUT terraform:   ~ resource \"null_resource\" \"example\" {
11:32:00.000 STDOUT terraform:       ~ triggers = {
11:32:00.000 STDOUT terraform:           ~ \"foo\" = \"bar\" -> \"CHANGED\"
11:32:00.000 STDOUT terraform:         }
11:32:00.000 STDOUT terraform:     }
11:32:00.000 STDOUT terraform: Plan: 0 to add, 1 to change, 0 to destroy.
";

    // Legacy `time=... level=... msg=...` format (older Terragrunt).
    const TG_LEGACY: &str = "\
time=2024-01-15T10:30:00Z level=info msg=terraform: Initializing the backend... prefix=[/infra]
time=2024-01-15T10:30:05Z level=error msg=terraform: something went wrong prefix=[/infra]
";

    // -----------------------------------------------------------------------
    // Real plan fixture
    // -----------------------------------------------------------------------

    #[test]
    fn test_real_plan_keeps_headers_and_summary() {
        let result = filter_terragrunt_output(TG_PLAN_REAL);
        assert!(
            result.contains("# null_resource.example will be created"),
            "Expected create header, got:\n{result}"
        );
        assert!(
            result.contains("# null_resource.second will be created"),
            "Expected second create header, got:\n{result}"
        );
        assert!(
            result.contains("Plan: 2 to add, 0 to change, 0 to destroy."),
            "Expected plan summary, got:\n{result}"
        );
        assert!(
            result.contains("Changes to Outputs:") && result.contains("example_id"),
            "Expected output changes, got:\n{result}"
        );
    }

    #[test]
    fn test_real_plan_strips_noise() {
        let result = filter_terragrunt_output(TG_PLAN_REAL);
        assert!(!result.contains("Downloading"), "download noise:\n{result}");
        assert!(!result.contains("Initializing"), "init noise:\n{result}");
        assert!(
            !result.contains("STDOUT") && !result.contains("terraform:"),
            "log prefix must be stripped:\n{result}"
        );
        // Per-attribute create dump should be gone.
        assert!(
            !result.contains("\"baz\" = \"qux\""),
            "attribute dump should be stripped:\n{result}"
        );
        assert!(
            !result.contains("Note: You didn't use"),
            "trailing note should be stripped:\n{result}"
        );
        // ANSI must be gone.
        assert!(
            !result.contains('\u{1b}'),
            "ANSI must be stripped:\n{result}"
        );
    }

    #[test]
    fn test_real_plan_token_savings() {
        let input_tokens = count_tokens(TG_PLAN_REAL);
        let filtered = filter_terragrunt_output(TG_PLAN_REAL);
        let output_tokens = count_tokens(&filtered);
        let savings_pct = 100.0 - (output_tokens as f64 / input_tokens as f64 * 100.0);
        assert!(
            savings_pct >= 70.0,
            "Expected >=70% savings on real plan, got {savings_pct:.1}% \
             ({input_tokens} -> {output_tokens} tokens)\nFiltered:\n{filtered}"
        );
    }

    // -----------------------------------------------------------------------
    // Real error fixture
    // -----------------------------------------------------------------------

    #[test]
    fn test_real_error_keeps_error_detail() {
        let result = filter_terragrunt_output(TG_ERROR_REAL);
        assert!(
            result.contains("Error: Call to unknown function"),
            "Expected error title, got:\n{result}"
        );
        assert!(
            result.contains("no function named \"nonexistent_function\""),
            "Expected error detail, got:\n{result}"
        );
        assert!(
            !result.contains("Initializing"),
            "init noise should be stripped even on error:\n{result}"
        );
        assert!(
            !result.contains('\u{1b}'),
            "ANSI must be stripped:\n{result}"
        );
    }

    // -----------------------------------------------------------------------
    // Synthesized apply / no-change / update
    // -----------------------------------------------------------------------

    #[test]
    fn test_apply_keeps_summary_and_outputs() {
        let result = filter_terragrunt_output(TG_APPLY);
        assert!(
            result.contains("Apply complete! Resources: 2 added, 0 changed, 0 destroyed."),
            "Expected apply summary, got:\n{result}"
        );
        assert!(
            result.contains("Outputs:"),
            "Expected outputs header:\n{result}"
        );
        assert!(
            result.contains("example_id = \"1234567890\""),
            "Expected output value, got:\n{result}"
        );
    }

    #[test]
    fn test_apply_strips_progress() {
        let result = filter_terragrunt_output(TG_APPLY);
        assert!(!result.contains("Creating..."), "progress noise:\n{result}");
        assert!(
            !result.contains("Creation complete"),
            "per-resource completion should be stripped:\n{result}"
        );
        assert!(!result.contains("Initializing"), "init noise:\n{result}");
    }

    #[test]
    fn test_no_changes_kept_and_noise_stripped() {
        let result = filter_terragrunt_output(TG_NO_CHANGES);
        assert!(
            result.contains("No changes. Your infrastructure matches the configuration."),
            "Expected no-changes summary, got:\n{result}"
        );
        assert!(
            !result.contains("Refreshing state"),
            "refresh noise should be stripped:\n{result}"
        );
        assert!(
            !result.contains("Terraform has compared"),
            "verbose explanation should be stripped:\n{result}"
        );
    }

    #[test]
    fn test_update_keeps_change_but_drops_block_opener() {
        let result = filter_terragrunt_output(TG_UPDATE);
        assert!(
            result.contains("# null_resource.example will be updated in-place"),
            "Expected update header, got:\n{result}"
        );
        assert!(
            result.contains("~ \"foo\" = \"bar\" -> \"CHANGED\""),
            "Expected in-place attribute change, got:\n{result}"
        );
        assert!(
            result.contains("Plan: 0 to add, 1 to change, 0 to destroy."),
            "Expected plan summary, got:\n{result}"
        );
        assert!(
            !result.contains("resource \"null_resource\" \"example\" {"),
            "Block opener should be dropped:\n{result}"
        );
    }

    // -----------------------------------------------------------------------
    // Legacy format + edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn test_legacy_format_drops_info_keeps_error() {
        let result = filter_terragrunt_output(TG_LEGACY);
        assert!(
            !result.contains("Initializing the backend"),
            "legacy info should be dropped:\n{result}"
        );
        assert!(
            result.contains("something went wrong"),
            "legacy error should be kept:\n{result}"
        );
    }

    #[test]
    fn test_identical_changes_across_resources_preserved() {
        // Two resources with the *same* in-place change must both survive: dedup
        // is restricted to error/box lines, not plan body. (Gemini review, PR #13.)
        let input = "\
11:00:00.000 STDOUT terraform:   # aws_instance.web will be updated in-place
11:00:00.000 STDOUT terraform:       ~ instance_type = \"t3.micro\" -> \"t3.small\"
11:00:00.000 STDOUT terraform:   # aws_instance.api will be updated in-place
11:00:00.000 STDOUT terraform:       ~ instance_type = \"t3.micro\" -> \"t3.small\"
11:00:00.000 STDOUT terraform: Plan: 0 to add, 2 to change, 0 to destroy.
";
        let result = filter_terragrunt_output(input);
        assert!(result.contains("# aws_instance.web will be updated in-place"));
        assert!(result.contains("# aws_instance.api will be updated in-place"));
        assert_eq!(
            result.matches("~ instance_type").count(),
            2,
            "both resources' identical changes must be preserved, got:\n{result}"
        );
    }

    #[test]
    fn test_error_box_deduplicated_to_single_copy() {
        // The error box is printed twice (STDERR + terragrunt footer); it must
        // collapse to one copy.
        let result = filter_terragrunt_output(TG_ERROR_REAL);
        assert_eq!(
            result.matches("Error: Call to unknown function").count(),
            1,
            "duplicated error box should be deduped to one, got:\n{result}"
        );
    }

    #[test]
    fn test_empty_output_returns_empty() {
        assert_eq!(filter_terragrunt_output(""), "");
    }

    #[test]
    fn test_classify_level_case_insensitive() {
        assert_eq!(classify_level("INFO"), LineClass::Noise);
        assert_eq!(classify_level("info"), LineClass::Noise);
        assert_eq!(classify_level("STDOUT"), LineClass::Body);
        assert_eq!(classify_level("ERROR"), LineClass::Error);
        assert_eq!(classify_level("STDERR"), LineClass::Error);
    }

    #[test]
    fn test_strip_tf_prefix() {
        assert_eq!(strip_tf_prefix("terraform:   # x"), "  # x");
        assert_eq!(strip_tf_prefix("tofu: Plan: 1 to add"), "Plan: 1 to add");
        assert_eq!(strip_tf_prefix("no prefix here"), "no prefix here");
        // Blank forwarded line collapses to empty (no leaked tag).
        assert_eq!(strip_tf_prefix("terraform:"), "");
    }
}
