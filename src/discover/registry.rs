use lazy_static::lazy_static;
use regex::{Regex, RegexSet};

/// A rule mapping a shell command pattern to its RTK equivalent.
struct RtkRule {
    rtk_cmd: &'static str,
    category: &'static str,
    savings_pct: f64,
    subcmd_savings: &'static [(&'static str, f64)],
    subcmd_status: &'static [(&'static str, super::report::RtkStatus)],
}

/// Result of classifying a command.
#[derive(Debug, PartialEq)]
pub enum Classification {
    Supported {
        rtk_equivalent: &'static str,
        category: &'static str,
        estimated_savings_pct: f64,
        status: super::report::RtkStatus,
    },
    Unsupported {
        base_command: String,
    },
    Ignored,
}

/// Average token counts per category for estimation when no output_len available.
pub fn category_avg_tokens(category: &str, subcmd: &str) -> usize {
    match category {
        "Git" => match subcmd {
            "log" | "diff" | "show" => 200,
            _ => 40,
        },
        "Cargo" => match subcmd {
            "test" => 500,
            _ => 150,
        },
        "Tests" => 800,
        "Files" => 100,
        "Build" => 300,
        "Infra" => 120,
        "Network" => 150,
        "GitHub" => 200,
        "PackageManager" => 150,
        _ => 150,
    }
}

// Patterns ordered to match RTK_RULES indices exactly.
const PATTERNS: &[&str] = &[
    r"^git\s+(status|log|diff|show|add|commit|push|pull|branch|fetch|stash|worktree)",
    r"^gh\s+(pr|issue|run|repo|api)",
    r"^cargo\s+(build|test|clippy|check|fmt)",
    r"^pnpm\s+(list|ls|outdated|install)",
    r"^npm\s+(run|exec)",
    r"^npx\s+",
    r"^(cat|head|tail)\s+",
    r"^(rg|grep)\s+",
    r"^ls(\s|$)",
    r"^find\s+",
    r"^(npx\s+|pnpm\s+)?tsc(\s|$)",
    r"^(npx\s+|pnpm\s+)?(eslint|biome|lint)(\s|$)",
    r"^(npx\s+|pnpm\s+)?prettier",
    r"^(npx\s+|pnpm\s+)?next\s+build",
    r"^(pnpm\s+|npx\s+)?(vitest|jest|test)(\s|$)",
    r"^(npx\s+|pnpm\s+)?playwright",
    r"^(npx\s+|pnpm\s+)?prisma",
    r"^docker\s+(ps|images|logs)",
    r"^kubectl\s+(get|logs)",
    r"^(python3?\s+-m\s+)?mypy(\s|$)",
    r"^curl\s+",
    r"^wget\s+",
    r"^terragrunt\s+(plan|apply|init|output|validate|state)",
    // git subcommands RTK forwards via passthrough (no dedicated filter, 0% savings)
    r"^git\s+(checkout|switch|restore|rebase|merge|reset|tag|clone|cherry-pick|revert|rm|mv|remote|config|describe|blame|apply|clean|rev-parse|reflog|ls-files|init|bisect|submodule)",
    // kubectl subcommands RTK forwards via passthrough (get/logs have filters above)
    r"^kubectl\s+(describe|apply|delete|rollout|scale|config|top|patch|expose|create|cordon|drain|explain|version|api-resources|cluster-info|label|annotate|wait|auth)",
    // glab: mr/ci/issue have compact filters, other subcommands pass through
    r"^glab\s+(mr|ci|issue|api|release|repo|pipeline|auth|variable|cluster)",
];

