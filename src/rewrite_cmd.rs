use anyhow::Result;

/// Rewrite a command to use RTK if supported.
///
/// Used by shell hooks to automatically rewrite commands like `git status` to `rtk git status`.
/// Respects the `hooks.exclude_commands` configuration to skip rewriting specific commands.
///
/// Exit codes:
/// - 0: Command was rewritten (prints rewritten command to stdout)
/// - 1: Command should not be rewritten (no match or excluded)
pub fn run(cmd: &str) -> Result<()> {
    // Load config to get excluded commands
    let config = crate::config::Config::load().unwrap_or_default();
    let excluded = &config.hooks.exclude_commands;

    // Try to rewrite the command
    if let Some(rewritten) = crate::discover::registry::rewrite_command(cmd, excluded) {
        // Print rewritten command to stdout
        println!("{}", rewritten);
        std::process::exit(0);
    } else {
        // No match - exit with code 1
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_rewrite_git_status() {
        let result = crate::discover::registry::rewrite_command("git status", &[]);
        assert_eq!(result, Some("rtk git status".to_string()));
    }

    #[test]
    fn test_rewrite_excluded_command() {
        let excluded = vec!["git".to_string()];
        let result = crate::discover::registry::rewrite_command("git status", &excluded);
        assert_eq!(result, None);
    }

    #[test]
    fn test_rewrite_cargo_test() {
        let result = crate::discover::registry::rewrite_command("cargo test", &[]);
        assert_eq!(result, Some("rtk cargo test".to_string()));
    }

    #[test]
    fn test_rewrite_npx_tsc() {
        let result = crate::discover::registry::rewrite_command("npx tsc --noEmit", &[]);
        assert_eq!(result, Some("rtk tsc --noEmit".to_string()));
    }

    #[test]
    fn test_rewrite_unsupported() {
        let result = crate::discover::registry::rewrite_command("terraform plan", &[]);
        assert_eq!(result, None);
    }

    #[test]
    fn test_rewrite_already_rtk() {
        let result = crate::discover::registry::rewrite_command("rtk git status", &[]);
        assert_eq!(result, None);
    }
}
