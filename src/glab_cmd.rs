//! GitLab CLI (glab) command output compression.
//!
//! Provides token-optimized alternatives to verbose `glab` commands.
//! Focuses on extracting essential information from JSON outputs.
//!
//! Subcommands handled:
//! - `mr list`      compact one-line-per-MR (number, title, branch, status)
//! - `mr view <id>` title, state, author, branch, CI status, URL
//! - `ci list`      compact pipeline list (id, status, ref, sha)
//! - `ci status`    current branch pipeline with stage breakdown
//! - `issue list`   compact one-line-per-issue
//! - everything else passes through unmodified

use crate::guard;
use crate::tracking;
use crate::utils::truncate;
use anyhow::{Context, Result};
use serde_json::Value;
use std::process::Command;

/// Detect if the user explicitly requested JSON output so we can passthrough
/// instead of double-filtering.
///
/// Handles all long/short flag forms used across glab subcommands:
/// `--output json`, `-F json`, `-O json`, `--output=json`.
fn has_json_output_flag(args: &[String]) -> bool {
    let mut iter = args.iter().peekable();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            // Two-token forms: `-F json` / `--output json` / `-O json`
            "-F" | "--output" | "-O" => {
                if iter.peek().map(|v| v.as_str()) == Some("json") {
                    return true;
                }
            }
            // Equals form: `--output=json`
            s if s.starts_with("--output=json") => return true,
            _ => {}
        }
    }
    false
}

/// Extract the first positional identifier (MR/issue number or branch name)
/// from args, returning it together with the remaining flags.
///
/// Flags that consume a value (e.g. `-R owner/repo`) are kept in extra_args
/// so they can be forwarded to the underlying glab command.
fn extract_identifier_and_extra_args(args: &[String]) -> Option<(String, Vec<String>)> {
    if args.is_empty() {
        return None;
    }

    // Flags that consume the next token as their value
    const FLAGS_WITH_VALUE: &[&str] = &[
        "-R",
        "--repo",
        "-F",
        "--output",
        "-O",
        "-a",
        "--assignee",
        "-l",
        "--label",
        "-m",
        "--milestone",
        "-P",
        "--per-page",
        "-p",
        "--page",
        "-b",
        "--branch",
    ];

    let mut identifier: Option<String> = None;
    let mut extra: Vec<String> = Vec::new();
    let mut skip_next = false;

    for arg in args {
        if skip_next {
            extra.push(arg.clone());
            skip_next = false;
            continue;
        }
        if FLAGS_WITH_VALUE.contains(&arg.as_str()) {
            extra.push(arg.clone());
            skip_next = true;
            continue;
        }
        if arg.starts_with('-') {
            extra.push(arg.clone());
            continue;
        }
        // First non-flag token is the identifier (number/branch name)
        if identifier.is_none() {
            identifier = Some(arg.clone());
        } else {
            extra.push(arg.clone());
        }
    }

    identifier.map(|id| (id, extra))
}

// ---------------------------------------------------------------------------
// Filter functions (pure: JSON string in, compact string out)
// ---------------------------------------------------------------------------

/// Filter `glab mr list --output json` output to compact one-line-per-MR format.
pub fn filter_mr_list(json_str: &str, ultra_compact: bool) -> String {
    let json: Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return String::new(),
    };

    let mrs = match json.as_array() {
        Some(a) => a,
        None => return String::new(),
    };

    if mrs.is_empty() {
        return "MRs: none\n".to_string();
    }

    let mut out = if ultra_compact {
        "MRs\n".to_string()
    } else {
        "Merge Requests\n".to_string()
    };

    for mr in mrs.iter().take(20) {
        let iid = mr["iid"].as_i64().unwrap_or(0);
        let title = mr["title"].as_str().unwrap_or("???");
        let state = mr["state"].as_str().unwrap_or("???");
        let author = mr["author"]["username"].as_str().unwrap_or("???");
        let branch = mr["source_branch"].as_str().unwrap_or("");
        let pipeline_status = mr["pipeline"]["status"].as_str().unwrap_or("");
        let draft = mr["draft"].as_bool().unwrap_or(false)
            || mr["work_in_progress"].as_bool().unwrap_or(false);

        let state_icon = match state {
            "opened" => {
                if draft {
                    "D"
                } else {
                    "O"
                }
            }
            "merged" => "M",
            "closed" => "C",
            _ => "?",
        };

        let ci_tag = match pipeline_status {
            "success" => " [CI:ok]",
            "failed" => " [CI:fail]",
            "running" => " [CI:run]",
            "pending" => " [CI:pend]",
            _ => "",
        };

        let branch_part = if !branch.is_empty() {
            format!(" ({})", truncate(branch, 28))
        } else {
            String::new()
        };

        out.push_str(&format!(
            "  {} !{} {}{} @{}{}\n",
            state_icon,
            iid,
            truncate(title, 52),
            branch_part,
            author,
            ci_tag,
        ));
    }

    if mrs.len() > 20 {
        out.push_str(&format!("  ... {} more\n", mrs.len() - 20));
    }

    out
}