const RULES: &[RtkRule] = &[
    RtkRule {
        rtk_cmd: "rtk git",
        category: "Git",
        savings_pct: 70.0,
        subcmd_savings: &[
            ("diff", 80.0),
            ("show", 80.0),
            ("add", 59.0),
            ("commit", 59.0),
        ],
        subcmd_status: &[],
    },
    RtkRule {
        rtk_cmd: "rtk gh",
        category: "GitHub",
        savings_pct: 82.0,
        subcmd_savings: &[("pr", 87.0), ("run", 82.0), ("issue", 80.0)],
        subcmd_status: &[],
    },
    RtkRule {
        rtk_cmd: "rtk cargo",
        category: "Cargo",
        savings_pct: 80.0,
        subcmd_savings: &[("test", 90.0), ("check", 80.0)],
        subcmd_status: &[("fmt", super::report::RtkStatus::Passthrough)],
    },
    RtkRule {
        rtk_cmd: "rtk pnpm",
        category: "PackageManager",
        savings_pct: 80.0,
        subcmd_savings: &[],
        subcmd_status: &[],
    },
    RtkRule {
        rtk_cmd: "rtk npm",
        category: "PackageManager",
        savings_pct: 70.0,
        subcmd_savings: &[],
        subcmd_status: &[],
    },
    RtkRule {
        rtk_cmd: "rtk npx",
        category: "PackageManager",
        savings_pct: 70.0,
        subcmd_savings: &[],
        subcmd_status: &[],
    },
    RtkRule {
        rtk_cmd: "rtk read",
        category: "Files",
        savings_pct: 60.0,
        subcmd_savings: &[],
        subcmd_status: &[],
    },
    RtkRule {
        rtk_cmd: "rtk grep",
        category: "Files",
        savings_pct: 75.0,
        subcmd_savings: &[],
        subcmd_status: &[],
    },
    RtkRule {
        rtk_cmd: "rtk ls",
        category: "Files",
        savings_pct: 65.0,
        subcmd_savings: &[],
        subcmd_status: &[],
    },
    RtkRule {
        rtk_cmd: "rtk find",
        category: "Files",
        savings_pct: 70.0,
        subcmd_savings: &[],
        subcmd_status: &[],
    },
    RtkRule {
        rtk_cmd: "rtk tsc",
        category: "Build",
        savings_pct: 83.0,
        subcmd_savings: &[],
        subcmd_status: &[],
    },
    RtkRule {
        rtk_cmd: "rtk lint",
        category: "Build",
        savings_pct: 84.0,
        subcmd_savings: &[],
        subcmd_status: &[],
    },
    RtkRule {
        rtk_cmd: "rtk prettier",
        category: "Build",
        savings_pct: 70.0,
        subcmd_savings: &[],
        subcmd_status: &[],
    },
    RtkRule {
        rtk_cmd: "rtk next",
        category: "Build",
        savings_pct: 87.0,
        subcmd_savings: &[],
        subcmd_status: &[],
    },
    RtkRule {
        rtk_cmd: "rtk vitest",
        category: "Tests",
        savings_pct: 99.0,
        subcmd_savings: &[],
        subcmd_status: &[],
    },
    RtkRule {
        rtk_cmd: "rtk playwright",
        category: "Tests",
        savings_pct: 94.0,
        subcmd_savings: &[],
        subcmd_status: &[],
    },
    RtkRule {
        rtk_cmd: "rtk prisma",
        category: "Build",
        savings_pct: 88.0,
        subcmd_savings: &[],
        subcmd_status: &[],
    },
    RtkRule {
        rtk_cmd: "rtk docker",
        category: "Infra",
        savings_pct: 85.0,
        subcmd_savings: &[],
        subcmd_status: &[],
    },
    RtkRule {
        rtk_cmd: "rtk kubectl",
        category: "Infra",
        savings_pct: 85.0,
        subcmd_savings: &[],
        subcmd_status: &[],
    },
    RtkRule {
        rtk_cmd: "rtk mypy",
        category: "Build",
        savings_pct: 80.0,
        subcmd_savings: &[],
        subcmd_status: &[],
    },
    RtkRule {
        rtk_cmd: "rtk curl",
        category: "Network",
        savings_pct: 70.0,
        subcmd_savings: &[],
        subcmd_status: &[],
    },
    RtkRule {
        rtk_cmd: "rtk wget",
        category: "Network",
        savings_pct: 65.0,
        subcmd_savings: &[],
        subcmd_status: &[],
    },
    RtkRule {
        rtk_cmd: "rtk terragrunt",
        category: "Infra",
        savings_pct: 80.0,
        subcmd_savings: &[("plan", 85.0), ("apply", 85.0)],
        subcmd_status: &[("state", super::report::RtkStatus::Passthrough)],
    },
    // git passthrough subcommands: 0% savings -> Passthrough status. Classified so
    // they leave the "unhandled" list and get rewritten/tracked as `rtk git`.
    RtkRule {
        rtk_cmd: "rtk git",
        category: "Git",
        savings_pct: 0.0,
        subcmd_savings: &[],
        subcmd_status: &[],
    },
    // kubectl passthrough subcommands: 0% savings -> Passthrough status.
    RtkRule {
        rtk_cmd: "rtk kubectl",
        category: "Infra",
        savings_pct: 0.0,
        subcmd_savings: &[],
        subcmd_status: &[],
    },
    // glab: mr/ci/issue have compact filters (Existing); other subcommands pass
    // through (0% -> Passthrough via the default-status heuristic).
    RtkRule {
        rtk_cmd: "rtk glab",
        category: "GitHub",
        savings_pct: 0.0,
        subcmd_savings: &[("mr", 80.0), ("ci", 75.0), ("issue", 80.0)],
        subcmd_status: &[],
    },
];

/// Commands to ignore (shell builtins, trivial, already rtk).
const IGNORED_PREFIXES: &[&str] = &[
    "cd ",
    "cd\t",
    "echo ",
    "printf ",
    "export ",
    "source ",
    "mkdir ",
    "rm ",
    "mv ",
    "cp ",
    "chmod ",
    "chown ",
    "touch ",
    "which ",
    "type ",
    "command ",
    "test ",
    "true",
    "false",
    "sleep ",
    "wait",
    "kill ",
    "set ",
    "unset ",
    "wc ",
    "sort ",
    "uniq ",
    "tr ",
    "cut ",
    "awk ",
    "sed ",
    "python3 -c",
    "python -c",
    "node -e",
    "ruby -e",
    "rtk ",
    "pwd",
    "bash ",
    "sh ",
    "then\n",
    "then ",
    "else\n",
    "else ",
    "do\n",
    "do ",
    "for ",
    "while ",
    "if ",
    "case ",
];

const IGNORED_EXACT: &[&str] = &[
    "cd", "echo", "true", "false", "wait", "pwd", "bash", "sh",
    // Shell keywords that must match exactly, not as prefixes ("fi" would shadow "find")
    "fi", "done",
];

