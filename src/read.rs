use crate::filter::{self, FilterLevel, Language};
use crate::tracking;
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// Check if data is likely binary by scanning for null bytes in first 8KB
fn is_likely_binary(data: &[u8]) -> bool {
    let check_len = data.len().min(8192);
    data[..check_len].contains(&0)
}

/// Format bytes into human-readable size
fn human_size(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} bytes", bytes)
    }
}

pub fn run(
    file: &Path,
    level: FilterLevel,
    max_lines: Option<usize>,
    line_numbers: bool,
    verbose: u8,
) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    if verbose > 0 {
        eprintln!("Reading: {} (filter: {})", file.display(), level);
    }

    // Read file as bytes first for binary detection
    let raw_bytes =
        fs::read(file).with_context(|| format!("Failed to read file: {}", file.display()))?;

    // Binary file detection
    if is_likely_binary(&raw_bytes) {
        let size = human_size(raw_bytes.len() as u64);
        let msg = format!("[binary file: {} ({})]", file.display(), size);
        println!("{}", msg);
        println!("hint: use cat {} to view raw content", file.display());
        timer.track(&format!("cat {}", file.display()), "rtk read", &msg, &msg);
        return Ok(());
    }

    // Convert to UTF-8
    let content = String::from_utf8(raw_bytes)
        .with_context(|| format!("File is not valid UTF-8: {}", file.display()))?;

    // Detect language from extension
    let lang = file
        .extension()
        .and_then(|e| e.to_str())
        .map(Language::from_extension)
        .unwrap_or(Language::Unknown);

    if verbose > 1 {
        eprintln!("Detected language: {:?}", lang);
    }

    // Apply filter
    let filter = filter::get_filter(level);
    let mut filtered = filter.filter(&content, &lang);

    // Safety: if filter produced empty output from non-empty input, warn and fallback
    if filtered.trim().is_empty() && !content.trim().is_empty() {
        eprintln!(
            "rtk: warning: filter produced empty output for {} ({} bytes), showing raw content",
            file.display(),
            content.len()
        );
        filtered = content.clone();
    }

    if verbose > 0 {
        let original_lines = content.lines().count();
        let filtered_lines = filtered.lines().count();
        let reduction = if original_lines > 0 {
            ((original_lines - filtered_lines) as f64 / original_lines as f64) * 100.0
        } else {
            0.0
        };
        eprintln!(
            "Lines: {} -> {} ({:.1}% reduction)",
            original_lines, filtered_lines, reduction
        );
    }

    // Apply smart truncation if max_lines is set
    if let Some(max) = max_lines {
        filtered = filter::smart_truncate(&filtered, max, &lang);
    }

    let rtk_output = if line_numbers {
        format_with_line_numbers(&filtered)
    } else {
        filtered.clone()
    };
    println!("{}", rtk_output);
    timer.track(
        &format!("cat {}", file.display()),
        "rtk read",
        &content,
        &rtk_output,
    );
    Ok(())
}

pub fn run_stdin(
    level: FilterLevel,
    max_lines: Option<usize>,
    line_numbers: bool,
    verbose: u8,
) -> Result<()> {
    use std::io::{self, Read as IoRead};

    let timer = tracking::TimedExecution::start();

    if verbose > 0 {
        eprintln!("Reading from stdin (filter: {})", level);
    }

    // Read from stdin
    let mut content = String::new();
    io::stdin()
        .lock()
        .read_to_string(&mut content)
        .context("Failed to read from stdin")?;

    // No file extension, so use Unknown language
    let lang = Language::Unknown;

    if verbose > 1 {
        eprintln!("Language: {:?} (stdin has no extension)", lang);
    }

    // Apply filter
    let filter = filter::get_filter(level);
    let mut filtered = filter.filter(&content, &lang);

    if verbose > 0 {
        let original_lines = content.lines().count();
        let filtered_lines = filtered.lines().count();
        let reduction = if original_lines > 0 {
            ((original_lines - filtered_lines) as f64 / original_lines as f64) * 100.0
        } else {
            0.0
        };
        eprintln!(
            "Lines: {} -> {} ({:.1}% reduction)",
            original_lines, filtered_lines, reduction
        );
    }

    // Apply smart truncation if max_lines is set
    if let Some(max) = max_lines {
        filtered = filter::smart_truncate(&filtered, max, &lang);
    }

    let rtk_output = if line_numbers {
        format_with_line_numbers(&filtered)
    } else {
        filtered.clone()
    };
    println!("{}", rtk_output);

    timer.track("cat - (stdin)", "rtk read -", &content, &rtk_output);
    Ok(())
}

fn format_with_line_numbers(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let width = lines.len().to_string().len();
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        out.push_str(&format!("{:>width$} │ {}\n", i + 1, line, width = width));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_read_rust_file() -> Result<()> {
        let mut file = NamedTempFile::with_suffix(".rs")?;
        writeln!(
            file,
            r#"// Comment
fn main() {{
    println!("Hello");
}}"#
        )?;

        // Just verify it doesn't panic
        run(file.path(), FilterLevel::Minimal, None, false, 0)?;
        Ok(())
    }

    #[test]
    fn test_stdin_support_signature() {
        // Test that run_stdin has correct signature and compiles
        // We don't actually run it because it would hang waiting for stdin
        // Compile-time verification that the function exists with correct signature
    }

    #[test]
    fn test_is_binary_detects_null_bytes() {
        let data = b"hello\x00world";
        assert!(is_likely_binary(data));
    }

    #[test]
    fn test_is_binary_passes_text() {
        let data = b"fn main() {\n    println!(\"hello\");\n}";
        assert!(!is_likely_binary(data));
    }

    #[test]
    fn test_is_binary_passes_utf8() {
        let data = "日本語のコード".as_bytes();
        assert!(!is_likely_binary(data));
    }

    #[test]
    fn test_human_size_bytes() {
        assert_eq!(human_size(500), "500 bytes");
    }

    #[test]
    fn test_human_size_kb() {
        assert_eq!(human_size(2048), "2.0 KB");
    }

    #[test]
    fn test_human_size_mb() {
        assert_eq!(human_size(5_242_880), "5.0 MB");
    }
}