/// Filter `glab mr view --output json` output to compact MR summary.
pub fn filter_mr_view(json_str: &str, ultra_compact: bool) -> String {
    let json: Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return String::new(),
    };

    let iid = json["iid"].as_i64().unwrap_or(0);
    let title = json["title"].as_str().unwrap_or("???");
    let state = json["state"].as_str().unwrap_or("???");
    let author = json["author"]["username"].as_str().unwrap_or("???");
    let source_branch = json["source_branch"].as_str().unwrap_or("");
    let target_branch = json["target_branch"].as_str().unwrap_or("");
    let url = json["web_url"].as_str().unwrap_or("");
    let draft = json["draft"].as_bool().unwrap_or(false)
        || json["work_in_progress"].as_bool().unwrap_or(false);
    let merge_status = json["merge_status"].as_str().unwrap_or("");

    let state_label = if draft {
        format!("{}/draft", state)
    } else {
        state.to_string()
    };

    let merge_tag = match merge_status {
        "can_be_merged" => " mergeable",
        "cannot_be_merged" | "cannot_be_merged_recheck" => " conflicts",
        _ => "",
    };

    let mut out = format!("!{} {} [{}{}]\n", iid, title, state_label, merge_tag);
    out.push_str(&format!(
        "  @{} | {} -> {}\n",
        author, source_branch, target_branch
    ));

    // Pipeline / CI status
    if let Some(pipeline) = json["pipeline"].as_object() {
        let status = pipeline
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let pid = pipeline.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
        out.push_str(&format!("  CI: {} (pipeline #{})\n", status, pid));
    }

    // Votes
    let upvotes = json["upvotes"].as_i64().unwrap_or(0);
    let downvotes = json["downvotes"].as_i64().unwrap_or(0);
    if upvotes > 0 || downvotes > 0 {
        out.push_str(&format!("  +{} -{}\n", upvotes, downvotes));
    }

    // Reviewers
    if let Some(reviewers) = json["reviewers"].as_array() {
        if !reviewers.is_empty() {
            let names: Vec<&str> = reviewers
                .iter()
                .take(3)
                .filter_map(|r| r["username"].as_str())
                .collect();
            out.push_str(&format!("  Reviewers: {}\n", names.join(", ")));
        }
    }

    if !url.is_empty() {
        out.push_str(&format!("  {}\n", url));
    }

    // Description preview: first 3 non-empty lines, capped at 200 chars.
    // Omitted in ultra_compact mode. Use `glab mr view --comments` for the full body.
    if !ultra_compact {
        if let Some(desc) = json["description"].as_str() {
            let trimmed = desc.trim();
            if !trimmed.is_empty() {
                let preview: String = trimmed
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .take(3)
                    .collect::<Vec<_>>()
                    .join("\n");
                if !preview.is_empty() {
                    out.push_str(&format!("---\n{}\n", truncate(&preview, 200)));
                }
            }
        }
    }

    out
}

/// Filter `glab ci list --output json` output to compact pipeline list.
pub fn filter_ci_list(json_str: &str, ultra_compact: bool) -> String {
    let json: Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return String::new(),
    };

    let pipelines = match json.as_array() {
        Some(a) => a,
        None => return String::new(),
    };

    if pipelines.is_empty() {
        return "Pipelines: none\n".to_string();
    }

    let header = if ultra_compact {
        "Pipelines\n"
    } else {
        "CI Pipelines\n"
    };
    let mut out = header.to_string();

    for pipeline in pipelines.iter().take(15) {
        let id = pipeline["id"].as_i64().unwrap_or(0);
        let status = pipeline["status"].as_str().unwrap_or("?");
        let ref_name = pipeline["ref"].as_str().unwrap_or("");
        let sha = pipeline["sha"].as_str().unwrap_or("");
        let sha_short = &sha[..sha.len().min(8)];

        let status_tag = match status {
            "success" => "ok",
            "failed" => "FAIL",
            "running" => "run",
            "pending" => "pend",
            "canceled" => "cancel",
            "skipped" => "skip",
            "manual" => "manual",
            other => other,
        };

        out.push_str(&format!(
            "  #{} [{}] {} {}\n",
            id,
            status_tag,
            truncate(ref_name, 30),
            sha_short,
        ));
    }

    if pipelines.len() > 15 {
        out.push_str(&format!("  ... {} more\n", pipelines.len() - 15));
    }

    out
}

/// Filter `glab ci status --output json` output to compact pipeline status.
///
/// The response may be a single pipeline object or a single-element array.
pub fn filter_ci_status(json_str: &str, _ultra_compact: bool) -> String {
    let json: Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return String::new(),
    };

    // ci status returns either a single object or a one-element array
    let pipeline = if json.is_array() {
        match json.as_array().and_then(|a| a.first()).cloned() {
            Some(p) => p,
            None => return "Pipelines: none\n".to_string(),
        }
    } else {
        json
    };

    let id = pipeline["id"].as_i64().unwrap_or(0);
    let status = pipeline["status"].as_str().unwrap_or("unknown");
    let ref_name = pipeline["ref"].as_str().unwrap_or("");
    let sha = pipeline["sha"].as_str().unwrap_or("");
    let sha_short = &sha[..sha.len().min(8)];
    let url = pipeline["web_url"].as_str().unwrap_or("");

    let mut out = format!("Pipeline #{} [{}] {} {}\n", id, status, ref_name, sha_short);

    // Stage-level breakdown
    if let Some(stages) = pipeline["stages"].as_array() {
        for stage in stages {
            let name = stage["name"].as_str().unwrap_or("?");
            let s = stage["status"].as_str().unwrap_or("?");
            out.push_str(&format!("  {} [{}]\n", name, s));
        }
    }

    // Failed job names (when jobs array is present)
    if let Some(jobs) = pipeline["jobs"].as_array() {
        let failed: Vec<&str> = jobs
            .iter()
            .filter(|j| j["status"].as_str() == Some("failed"))
            .filter_map(|j| j["name"].as_str())
            .collect();
        if !failed.is_empty() {
            out.push_str(&format!("  Failed jobs: {}\n", failed.join(", ")));
        }
    }

    if !url.is_empty() {
        out.push_str(&format!("  {}\n", url));
    }

    out
}