lazy_static! {
    static ref REGEX_SET: RegexSet = RegexSet::new(PATTERNS).expect("invalid regex patterns");
    static ref COMPILED: Vec<Regex> = PATTERNS
        .iter()
        .map(|p| Regex::new(p).expect("invalid regex"))
        .collect();
    static ref ENV_PREFIX: Regex =
        Regex::new(r"^(?:sudo\s+|env\s+|[A-Z_][A-Z0-9_]*=[^\s]*\s+)+").unwrap();
    // Git global options that appear before the subcommand: -C <path>, -c <key=val>,
    // --git-dir <dir>, --work-tree <dir>, and flag-only options.
    // Ported from upstream rtk-ai/rtk (#163: `git -C /tmp status` -> `git status`).
    static ref GIT_GLOBAL_OPT: Regex = Regex::new(
        r"^(?:(?:-C\s+\S+|-c\s+\S+|--git-dir(?:=\S+|\s+\S+)|--work-tree(?:=\S+|\s+\S+)|--no-pager|--no-optional-locks|--bare|--literal-pathspecs)\s+)+"
    ).unwrap();
    // Kubectl global options that appear before the subcommand: --context, -n/--namespace,
    // --kubeconfig, --cluster, --user, --as, --server/-s, --token, --request-timeout, and
    // the boolean --insecure-skip-tls-verify. Net-new (missing in upstream too): normalizes
    // `kubectl --context x -n y get pods` -> `kubectl get pods` for classification.
    static ref KUBECTL_GLOBAL_OPT: Regex = Regex::new(
        r"^(?:(?:--context(?:=\S+|\s+\S+)|(?:-n|--namespace)(?:=\S+|\s+\S+)|--kubeconfig(?:=\S+|\s+\S+)|--cluster(?:=\S+|\s+\S+)|--user(?:=\S+|\s+\S+)|--as(?:=\S+|\s+\S+)|(?:-s|--server)(?:=\S+|\s+\S+)|--token(?:=\S+|\s+\S+)|--request-timeout(?:=\S+|\s+\S+)|--insecure-skip-tls-verify)\s+)+"
    ).unwrap();
}

/// Classify a single (already-split) command.
pub fn classify_command(cmd: &str) -> Classification {
    // Unwrap noise wrappers before anything else: a leading `\` line-continuation
    // artifact and `$(which foo)` / `` `which foo` `` command substitution.
    let unwrapped = strip_leading_continuation(cmd);
    let unwrapped = unwrap_command_substitution(&unwrapped);
    let trimmed = unwrapped.trim();
    if trimmed.is_empty() {
        return Classification::Ignored;
    }

    // Check ignored
    for exact in IGNORED_EXACT {
        if trimmed == *exact {
            return Classification::Ignored;
        }
    }
    for prefix in IGNORED_PREFIXES {
        if trimmed.starts_with(prefix) {
            return Classification::Ignored;
        }
    }

    // Strip env prefixes (sudo, env VAR=val, VAR=val)
    let stripped = ENV_PREFIX.replace(trimmed, "");
    let cmd_clean = stripped.trim();
    if cmd_clean.is_empty() {
        return Classification::Ignored;
    }

    // Normalize before matching so wrapped invocations still classify:
    //   /usr/bin/git status         -> git status            (abs path, upstream #485)
    //   git -C /tmp status          -> git status            (git global opts, upstream #163)
    //   kubectl --context x get pods-> kubectl get pods      (kubectl global opts, net-new)
    let cmd_normalized = strip_absolute_path(cmd_clean);
    let cmd_normalized = strip_git_global_opts(&cmd_normalized);
    let cmd_normalized = strip_kubectl_global_opts(&cmd_normalized);
    let cmd_clean = cmd_normalized.as_str();

    // Fast check with RegexSet — take the last (most specific) match
    let matches: Vec<usize> = REGEX_SET.matches(cmd_clean).into_iter().collect();
    if let Some(&idx) = matches.last() {
        let rule = &RULES[idx];

        // Extract subcommand for savings override and status detection
        let (savings, status) = if let Some(caps) = COMPILED[idx].captures(cmd_clean) {
            if let Some(sub) = caps.get(1) {
                let subcmd = sub.as_str();

                // Check if this subcommand has custom savings
                let savings = rule
                    .subcmd_savings
                    .iter()
                    .find(|(s, _)| *s == subcmd)
                    .map(|(_, pct)| *pct)
                    .unwrap_or(rule.savings_pct);

                // An explicit override wins; otherwise a zero-savings subcommand is
                // one RTK only passes through (no dedicated filter).
                let status = rule
                    .subcmd_status
                    .iter()
                    .find(|(s, _)| *s == subcmd)
                    .map(|(_, st)| *st)
                    .unwrap_or_else(|| default_status_for(savings));

                (savings, status)
            } else {
                (rule.savings_pct, default_status_for(rule.savings_pct))
            }
        } else {
            (rule.savings_pct, default_status_for(rule.savings_pct))
        };

        Classification::Supported {
            rtk_equivalent: rule.rtk_cmd,
            category: rule.category,
            estimated_savings_pct: savings,
            status,
        }
    } else {
        // Extract base command for unsupported
        let base = extract_base_command(cmd_clean);
        if base.is_empty() {
            Classification::Ignored
        } else {
            Classification::Unsupported {
                base_command: base.to_string(),
            }
        }
    }
}

/// Default RTK status for a subcommand given its estimated savings: a command RTK
/// only forwards (no dedicated filter) has zero savings and is a passthrough.
fn default_status_for(savings_pct: f64) -> super::report::RtkStatus {
    if savings_pct <= 0.0 {
        super::report::RtkStatus::Passthrough
    } else {
        super::report::RtkStatus::Existing
    }
}

/// Strip a leading `\` line-continuation artifact (`\<newline>git ...`, `\ git`).
///
/// Multi-line commands recorded in Claude Code history sometimes begin with a
/// stray backslash from shell line continuation. `\ls`/`\command` (alias bypass)
/// normalize the same way, which is also correct.
fn strip_leading_continuation(cmd: &str) -> String {
    let t = cmd.trim_start();
    match t.strip_prefix('\\') {
        Some(rest) => rest.trim_start().to_string(),
        None => cmd.to_string(),
    }
}