/// Filter `glab issue list --output json` output to compact one-line-per-issue.
pub fn filter_issue_list(json_str: &str, ultra_compact: bool) -> String {
    let json: Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return String::new(),
    };

    let issues = match json.as_array() {
        Some(a) => a,
        None => return String::new(),
    };

    if issues.is_empty() {
        return "Issues: none\n".to_string();
    }

    let header = if ultra_compact {
        "Issues\n"
    } else {
        "Issues\n"
    };
    let mut out = header.to_string();

    for issue in issues.iter().take(20) {
        let iid = issue["iid"].as_i64().unwrap_or(0);
        let title = issue["title"].as_str().unwrap_or("???");
        let state = issue["state"].as_str().unwrap_or("???");
        let author = issue["author"]["username"].as_str().unwrap_or("???");

        let labels: Vec<&str> = issue["labels"]
            .as_array()
            .map(|l| l.iter().filter_map(|v| v.as_str()).take(3).collect())
            .unwrap_or_default();

        let state_icon = match state {
            "opened" => "O",
            "closed" => "C",
            _ => "?",
        };

        let label_part = if !labels.is_empty() {
            format!(" [{}]", labels.join(","))
        } else {
            String::new()
        };

        out.push_str(&format!(
            "  {} #{} {}{} @{}\n",
            state_icon,
            iid,
            truncate(title, 52),
            label_part,
            author,
        ));
    }

    if issues.len() > 20 {
        out.push_str(&format!("  ... {} more\n", issues.len() - 20));
    }

    out
}

// ---------------------------------------------------------------------------
// Command execution helpers
// ---------------------------------------------------------------------------

fn run_passthrough(cmd: &str, subcommand: &str, args: &[String]) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    let mut command = Command::new(cmd);
    command.arg(subcommand);
    for arg in args {
        command.arg(arg);
    }

    let status = command
        .status()
        .context(format!("Failed to run {} {}", cmd, subcommand))?;

    let args_str = tracking::args_display(&args.iter().map(|s| s.into()).collect::<Vec<_>>());
    timer.track_passthrough(
        &format!("{} {} {}", cmd, subcommand, args_str),
        &format!("rtk {} {} {} (passthrough)", cmd, subcommand, args_str),
    );

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}

fn run_passthrough_with_extra(cmd: &str, base_args: &[&str], extra_args: &[String]) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    let mut command = Command::new(cmd);
    for arg in base_args {
        command.arg(arg);
    }
    for arg in extra_args {
        command.arg(arg);
    }

    let status =
        command
            .status()
            .context(format!("Failed to run {} {}", cmd, base_args.join(" ")))?;

    let full_cmd = format!(
        "{} {} {}",
        cmd,
        base_args.join(" "),
        tracking::args_display(&extra_args.iter().map(|s| s.into()).collect::<Vec<_>>())
    );
    timer.track_passthrough(&full_cmd, &format!("rtk {} (passthrough)", full_cmd));

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Subcommand routers
// ---------------------------------------------------------------------------

fn mr_list(args: &[String], ultra_compact: bool) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    let mut cmd = Command::new("glab");
    cmd.args(["mr", "list", "--output", "json"]);
    for arg in args {
        cmd.arg(arg);
    }

    let output = cmd.output().context("Failed to run glab mr list")?;
    let raw = String::from_utf8_lossy(&output.stdout).to_string();

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        timer.track("glab mr list", "rtk glab mr list", &stderr, &stderr);
        eprintln!("{}", stderr.trim());
        std::process::exit(output.status.code().unwrap_or(1));
    }

    let filtered = filter_mr_list(&raw, ultra_compact);
    let to_print = guard::never_worse(&raw, &filtered);
    print!("{}", to_print);

    timer.track("glab mr list", "rtk glab mr list", &raw, &filtered);
    Ok(())
}

fn mr_view(args: &[String], _verbose: u8, ultra_compact: bool) -> Result<()> {
    let (mr_id, extra_args) = match extract_identifier_and_extra_args(args) {
        Some(result) => result,
        None => {
            // No ID — glab will use current branch; passthrough without JSON flag
            // since we can't reliably reconstruct iid from text output
            let mut view_args = vec!["view".to_string()];
            view_args.extend_from_slice(args);
            return run_passthrough("glab", "mr", &view_args);
        }
    };

    // --web opens a browser; no filtering needed
    if extra_args.iter().any(|a| a == "--web" || a == "-w") {
        return run_passthrough_with_extra("glab", &["mr", "view", &mr_id], &extra_args);
    }

    let timer = tracking::TimedExecution::start();

    let mut cmd = Command::new("glab");
    cmd.args(["mr", "view", &mr_id, "--output", "json"]);
    for arg in &extra_args {
        cmd.arg(arg);
    }

    let output = cmd.output().context("Failed to run glab mr view")?;
    let raw = String::from_utf8_lossy(&output.stdout).to_string();

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        timer.track(
            &format!("glab mr view {}", mr_id),
            &format!("rtk glab mr view {}", mr_id),
            &stderr,
            &stderr,
        );
        eprintln!("{}", stderr.trim());
        std::process::exit(output.status.code().unwrap_or(1));
    }

    let filtered = filter_mr_view(&raw, ultra_compact);
    let to_print = guard::never_worse(&raw, &filtered);
    print!("{}", to_print);

    timer.track(
        &format!("glab mr view {}", mr_id),
        &format!("rtk glab mr view {}", mr_id),
        &raw,
        &filtered,
    );
    Ok(())
}

fn run_mr(args: &[String], verbose: u8, ultra_compact: bool) -> Result<()> {
    if args.is_empty() {
        return run_passthrough("glab", "mr", args);
    }

    match args[0].as_str() {
        "list" | "ls" => mr_list(&args[1..], ultra_compact),
        "view" => mr_view(&args[1..], verbose, ultra_compact),
        _ => run_passthrough("glab", "mr", args),
    }
}