/// Unwrap `$(which foo) ...` / `` `which foo` ... `` into `foo ...`.
///
/// Only the leading command-substitution form is handled (the command itself is
/// resolved via `which`); substitutions used as arguments are left untouched.
fn unwrap_command_substitution(cmd: &str) -> String {
    let c = cmd.trim_start();
    for (open, close) in [("$(which ", ")"), ("`which ", "`")] {
        if let Some(rest) = c.strip_prefix(open) {
            if let Some(close_idx) = rest.find(close) {
                let name = rest[..close_idx].trim();
                // `which` yields a path or bare name; keep just the basename.
                let name = name.rsplit('/').next().unwrap_or(name);
                let after = &rest[close_idx + close.len()..];
                if !name.is_empty() {
                    return format!("{}{}", name, after);
                }
            }
        }
    }
    cmd.to_string()
}

/// Normalize an absolute binary path to its basename: `/usr/bin/grep` -> `grep`.
///
/// Ported from upstream rtk-ai/rtk (#485).
fn strip_absolute_path(cmd: &str) -> String {
    let first_space = cmd.find(' ');
    let first_word = match first_space {
        Some(pos) => &cmd[..pos],
        None => cmd,
    };
    if first_word.contains('/') {
        let basename = first_word.rsplit('/').next().unwrap_or(first_word);
        if basename.is_empty() {
            return cmd.to_string();
        }
        match first_space {
            Some(pos) => format!("{}{}", basename, &cmd[pos..]),
            None => basename.to_string(),
        }
    } else {
        cmd.to_string()
    }
}

/// Strip git global options that appear before the subcommand.
///
/// Ported from upstream rtk-ai/rtk (#163): `git -C /tmp status` -> `git status`.
fn strip_git_global_opts(cmd: &str) -> String {
    if !cmd.starts_with("git ") {
        return cmd.to_string();
    }
    let after_git = &cmd[4..]; // skip "git "
    let stripped = GIT_GLOBAL_OPT.replace(after_git, "");
    format!("git {}", stripped.trim())
}

/// Strip kubectl global options that appear before the subcommand.
///
/// Net-new (missing upstream): `kubectl --context x -n y get pods` -> `kubectl get pods`.
fn strip_kubectl_global_opts(cmd: &str) -> String {
    if !cmd.starts_with("kubectl ") {
        return cmd.to_string();
    }
    let after = &cmd["kubectl ".len()..];
    let stripped = KUBECTL_GLOBAL_OPT.replace(after, "");
    format!("kubectl {}", stripped.trim())
}

/// Extract the base command (first word, or first two if it looks like a subcommand pattern).
fn extract_base_command(cmd: &str) -> &str {
    let parts: Vec<&str> = cmd.splitn(3, char::is_whitespace).collect();
    match parts.len() {
        0 => "",
        1 => parts[0],
        _ => {
            let second = parts[1];
            // If the second token looks like a subcommand (no leading -)
            if !second.starts_with('-') && !second.contains('/') && !second.contains('.') {
                // Return "cmd subcmd"
                let end = cmd
                    .find(char::is_whitespace)
                    .and_then(|i| {
                        let rest = &cmd[i..];
                        let trimmed = rest.trim_start();
                        trimmed
                            .find(char::is_whitespace)
                            .map(|j| i + (rest.len() - trimmed.len()) + j)
                    })
                    .unwrap_or(cmd.len());
                &cmd[..end]
            } else {
                parts[0]
            }
        }
    }
}

/// Rewrite a command to use RTK if it's supported.
/// Returns Some(rewritten) if the command should be rewritten, None otherwise.
/// Respects excluded commands from config.
/// Check if a cat command has flags incompatible with rtk read.
/// cat -n (line numbers), -A (show-all), and similar flags are not supported.
fn has_incompatible_cat_flags(cmd: &str) -> bool {
    // Incompatible flags that rtk read doesn't support
    const INCOMPATIBLE_FLAGS: &[&str] = &[
        "-n",
        "--number",
        "-A",
        "--show-all",
        "-b",
        "--number-nonblank",
        "-e",
        "-E",
        "--show-ends",
        "-s",
        "--squeeze-blank",
        "-T",
        "--show-tabs",
        "-t",
        "-v",
        "--show-nonprinting",
    ];

    let words: Vec<&str> = cmd.split_whitespace().collect();
    for word in words.iter().skip(1) {
        // Skip the "cat" command itself
        if INCOMPATIBLE_FLAGS.contains(word) {
            return true;
        }
    }
    false
}

pub fn rewrite_command(cmd: &str, excluded: &[String]) -> Option<String> {
    // Unwrap the same noise wrappers classify_command handles, so the rewritten
    // form is clean: `$(which git) push` -> `rtk git push`, `\ git ...` -> `rtk git ...`.
    let unwrapped = strip_leading_continuation(cmd);
    let unwrapped = unwrap_command_substitution(&unwrapped);
    let trimmed = unwrapped.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Check if command is excluded
    for excl in excluded {
        if trimmed.starts_with(excl.as_str()) {
            return None;
        }
    }

    // Skip cat rewrite if incompatible flags are present
    if trimmed.starts_with("cat ") && has_incompatible_cat_flags(trimmed) {
        return None;
    }

    // Classify the command
    match classify_command(trimmed) {
        Classification::Supported { rtk_equivalent, .. } => {
            // Extract the base command to replace. Normalize an absolute binary
            // path (`/usr/bin/git` -> `git`) but DELIBERATELY keep git/kubectl
            // global flags (`-C <path>`, `--context <x>`) so the rewritten command
            // still targets the right repo/context.
            let stripped = ENV_PREFIX.replace(trimmed, "");
            let cmd_clean = strip_absolute_path(stripped.trim());
            let cmd_clean = cmd_clean.as_str();

            // Find the command part to replace
            // Simple approach: replace the first word with rtk equivalent
            let parts: Vec<&str> = cmd_clean.splitn(2, char::is_whitespace).collect();
            if parts.is_empty() {
                return None;
            }

            // Handle compound commands (npx tsc, pnpm vitest, etc.)
            let rewritten = if cmd_clean.starts_with("npx ") || cmd_clean.starts_with("pnpm ") {
                // For npx/pnpm commands, strip the package manager prefix and tool name
                if parts.len() >= 2 {
                    // parts[1] is "tsc --noEmit" - we need to split again to skip "tsc"
                    let tool_and_args: Vec<&str> =
                        parts[1].splitn(2, char::is_whitespace).collect();
                    if tool_and_args.len() > 1 {
                        // Has args after the tool name
                        format!("{} {}", rtk_equivalent, tool_and_args[1])
                    } else {
                        // No args after the tool name
                        rtk_equivalent.to_string()
                    }
                } else {
                    rtk_equivalent.to_string()
                }
            } else {
                // For regular commands, keep everything after the base command
                if parts.len() > 1 {
                    format!("{} {}", rtk_equivalent, parts[1])
                } else {
                    rtk_equivalent.to_string()
                }
            };

            Some(rewritten)
        }
        Classification::Unsupported { .. } | Classification::Ignored => None,
    }
}