fn ci_list(args: &[String], ultra_compact: bool) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    let mut cmd = Command::new("glab");
    cmd.args(["ci", "list", "--output", "json"]);
    for arg in args {
        cmd.arg(arg);
    }

    let output = cmd.output().context("Failed to run glab ci list")?;
    let raw = String::from_utf8_lossy(&output.stdout).to_string();

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        timer.track("glab ci list", "rtk glab ci list", &stderr, &stderr);
        eprintln!("{}", stderr.trim());
        std::process::exit(output.status.code().unwrap_or(1));
    }

    let filtered = filter_ci_list(&raw, ultra_compact);
    let to_print = guard::never_worse(&raw, &filtered);
    print!("{}", to_print);

    timer.track("glab ci list", "rtk glab ci list", &raw, &filtered);
    Ok(())
}

fn ci_status(args: &[String], _verbose: u8, ultra_compact: bool) -> Result<()> {
    // --live produces interactive output incompatible with --output json
    if args.iter().any(|a| a == "--live" || a == "-l") {
        return run_passthrough_with_extra("glab", &["ci", "status"], args);
    }

    let timer = tracking::TimedExecution::start();

    let mut cmd = Command::new("glab");
    cmd.args(["ci", "status", "--output", "json"]);
    for arg in args {
        // --compact conflicts with --output json per glab's own validation
        if arg == "--compact" || arg == "-c" {
            continue;
        }
        cmd.arg(arg);
    }

    let output = cmd.output().context("Failed to run glab ci status")?;
    let raw = String::from_utf8_lossy(&output.stdout).to_string();

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        timer.track("glab ci status", "rtk glab ci status", &stderr, &stderr);
        eprintln!("{}", stderr.trim());
        std::process::exit(output.status.code().unwrap_or(1));
    }

    let filtered = filter_ci_status(&raw, ultra_compact);
    let to_print = guard::never_worse(&raw, &filtered);
    print!("{}", to_print);

    timer.track("glab ci status", "rtk glab ci status", &raw, &filtered);
    Ok(())
}

fn run_ci(args: &[String], verbose: u8, ultra_compact: bool) -> Result<()> {
    if args.is_empty() {
        return run_passthrough("glab", "ci", args);
    }

    match args[0].as_str() {
        "list" => ci_list(&args[1..], ultra_compact),
        "status" => ci_status(&args[1..], verbose, ultra_compact),
        _ => run_passthrough("glab", "ci", args),
    }
}

fn issue_list(args: &[String], ultra_compact: bool) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    let mut cmd = Command::new("glab");
    // issue list uses --output (short: -O) for text/json
    cmd.args(["issue", "list", "--output", "json"]);
    for arg in args {
        cmd.arg(arg);
    }

    let output = cmd.output().context("Failed to run glab issue list")?;
    let raw = String::from_utf8_lossy(&output.stdout).to_string();

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        timer.track("glab issue list", "rtk glab issue list", &stderr, &stderr);
        eprintln!("{}", stderr.trim());
        std::process::exit(output.status.code().unwrap_or(1));
    }

    let filtered = filter_issue_list(&raw, ultra_compact);
    let to_print = guard::never_worse(&raw, &filtered);
    print!("{}", to_print);

    timer.track("glab issue list", "rtk glab issue list", &raw, &filtered);
    Ok(())
}