/// Split a command chain on `&&`, `||`, `;` outside quotes.
/// For pipes `|`, only keep the first command.
/// Lines with `<<` (heredoc) or `$((` are returned whole.
pub fn split_command_chain(cmd: &str) -> Vec<&str> {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return vec![];
    }

    // Heredoc or arithmetic expansion: treat as single command
    if trimmed.contains("<<") || trimmed.contains("$((") {
        return vec![trimmed];
    }

    let mut results = Vec::new();
    let mut start = 0;
    let bytes = trimmed.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut in_single = false;
    let mut in_double = false;
    let mut pipe_seen = false;

    while i < len {
        let b = bytes[i];
        match b {
            b'\'' if !in_double => {
                in_single = !in_single;
                i += 1;
            }
            b'"' if !in_single => {
                in_double = !in_double;
                i += 1;
            }
            b'|' if !in_single && !in_double => {
                if i + 1 < len && bytes[i + 1] == b'|' {
                    // ||
                    let segment = trimmed[start..i].trim();
                    if !segment.is_empty() {
                        results.push(segment);
                    }
                    i += 2;
                    start = i;
                } else {
                    // pipe: keep only first command
                    let segment = trimmed[start..i].trim();
                    if !segment.is_empty() {
                        results.push(segment);
                    }
                    pipe_seen = true;
                    break;
                }
            }
            b'&' if !in_single && !in_double && i + 1 < len && bytes[i + 1] == b'&' => {
                let segment = trimmed[start..i].trim();
                if !segment.is_empty() {
                    results.push(segment);
                }
                i += 2;
                start = i;
            }
            b';' if !in_single && !in_double => {
                let segment = trimmed[start..i].trim();
                if !segment.is_empty() {
                    results.push(segment);
                }
                i += 1;
                start = i;
            }
            _ => {
                i += 1;
            }
        }
    }

    if !pipe_seen && start < len {
        let segment = trimmed[start..].trim();
        if !segment.is_empty() {
            results.push(segment);
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::super::report::RtkStatus;
    use super::*;

    #[test]
    fn test_classify_git_status() {
        assert_eq!(
            classify_command("git status"),
            Classification::Supported {
                rtk_equivalent: "rtk git",
                category: "Git",
                estimated_savings_pct: 70.0,
                status: RtkStatus::Existing,
            }
        );
    }

    #[test]
    fn test_classify_git_diff_cached() {
        assert_eq!(
            classify_command("git diff --cached"),
            Classification::Supported {
                rtk_equivalent: "rtk git",
                category: "Git",
                estimated_savings_pct: 80.0,
                status: RtkStatus::Existing,
            }
        );
    }

    // -----------------------------------------------------------------------
    // Command normalization (#2): git/kubectl global flags, $(which), abs paths,
    // leading backslash continuation.
    // -----------------------------------------------------------------------

    fn rtk_equiv(cmd: &str) -> Option<&'static str> {
        match classify_command(cmd) {
            Classification::Supported { rtk_equivalent, .. } => Some(rtk_equivalent),
            _ => None,
        }
    }

    #[test]
    fn test_classify_git_dash_c_path() {
        // `git -C <path> <subcommand>` must classify as the subcommand (upstream #163).
        assert_eq!(
            rtk_equiv("git -C /home/tmoran/ai-root status"),
            Some("rtk git")
        );
        assert_eq!(rtk_equiv("git -C /tmp log --oneline"), Some("rtk git"));
    }

    #[test]
    fn test_classify_git_dash_c_config_flag() {
        assert_eq!(
            rtk_equiv("git -c user.name=bot commit -m x"),
            Some("rtk git")
        );
    }

    #[test]
    fn test_classify_absolute_path_git() {
        // Absolute binary path normalizes to basename (upstream #485).
        assert_eq!(rtk_equiv("/usr/bin/git status"), Some("rtk git"));
    }

    #[test]
    fn test_classify_which_wrapped_git() {
        // $(which git) push ... resolves to a git push.
        assert_eq!(
            rtk_equiv("$(which git) push -u origin HEAD"),
            Some("rtk git")
        );
        assert_eq!(rtk_equiv("`which git` status"), Some("rtk git"));
    }

    #[test]
    fn test_classify_leading_backslash_continuation() {
        // A leading backslash (line continuation / alias bypass) is stripped.
        assert_eq!(rtk_equiv("\\ git status"), Some("rtk git"));
        assert_eq!(rtk_equiv("\\\ngit status"), Some("rtk git"));
    }

    #[test]
    fn test_classify_kubectl_global_flags_before_subcommand() {
        // Net-new: global flags before the subcommand must not defeat detection.
        assert_eq!(
            rtk_equiv("kubectl --context tools -n bot-labs get pods"),
            Some("rtk kubectl")
        );
        assert_eq!(
            rtk_equiv("kubectl -n kube-system get pods"),
            Some("rtk kubectl")
        );
        assert_eq!(
            rtk_equiv("kubectl --kubeconfig /tmp/kc --context prod get deploy"),
            Some("rtk kubectl")
        );
    }

    #[test]
    fn test_rewrite_git_dash_c_preserves_flag() {
        // Rewrite must KEEP the -C flag so the command still targets the repo.
        assert_eq!(
            rewrite_command("git -C /home/x status", &[]),
            Some("rtk git -C /home/x status".to_string())
        );
    }

    #[test]
    fn test_rewrite_which_wrapped_git() {
        assert_eq!(
            rewrite_command("$(which git) push -u origin HEAD", &[]),
            Some("rtk git push -u origin HEAD".to_string())
        );
    }

    #[test]
    fn test_rewrite_kubectl_context_preserves_flag() {
        assert_eq!(
            rewrite_command("kubectl --context tools get pods", &[]),
            Some("rtk kubectl --context tools get pods".to_string())
        );
    }

    // -----------------------------------------------------------------------
    // Bucket 3: expanded git/kubectl subcommand coverage + glab classification.
    // -----------------------------------------------------------------------

    fn classify_full(cmd: &str) -> Option<(&'static str, RtkStatus, f64)> {
        match classify_command(cmd) {
            Classification::Supported {
                rtk_equivalent,
                status,
                estimated_savings_pct,
                ..
            } => Some((rtk_equivalent, status, estimated_savings_pct)),
            _ => None,
        }
    }

    #[test]
    fn test_classify_git_passthrough_subcommands() {
        for sub in [
            "checkout",
            "switch",
            "rebase",
            "merge",
            "reset",
            "tag",
            "cherry-pick",
        ] {
            let cmd = format!("git {sub} whatever");
            let (equiv, status, savings) =
                classify_full(&cmd).unwrap_or_else(|| panic!("{cmd} should classify"));
            assert_eq!(equiv, "rtk git", "{cmd}");
            assert_eq!(
                status,
                RtkStatus::Passthrough,
                "{cmd} should be passthrough"
            );
            assert_eq!(savings, 0.0, "{cmd} passthrough has no savings");
        }
    }

    #[test]
    fn test_classify_git_compact_still_existing() {
        // Regression: dedicated-filter subcommands stay Existing with real savings.
        let (_, status, savings) = classify_full("git status").unwrap();
        assert_eq!(status, RtkStatus::Existing);
        assert!(savings > 0.0);
    }

    #[test]
    fn test_classify_kubectl_passthrough_subcommands() {
        for sub in ["config", "describe", "apply", "delete", "rollout", "scale"] {
            let cmd = format!("kubectl {sub} something");
            let (equiv, status, _) =
                classify_full(&cmd).unwrap_or_else(|| panic!("{cmd} should classify"));
            assert_eq!(equiv, "rtk kubectl", "{cmd}");
            assert_eq!(status, RtkStatus::Passthrough, "{cmd}");
        }
        // get/logs keep their dedicated-filter status.
        assert_eq!(
            classify_full("kubectl get pods").unwrap().1,
            RtkStatus::Existing
        );
    }

    #[test]
    fn test_classify_glab() {
        // mr/ci/issue have compact filters.
        let (equiv, status, savings) = classify_full("glab mr view 31").unwrap();
        assert_eq!(equiv, "rtk glab");
        assert_eq!(status, RtkStatus::Existing);
        assert!(savings > 0.0);
        // other subcommands pass through.
        let (equiv, status, _) = classify_full("glab api projects/1").unwrap();
        assert_eq!(equiv, "rtk glab");
        assert_eq!(status, RtkStatus::Passthrough);
    }

    #[test]
    fn test_default_status_for_heuristic() {
        assert_eq!(default_status_for(0.0), RtkStatus::Passthrough);
        assert_eq!(default_status_for(80.0), RtkStatus::Existing);
    }

    #[test]
    fn test_strip_helpers_units() {
        assert_eq!(strip_git_global_opts("git -C /tmp status"), "git status");
        assert_eq!(strip_git_global_opts("git status"), "git status");
        assert_eq!(
            strip_kubectl_global_opts("kubectl --context x -n y get pods"),
            "kubectl get pods"
        );
        assert_eq!(strip_absolute_path("/usr/local/bin/grep foo"), "grep foo");
        assert_eq!(unwrap_command_substitution("$(which git) push"), "git push");
        assert_eq!(strip_leading_continuation("\\ git status"), "git status");
    }

    #[test]
    fn test_classify_cargo_test_filter() {
        assert_eq!(
            classify_command("cargo test filter::"),
            Classification::Supported {
                rtk_equivalent: "rtk cargo",
                category: "Cargo",
                estimated_savings_pct: 90.0,
                status: RtkStatus::Existing,
            }
        );
    }

    #[test]
    fn test_classify_npx_tsc() {
        assert_eq!(
            classify_command("npx tsc --noEmit"),
            Classification::Supported {
                rtk_equivalent: "rtk tsc",
                category: "Build",
                estimated_savings_pct: 83.0,
                status: RtkStatus::Existing,
            }
        );
    }

    #[test]
    fn test_classify_cat_file() {
        assert_eq!(
            classify_command("cat src/main.rs"),
            Classification::Supported {
                rtk_equivalent: "rtk read",
                category: "Files",
                estimated_savings_pct: 60.0,
                status: RtkStatus::Existing,
            }
        );
    }

    #[test]
    fn test_classify_cd_ignored() {
        assert_eq!(classify_command("cd /tmp"), Classification::Ignored);
    }

    #[test]
    fn test_classify_rtk_already() {
        assert_eq!(classify_command("rtk git status"), Classification::Ignored);
    }

    #[test]
    fn test_classify_echo_ignored() {
        assert_eq!(
            classify_command("echo hello world"),
            Classification::Ignored
        );
    }

    #[test]
    fn test_classify_terraform_unsupported() {
        match classify_command("terraform plan -var-file=prod.tfvars") {
            Classification::Unsupported { base_command } => {
                assert_eq!(base_command, "terraform plan");
            }
            other => panic!("expected Unsupported, got {:?}", other),
        }
    }

    #[test]
    fn test_classify_mypy() {
        assert_eq!(
            classify_command("mypy src/"),
            Classification::Supported {
                rtk_equivalent: "rtk mypy",
                category: "Build",
                estimated_savings_pct: 80.0,
                status: RtkStatus::Existing,
            }
        );
    }

    #[test]
    fn test_classify_python_m_mypy() {
        assert_eq!(
            classify_command("python3 -m mypy --strict"),
            Classification::Supported {
                rtk_equivalent: "rtk mypy",
                category: "Build",
                estimated_savings_pct: 80.0,
                status: RtkStatus::Existing,
            }
        );
    }

    #[test]
    fn test_classify_env_prefix_stripped() {
        assert_eq!(
            classify_command("GIT_SSH_COMMAND=ssh git push"),
            Classification::Supported {
                rtk_equivalent: "rtk git",
                category: "Git",
                estimated_savings_pct: 70.0,
                status: RtkStatus::Existing,
            }
        );
    }

    #[test]
    fn test_classify_sudo_stripped() {
        assert_eq!(
            classify_command("sudo docker ps"),
            Classification::Supported {
                rtk_equivalent: "rtk docker",
                category: "Infra",
                estimated_savings_pct: 85.0,
                status: RtkStatus::Existing,
            }
        );
    }

    #[test]
    fn test_classify_cargo_check() {
        assert_eq!(
            classify_command("cargo check"),
            Classification::Supported {
                rtk_equivalent: "rtk cargo",
                category: "Cargo",
                estimated_savings_pct: 80.0,
                status: RtkStatus::Existing,
            }
        );
    }

    #[test]
    fn test_classify_cargo_check_all_targets() {
        assert_eq!(
            classify_command("cargo check --all-targets"),
            Classification::Supported {
                rtk_equivalent: "rtk cargo",
                category: "Cargo",
                estimated_savings_pct: 80.0,
                status: RtkStatus::Existing,
            }
        );
    }

    #[test]
    fn test_classify_cargo_fmt_passthrough() {
        assert_eq!(
            classify_command("cargo fmt"),
            Classification::Supported {
                rtk_equivalent: "rtk cargo",
                category: "Cargo",
                estimated_savings_pct: 80.0,
                status: RtkStatus::Passthrough,
            }
        );
    }

    #[test]
    fn test_classify_cargo_clippy_savings() {
        assert_eq!(
            classify_command("cargo clippy --all-targets"),
            Classification::Supported {
                rtk_equivalent: "rtk cargo",
                category: "Cargo",
                estimated_savings_pct: 80.0,
                status: RtkStatus::Existing,
            }
        );
    }

    #[test]
    fn test_patterns_rules_length_match() {
        assert_eq!(
            PATTERNS.len(),
            RULES.len(),
            "PATTERNS and RULES must be aligned"
        );
    }

    #[test]
    fn test_registry_covers_all_cargo_subcommands() {
        // Verify that every CargoCommand variant (Build, Test, Clippy, Check, Fmt)
        // except Other has a matching pattern in the registry
        for subcmd in ["build", "test", "clippy", "check", "fmt"] {
            let cmd = format!("cargo {subcmd}");
            match classify_command(&cmd) {
                Classification::Supported { .. } => {}
                other => panic!("cargo {subcmd} should be Supported, got {other:?}"),
            }
        }
    }

    #[test]
    fn test_registry_covers_all_git_subcommands() {
        // Verify that every GitCommand subcommand has a matching pattern
        for subcmd in [
            "status", "log", "diff", "show", "add", "commit", "push", "pull", "branch", "fetch",
            "stash", "worktree",
        ] {
            let cmd = format!("git {subcmd}");
            match classify_command(&cmd) {
                Classification::Supported { .. } => {}
                other => panic!("git {subcmd} should be Supported, got {other:?}"),
            }
        }
    }

    #[test]
    fn test_split_chain_and() {
        assert_eq!(split_command_chain("a && b"), vec!["a", "b"]);
    }

    #[test]
    fn test_split_chain_semicolon() {
        assert_eq!(split_command_chain("a ; b"), vec!["a", "b"]);
    }

    #[test]
    fn test_split_pipe_first_only() {
        assert_eq!(split_command_chain("a | b"), vec!["a"]);
    }

    #[test]
    fn test_split_single() {
        assert_eq!(split_command_chain("git status"), vec!["git status"]);
    }

    #[test]
    fn test_split_quoted_and() {
        assert_eq!(
            split_command_chain(r#"echo "a && b""#),
            vec![r#"echo "a && b""#]
        );
    }

    #[test]
    fn test_split_heredoc_no_split() {
        let cmd = "cat <<'EOF'\nhello && world\nEOF";
        assert_eq!(split_command_chain(cmd), vec![cmd]);
    }

    #[test]
    fn test_rewrite_git_status() {
        assert_eq!(
            rewrite_command("git status", &[]),
            Some("rtk git status".to_string())
        );
    }

    #[test]
    fn test_rewrite_git_with_args() {
        assert_eq!(
            rewrite_command("git log --oneline -10", &[]),
            Some("rtk git log --oneline -10".to_string())
        );
    }

    #[test]
    fn test_rewrite_cargo_test() {
        assert_eq!(
            rewrite_command("cargo test", &[]),
            Some("rtk cargo test".to_string())
        );
    }

    #[test]
    fn test_rewrite_npx_tsc() {
        assert_eq!(
            rewrite_command("npx tsc --noEmit", &[]),
            Some("rtk tsc --noEmit".to_string())
        );
    }

    #[test]
    fn test_rewrite_pnpm_vitest() {
        assert_eq!(
            rewrite_command("pnpm vitest run", &[]),
            Some("rtk vitest run".to_string())
        );
    }

    #[test]
    fn test_rewrite_excluded_git() {
        assert_eq!(rewrite_command("git status", &["git".to_string()]), None);
    }

    #[test]
    fn test_rewrite_excluded_cargo() {
        assert_eq!(rewrite_command("cargo test", &["cargo".to_string()]), None);
    }

    #[test]
    fn test_rewrite_unsupported_terraform() {
        assert_eq!(rewrite_command("terraform plan", &[]), None);
    }

    #[test]
    fn test_rewrite_ignored_echo() {
        assert_eq!(rewrite_command("echo hello", &[]), None);
    }

    #[test]
    fn test_rewrite_already_rtk() {
        assert_eq!(rewrite_command("rtk git status", &[]), None);
    }

    #[test]
    fn test_cat_without_flags_is_rewritten() {
        assert_eq!(
            rewrite_command("cat file.txt", &[]),
            Some("rtk read file.txt".to_string())
        );
    }

    #[test]
    fn test_cat_with_n_flag_not_rewritten() {
        assert_eq!(rewrite_command("cat -n file.txt", &[]), None);
        assert_eq!(rewrite_command("cat --number file.txt", &[]), None);
    }

    #[test]
    fn test_cat_with_show_all_flag_not_rewritten() {
        assert_eq!(rewrite_command("cat -A file.txt", &[]), None);
        assert_eq!(rewrite_command("cat --show-all file.txt", &[]), None);
    }

    #[test]
    fn test_cat_with_b_flag_not_rewritten() {
        assert_eq!(rewrite_command("cat -b file.txt", &[]), None);
        assert_eq!(rewrite_command("cat --number-nonblank file.txt", &[]), None);
    }

    #[test]
    fn test_cat_with_other_incompatible_flags_not_rewritten() {
        assert_eq!(rewrite_command("cat -e file.txt", &[]), None);
        assert_eq!(rewrite_command("cat -E file.txt", &[]), None);
        assert_eq!(rewrite_command("cat --show-ends file.txt", &[]), None);
        assert_eq!(rewrite_command("cat -s file.txt", &[]), None);
        assert_eq!(rewrite_command("cat --squeeze-blank file.txt", &[]), None);
        assert_eq!(rewrite_command("cat -T file.txt", &[]), None);
        assert_eq!(rewrite_command("cat --show-tabs file.txt", &[]), None);
        assert_eq!(rewrite_command("cat -t file.txt", &[]), None);
        assert_eq!(rewrite_command("cat -v file.txt", &[]), None);
        assert_eq!(
            rewrite_command("cat --show-nonprinting file.txt", &[]),
            None
        );
    }

    #[test]
    fn test_rewrite_with_env_prefix() {
        assert_eq!(
            rewrite_command("GIT_SSH_COMMAND=ssh git push", &[]),
            Some("rtk git push".to_string())
        );
    }

    #[test]
    fn test_classify_terragrunt_plan() {
        assert_eq!(
            classify_command("terragrunt plan 2>&1"),
            Classification::Supported {
                rtk_equivalent: "rtk terragrunt",
                category: "Infra",
                estimated_savings_pct: 85.0,
                status: RtkStatus::Existing,
            }
        );
    }

    #[test]
    fn test_classify_terragrunt_apply() {
        assert_eq!(
            classify_command("terragrunt apply -auto-approve 2>&1"),
            Classification::Supported {
                rtk_equivalent: "rtk terragrunt",
                category: "Infra",
                estimated_savings_pct: 85.0,
                status: RtkStatus::Existing,
            }
        );
    }

    #[test]
    fn test_classify_terragrunt_state_passthrough() {
        assert_eq!(
            classify_command("terragrunt state list 2>&1"),
            Classification::Supported {
                rtk_equivalent: "rtk terragrunt",
                category: "Infra",
                estimated_savings_pct: 80.0,
                status: RtkStatus::Passthrough,
            }
        );
    }

    #[test]
    fn test_classify_terragrunt_init() {
        assert_eq!(
            classify_command("terragrunt init"),
            Classification::Supported {
                rtk_equivalent: "rtk terragrunt",
                category: "Infra",
                estimated_savings_pct: 80.0,
                status: RtkStatus::Existing,
            }
        );
    }

    #[test]
    fn test_rewrite_terragrunt_plan() {
        assert_eq!(
            rewrite_command("terragrunt plan -var-file=prod.tfvars", &[]),
            Some("rtk terragrunt plan -var-file=prod.tfvars".to_string())
        );
    }

    #[test]
    fn test_rewrite_with_sudo() {
        assert_eq!(
            rewrite_command("sudo docker ps", &[]),
            Some("rtk docker ps".to_string())
        );
    }
}