fn run_issue(args: &[String], _verbose: u8, ultra_compact: bool) -> Result<()> {
    if args.is_empty() {
        return run_passthrough("glab", "issue", args);
    }

    match args[0].as_str() {
        "list" | "ls" => issue_list(&args[1..], ultra_compact),
        _ => run_passthrough("glab", "issue", args),
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Run a glab command with token-optimized output.
///
/// # Arguments
/// - `subcommand`: first glab subcommand (e.g. `"mr"`, `"ci"`, `"issue"`)
/// - `args`: remaining arguments (subcommand's own sub-subcommand + flags)
/// - `verbose`: verbosity level (0 = compact, 1+ = more detail)
/// - `ultra_compact`: when true, omit all decorative text and use shortest labels
pub fn run(subcommand: &str, args: &[String], verbose: u8, ultra_compact: bool) -> Result<()> {
    // When the user explicitly requests JSON output, pass through unchanged
    if has_json_output_flag(args) {
        return run_passthrough("glab", subcommand, args);
    }

    match subcommand {
        "mr" => run_mr(args, verbose, ultra_compact),
        "ci" => run_ci(args, verbose, ultra_compact),
        "issue" => run_issue(args, verbose, ultra_compact),
        _ => run_passthrough("glab", subcommand, args),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn count_tokens(text: &str) -> usize {
        text.split_whitespace().count()
    }

    // ----- fixtures --------------------------------------------------------

    /// Realistic `glab mr list --output json` fixture: 3 MRs with pipeline + draft states.
    const MR_LIST_JSON: &str = r###"[
  {
    "id": 20001,
    "iid": 45,
    "title": "Fix authentication token refresh bug in OAuth2 client middleware",
    "state": "opened",
    "author": {"id": 100, "username": "jdoe", "name": "John Doe"},
    "source_branch": "fix/oauth2-token-refresh",
    "target_branch": "main",
    "web_url": "https://gitlab.com/mygroup/myproject/-/merge_requests/45",
    "draft": false,
    "work_in_progress": false,
    "pipeline": {"id": 789, "status": "running", "ref": "fix/oauth2-token-refresh"},
    "upvotes": 3,
    "downvotes": 0,
    "reviewers": [{"username": "alice"}, {"username": "bob"}],
    "description": "## Summary\n\nThis MR fixes the token refresh bug.\n\n## Changes\n\n- Add mutex lock\n- Add tests"
  },
  {
    "id": 20000,
    "iid": 44,
    "title": "Add Kubernetes deployment manifests for staging environment with auto-scaling",
    "state": "opened",
    "author": {"id": 101, "username": "alice", "name": "Alice Smith"},
    "source_branch": "feat/k8s-staging-deployment",
    "target_branch": "main",
    "web_url": "https://gitlab.com/mygroup/myproject/-/merge_requests/44",
    "draft": true,
    "work_in_progress": true,
    "pipeline": {"id": 788, "status": "failed", "ref": "feat/k8s-staging-deployment"},
    "upvotes": 1,
    "downvotes": 0,
    "reviewers": [],
    "description": "Adds K8s deployment manifests for staging."
  },
  {
    "id": 19999,
    "iid": 43,
    "title": "Refactor database connection pool to use pgbouncer",
    "state": "merged",
    "author": {"id": 102, "username": "bob", "name": "Bob Johnson"},
    "source_branch": "refactor/db-pool-pgbouncer",
    "target_branch": "main",
    "web_url": "https://gitlab.com/mygroup/myproject/-/merge_requests/43",
    "draft": false,
    "work_in_progress": false,
    "pipeline": {"id": 787, "status": "success", "ref": "refactor/db-pool-pgbouncer"},
    "upvotes": 5,
    "downvotes": 0,
    "reviewers": [{"username": "jdoe"}],
    "description": "Switches from direct postgres connections to pgbouncer pooling."
  }
]"###;

    /// Realistic `glab mr view 45 --output json` fixture.
    const MR_VIEW_JSON: &str = r###"{
  "id": 20001,
  "iid": 45,
  "title": "Fix authentication token refresh bug in OAuth2 client middleware",
  "state": "opened",
  "author": {"id": 100, "username": "jdoe", "name": "John Doe"},
  "source_branch": "fix/oauth2-token-refresh",
  "target_branch": "main",
  "web_url": "https://gitlab.com/mygroup/myproject/-/merge_requests/45",
  "draft": false,
  "work_in_progress": false,
  "merge_status": "can_be_merged",
  "pipeline": {"id": 789, "status": "running"},
  "upvotes": 3,
  "downvotes": 0,
  "reviewers": [{"username": "alice"}, {"username": "bob"}],
  "description": "## Summary\n\nThis MR fixes the token refresh bug that causes users to be logged out unexpectedly after OAuth2 access tokens expire due to race conditions in the middleware layer.\n\n## Root Cause\n\nThe refresh logic in `middleware/oauth2.go` did not handle concurrent requests properly, leading to race conditions when multiple goroutines attempted to refresh the same token simultaneously.\n\n## Changes\n\n- Add mutex lock around token refresh logic in middleware/oauth2.go\n- Implement exponential backoff for failed refresh attempts\n- Add unit tests for concurrent refresh scenarios covering 12 edge cases\n- Update integration test suite with new OAuth2 mock server\n\n## Testing\n\n```bash\ngo test ./middleware/...\n```\n\nAll 47 tests pass. Coverage increased from 72% to 89%.\n\n## Breaking Changes\n\nNone. The middleware API is unchanged.\n\n## Rollback Plan\n\nRevert this commit: `git revert HEAD` and redeploy."
}"###;

    /// Realistic `glab ci list --output json` fixture: 3 pipelines.
    const CI_LIST_JSON: &str = r###"[
  {
    "id": 123456,
    "status": "success",
    "ref": "main",
    "sha": "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2",
    "web_url": "https://gitlab.com/mygroup/myproject/-/pipelines/123456",
    "created_at": "2026-06-29T10:00:00Z",
    "updated_at": "2026-06-29T10:15:00Z"
  },
  {
    "id": 123455,
    "status": "failed",
    "ref": "fix/oauth2-token-refresh",
    "sha": "b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3",
    "web_url": "https://gitlab.com/mygroup/myproject/-/pipelines/123455",
    "created_at": "2026-06-29T09:45:00Z",
    "updated_at": "2026-06-29T10:00:00Z"
  },
  {
    "id": 123454,
    "status": "running",
    "ref": "feat/k8s-staging-deployment",
    "sha": "c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4",
    "web_url": "https://gitlab.com/mygroup/myproject/-/pipelines/123454",
    "created_at": "2026-06-29T09:30:00Z",
    "updated_at": "2026-06-29T09:30:00Z"
  }
]"###;

    /// Realistic `glab ci status --output json` fixture: failed pipeline with stages.
    const CI_STATUS_JSON: &str = r###"{
  "id": 123455,
  "status": "failed",
  "ref": "fix/oauth2-token-refresh",
  "sha": "b2c3d4e5f6a1b2",
  "web_url": "https://gitlab.com/mygroup/myproject/-/pipelines/123455",
  "stages": [
    {"name": "build", "status": "success"},
    {"name": "test", "status": "failed"},
    {"name": "deploy", "status": "skipped"}
  ],
  "jobs": [
    {"name": "build:docker", "status": "success", "stage": "build"},
    {"name": "test:unit", "status": "failed", "stage": "test"},
    {"name": "test:integration", "status": "failed", "stage": "test"},
    {"name": "deploy:staging", "status": "skipped", "stage": "deploy"}
  ]
}"###;

    /// Realistic `glab issue list --output json` fixture: 3 issues.
    const ISSUE_LIST_JSON: &str = r###"[
  {
    "id": 5001,
    "iid": 101,
    "title": "API rate limiting not working correctly under high load conditions in production",
    "state": "opened",
    "author": {"id": 100, "username": "jdoe", "name": "John Doe"},
    "labels": ["bug", "backend", "high-priority"],
    "assignees": [{"username": "alice"}],
    "web_url": "https://gitlab.com/mygroup/myproject/-/issues/101"
  },
  {
    "id": 5000,
    "iid": 100,
    "title": "Add dark mode support to the dashboard UI using CSS custom properties",
    "state": "opened",
    "author": {"id": 101, "username": "alice", "name": "Alice Smith"},
    "labels": ["enhancement", "frontend"],
    "assignees": [],
    "web_url": "https://gitlab.com/mygroup/myproject/-/issues/100"
  },
  {
    "id": 4999,
    "iid": 99,
    "title": "Documentation update for v2.0 API release notes and migration guide",
    "state": "closed",
    "author": {"id": 102, "username": "bob", "name": "Bob Johnson"},
    "labels": ["documentation"],
    "assignees": [{"username": "bob"}],
    "web_url": "https://gitlab.com/mygroup/myproject/-/issues/99"
  }
]"###;

    // ----- has_json_output_flag --------------------------------------------

    #[test]
    fn test_has_json_flag_short_f() {
        let args: Vec<String> = vec!["-F".into(), "json".into()];
        assert!(has_json_output_flag(&args));
    }

    #[test]
    fn test_has_json_flag_long_output() {
        let args: Vec<String> = vec!["--output".into(), "json".into()];
        assert!(has_json_output_flag(&args));
    }

    #[test]
    fn test_has_json_flag_short_o() {
        let args: Vec<String> = vec!["-O".into(), "json".into()];
        assert!(has_json_output_flag(&args));
    }

    #[test]
    fn test_has_json_flag_equals_form() {
        let args: Vec<String> = vec!["--output=json".into()];
        assert!(has_json_output_flag(&args));
    }

    #[test]
    fn test_has_json_flag_absent() {
        let args: Vec<String> = vec!["list".into(), "--all".into()];
        assert!(!has_json_output_flag(&args));
    }

    #[test]
    fn test_has_json_flag_text_output() {
        let args: Vec<String> = vec!["--output".into(), "text".into()];
        assert!(!has_json_output_flag(&args));
    }

    // ----- extract_identifier_and_extra_args --------------------------------

    #[test]
    fn test_extract_id_simple() {
        let args: Vec<String> = vec!["45".into()];
        let (id, extra) = extract_identifier_and_extra_args(&args).unwrap();
        assert_eq!(id, "45");
        assert!(extra.is_empty());
    }

    #[test]
    fn test_extract_id_with_repo_flag_after() {
        let args: Vec<String> = vec!["45".into(), "-R".into(), "group/project".into()];
        let (id, extra) = extract_identifier_and_extra_args(&args).unwrap();
        assert_eq!(id, "45");
        assert_eq!(extra, vec!["-R", "group/project"]);
    }

    #[test]
    fn test_extract_id_with_repo_flag_before() {
        let args: Vec<String> = vec!["-R".into(), "group/project".into(), "45".into()];
        let (id, extra) = extract_identifier_and_extra_args(&args).unwrap();
        assert_eq!(id, "45");
        assert_eq!(extra, vec!["-R", "group/project"]);
    }

    #[test]
    fn test_extract_id_branch_name() {
        let args: Vec<String> = vec!["fix/oauth2-token-refresh".into()];
        let (id, extra) = extract_identifier_and_extra_args(&args).unwrap();
        assert_eq!(id, "fix/oauth2-token-refresh");
        assert!(extra.is_empty());
    }

    #[test]
    fn test_extract_id_empty() {
        let args: Vec<String> = vec![];
        assert!(extract_identifier_and_extra_args(&args).is_none());
    }

    #[test]
    fn test_extract_id_only_flags() {
        let args: Vec<String> = vec!["-R".into(), "group/project".into()];
        // -R consumes the next token so there's no identifier left
        assert!(extract_identifier_and_extra_args(&args).is_none());
    }

    #[test]
    fn test_extract_id_with_web_flag() {
        let args: Vec<String> = vec!["45".into(), "--web".into()];
        let (id, extra) = extract_identifier_and_extra_args(&args).unwrap();
        assert_eq!(id, "45");
        assert_eq!(extra, vec!["--web"]);
    }

    // ----- filter_mr_list --------------------------------------------------

    #[test]
    fn test_mr_list_essentials_present() {
        let out = filter_mr_list(MR_LIST_JSON, false);
        // MR numbers
        assert!(out.contains("!45"), "should contain !45");
        assert!(out.contains("!44"), "should contain !44");
        assert!(out.contains("!43"), "should contain !43");
        // Authors
        assert!(out.contains("@jdoe"), "should contain @jdoe");
        assert!(out.contains("@alice"), "should contain @alice");
        assert!(out.contains("@bob"), "should contain @bob");
    }

    #[test]
    fn test_mr_list_state_icons() {
        let out = filter_mr_list(MR_LIST_JSON, false);
        // opened non-draft → O, opened draft → D, merged → M
        assert!(out.contains(" O "), "opened MR should show O");
        assert!(out.contains(" D "), "draft MR should show D");
        assert!(out.contains(" M "), "merged MR should show M");
    }

    #[test]
    fn test_mr_list_ci_tags() {
        let out = filter_mr_list(MR_LIST_JSON, false);
        assert!(out.contains("[CI:run]"), "running CI should show [CI:run]");
        assert!(out.contains("[CI:fail]"), "failed CI should show [CI:fail]");
        assert!(out.contains("[CI:ok]"), "succeeded CI should show [CI:ok]");
    }

    #[test]
    fn test_mr_list_excludes_verbose_fields() {
        let out = filter_mr_list(MR_LIST_JSON, false);
        // Raw JSON fields that should NOT appear in filtered output
        assert!(!out.contains("\"web_url\""), "should not contain JSON keys");
        assert!(!out.contains("updated_at"), "should not contain updated_at");
        assert!(!out.contains("\"id\""), "should not contain raw id key");
    }

    #[test]
    fn test_mr_list_ultra_compact_header() {
        let compact = filter_mr_list(MR_LIST_JSON, true);
        let normal = filter_mr_list(MR_LIST_JSON, false);
        assert!(compact.starts_with("MRs\n"));
        assert!(normal.starts_with("Merge Requests\n"));
    }

    #[test]
    fn test_mr_list_empty_json() {
        let out = filter_mr_list("[]", false);
        assert_eq!(out, "MRs: none\n");
    }

    #[test]
    fn test_mr_list_invalid_json() {
        let out = filter_mr_list("not json", false);
        assert_eq!(out, "");
    }

    #[test]
    fn test_mr_list_token_savings() {
        let filtered = filter_mr_list(MR_LIST_JSON, false);
        let input_tokens = count_tokens(MR_LIST_JSON);
        let output_tokens = count_tokens(&filtered);
        let savings = 100.0 - (output_tokens as f64 / input_tokens as f64 * 100.0);
        assert!(
            savings >= 60.0,
            "mr list: expected >=60% token savings, got {:.1}% (in={} out={})",
            savings,
            input_tokens,
            output_tokens
        );
    }

    // ----- filter_mr_view --------------------------------------------------

    #[test]
    fn test_mr_view_essentials_present() {
        let out = filter_mr_view(MR_VIEW_JSON, false);
        assert!(out.contains("!45"), "should contain MR iid");
        assert!(out.contains("@jdoe"), "should contain author");
        assert!(
            out.contains("fix/oauth2-token-refresh"),
            "should contain source branch"
        );
        assert!(out.contains("main"), "should contain target branch");
        assert!(out.contains("https://gitlab.com"), "should contain URL");
    }

    #[test]
    fn test_mr_view_state_and_merge_status() {
        let out = filter_mr_view(MR_VIEW_JSON, false);
        assert!(out.contains("opened"), "should show state");
        assert!(
            out.contains("mergeable"),
            "should show merge_status=can_be_merged as mergeable"
        );
    }

    #[test]
    fn test_mr_view_pipeline_info() {
        let out = filter_mr_view(MR_VIEW_JSON, false);
        assert!(out.contains("CI:"), "should contain CI label");
        assert!(out.contains("running"), "should show pipeline status");
        assert!(out.contains("789"), "should show pipeline id");
    }

    #[test]
    fn test_mr_view_reviewers() {
        let out = filter_mr_view(MR_VIEW_JSON, false);
        assert!(out.contains("alice"), "should list reviewer alice");
        assert!(out.contains("bob"), "should list reviewer bob");
    }

    #[test]
    fn test_mr_view_description_included_normal() {
        let out = filter_mr_view(MR_VIEW_JSON, false);
        // Description should be partially included (first 5 non-empty lines)
        assert!(out.contains("---"), "should include description separator");
        assert!(
            out.contains("Summary"),
            "should include first line of description"
        );
    }

    #[test]
    fn test_mr_view_description_excluded_ultra_compact() {
        let out = filter_mr_view(MR_VIEW_JSON, true);
        // In ultra_compact mode, description is omitted entirely
        assert!(
            !out.contains("Rolling back"),
            "ultra_compact should omit description"
        );
        assert!(
            !out.contains("Coverage"),
            "ultra_compact should omit coverage text"
        );
    }

    #[test]
    fn test_mr_view_invalid_json() {
        let out = filter_mr_view("not json", false);
        assert_eq!(out, "");
    }

    #[test]
    fn test_mr_view_token_savings() {
        let filtered = filter_mr_view(MR_VIEW_JSON, false);
        let input_tokens = count_tokens(MR_VIEW_JSON);
        let output_tokens = count_tokens(&filtered);
        let savings = 100.0 - (output_tokens as f64 / input_tokens as f64 * 100.0);
        assert!(
            savings >= 60.0,
            "mr view: expected >=60% token savings, got {:.1}% (in={} out={})",
            savings,
            input_tokens,
            output_tokens
        );
    }

    #[test]
    fn test_mr_view_ultra_compact_token_savings() {
        let filtered = filter_mr_view(MR_VIEW_JSON, true);
        let input_tokens = count_tokens(MR_VIEW_JSON);
        let output_tokens = count_tokens(&filtered);
        let savings = 100.0 - (output_tokens as f64 / input_tokens as f64 * 100.0);
        assert!(
            savings >= 75.0,
            "mr view ultra_compact: expected >=75% savings, got {:.1}%",
            savings
        );
    }

    // ----- filter_ci_list --------------------------------------------------

    #[test]
    fn test_ci_list_essentials_present() {
        let out = filter_ci_list(CI_LIST_JSON, false);
        assert!(out.contains("#123456"), "should contain pipeline 123456");
        assert!(out.contains("#123455"), "should contain pipeline 123455");
        assert!(out.contains("#123454"), "should contain pipeline 123454");
        assert!(out.contains("main"), "should contain ref name");
    }

    #[test]
    fn test_ci_list_status_tags() {
        let out = filter_ci_list(CI_LIST_JSON, false);
        assert!(out.contains("[ok]"), "success should show [ok]");
        assert!(out.contains("[FAIL]"), "failed should show [FAIL]");
        assert!(out.contains("[run]"), "running should show [run]");
    }

    #[test]
    fn test_ci_list_sha_shortened() {
        let out = filter_ci_list(CI_LIST_JSON, false);
        // SHA in fixture is 40 chars; we truncate to 8
        assert!(
            out.contains("a1b2c3d4"),
            "should contain first 8 chars of SHA"
        );
        // The full SHA should not appear
        assert!(
            !out.contains("a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2"),
            "should not print full SHA"
        );
    }

    #[test]
    fn test_ci_list_empty() {
        let out = filter_ci_list("[]", false);
        assert_eq!(out, "Pipelines: none\n");
    }

    #[test]
    fn test_ci_list_invalid_json() {
        let out = filter_ci_list("{}", false);
        // Top-level object, not array → empty
        assert_eq!(out, "");
    }

    #[test]
    fn test_ci_list_token_savings() {
        let filtered = filter_ci_list(CI_LIST_JSON, false);
        let input_tokens = count_tokens(CI_LIST_JSON);
        let output_tokens = count_tokens(&filtered);
        let savings = 100.0 - (output_tokens as f64 / input_tokens as f64 * 100.0);
        assert!(
            savings >= 60.0,
            "ci list: expected >=60% savings, got {:.1}% (in={} out={})",
            savings,
            input_tokens,
            output_tokens
        );
    }

    // ----- filter_ci_status ------------------------------------------------

    #[test]
    fn test_ci_status_essentials_present() {
        let out = filter_ci_status(CI_STATUS_JSON, false);
        assert!(out.contains("123455"), "should contain pipeline id");
        assert!(out.contains("failed"), "should show failed status");
        assert!(out.contains("fix/oauth2-token-refresh"), "should show ref");
        assert!(out.contains("https://gitlab.com"), "should show URL");
    }

    #[test]
    fn test_ci_status_stages() {
        let out = filter_ci_status(CI_STATUS_JSON, false);
        assert!(out.contains("build"), "should list build stage");
        assert!(out.contains("test"), "should list test stage");
        assert!(out.contains("deploy"), "should list deploy stage");
    }

    #[test]
    fn test_ci_status_failed_jobs() {
        let out = filter_ci_status(CI_STATUS_JSON, false);
        assert!(
            out.contains("test:unit"),
            "should list failed job test:unit"
        );
        assert!(
            out.contains("test:integration"),
            "should list failed job test:integration"
        );
        // Successful job should NOT appear in failed list
        assert!(
            !out.contains("build:docker"),
            "successful job should not appear in failed list"
        );
    }

    #[test]
    fn test_ci_status_accepts_array_response() {
        // Some glab versions wrap the result in an array
        let json_array = format!("[{}]", CI_STATUS_JSON);
        let out = filter_ci_status(&json_array, false);
        assert!(
            out.contains("123455"),
            "array form: should still extract pipeline id"
        );
    }

    #[test]
    fn test_ci_status_empty_array() {
        let out = filter_ci_status("[]", false);
        assert_eq!(out, "Pipelines: none\n");
    }

    #[test]
    fn test_ci_status_token_savings() {
        let filtered = filter_ci_status(CI_STATUS_JSON, false);
        let input_tokens = count_tokens(CI_STATUS_JSON);
        let output_tokens = count_tokens(&filtered);
        let savings = 100.0 - (output_tokens as f64 / input_tokens as f64 * 100.0);
        assert!(
            savings >= 60.0,
            "ci status: expected >=60% savings, got {:.1}% (in={} out={})",
            savings,
            input_tokens,
            output_tokens
        );
    }

    // ----- filter_issue_list -----------------------------------------------

    #[test]
    fn test_issue_list_essentials_present() {
        let out = filter_issue_list(ISSUE_LIST_JSON, false);
        assert!(out.contains("#101"), "should contain issue 101");
        assert!(out.contains("#100"), "should contain issue 100");
        assert!(out.contains("#99"), "should contain issue 99");
        assert!(out.contains("@jdoe"), "should contain author jdoe");
        assert!(out.contains("@alice"), "should contain author alice");
    }

    #[test]
    fn test_issue_list_state_icons() {
        let out = filter_issue_list(ISSUE_LIST_JSON, false);
        // Issues 101 and 100 are opened (O), issue 99 is closed (C)
        assert!(out.contains(" O "), "opened issues should show O");
        assert!(out.contains(" C "), "closed issues should show C");
    }

    #[test]
    fn test_issue_list_labels() {
        let out = filter_issue_list(ISSUE_LIST_JSON, false);
        assert!(out.contains("bug"), "should include bug label");
        assert!(out.contains("backend"), "should include backend label");
        assert!(
            out.contains("documentation"),
            "should include documentation label"
        );
    }

    #[test]
    fn test_issue_list_excludes_json_bloat() {
        let out = filter_issue_list(ISSUE_LIST_JSON, false);
        assert!(
            !out.contains("\"web_url\""),
            "should not contain raw JSON keys"
        );
        assert!(!out.contains("created_at"), "should not contain created_at");
    }

    #[test]
    fn test_issue_list_empty() {
        let out = filter_issue_list("[]", false);
        assert_eq!(out, "Issues: none\n");
    }

    #[test]
    fn test_issue_list_invalid_json() {
        let out = filter_issue_list("malformed{", false);
        assert_eq!(out, "");
    }

    #[test]
    fn test_issue_list_token_savings() {
        let filtered = filter_issue_list(ISSUE_LIST_JSON, false);
        let input_tokens = count_tokens(ISSUE_LIST_JSON);
        let output_tokens = count_tokens(&filtered);
        let savings = 100.0 - (output_tokens as f64 / input_tokens as f64 * 100.0);
        assert!(
            savings >= 60.0,
            "issue list: expected >=60% savings, got {:.1}% (in={} out={})",
            savings,
            input_tokens,
            output_tokens
        );
    }

    // ----- guard integration -----------------------------------------------

    #[test]
    fn test_never_worse_keeps_filtered_when_smaller() {
        let filtered = filter_mr_list(MR_LIST_JSON, false);
        // filtered must be smaller (in tokens) than the raw JSON
        assert!(
            count_tokens(&filtered) < count_tokens(MR_LIST_JSON),
            "filtered MR list should be smaller than raw JSON"
        );
        let chosen = guard::never_worse(MR_LIST_JSON, &filtered);
        assert_eq!(
            chosen,
            filtered.as_str(),
            "guard should keep filtered output"
        );
    }

    #[test]
    fn test_never_worse_falls_back_for_inflated_output() {
        let raw = "ok";
        let inflated = "This is a much longer output that is bigger than the raw command output and should trigger the fallback";
        let chosen = guard::never_worse(raw, inflated);
        assert_eq!(
            chosen, raw,
            "guard should fall back to raw when filter inflates"
        );
    }
}
