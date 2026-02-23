//! Token savings tracking and analytics system.
//!
//! This module provides comprehensive tracking of RTK command executions,
//! recording token savings, execution times, and providing aggregation APIs
//! for daily/weekly/monthly statistics.
//!
//! # Architecture
//!
//! - Storage: SQLite database (~/.local/share/rtk/tracking.db)
//! - Retention: 90-day automatic cleanup
//! - Metrics: Input/output tokens, savings %, execution time
//!
//! # Quick Start
//!
//! ```no_run
//! use rtk::tracking::{QueryScope, TimedExecution, Tracker};
//!
//! // Track a command execution
//! let timer = TimedExecution::start();
//! let input = "raw output";
//! let output = "filtered output";
//! timer.track("ls -la", "rtk ls", input, output);
//!
//! // Query statistics
//! let tracker = Tracker::new().unwrap();
//! let summary = tracker.get_summary(&QueryScope::Global, 10).unwrap();
//! println!("Saved {} tokens", summary.total_saved);
//! ```
//!
//! See [docs/tracking.md](../docs/tracking.md) for full documentation.

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Number of days to retain tracking history before automatic cleanup.
const HISTORY_DAYS: i64 = 90;

/// Scope for gain queries: filter by project or show all.
pub enum QueryScope {
    /// Filter to records matching a specific working directory.
    Project(String),
    /// No filter — include all records.
    Global,
}

/// Detect the project root by walking up from CWD.
///
/// Looks for `.git`, `Cargo.toml`, `package.json`, `go.mod`, `pyproject.toml`.
/// Returns the root path as a string, or empty string if no marker found.
pub fn detect_project_root() -> String {
    detect_project_root_from(&std::env::current_dir().unwrap_or_default())
}

fn detect_project_root_from(start: &Path) -> String {
    const MARKERS: &[&str] = &[
        ".git",
        "Cargo.toml",
        "package.json",
        "go.mod",
        "pyproject.toml",
    ];

    // Canonicalize once so symlinks, relative components, and platform
    // path differences (e.g. drive-letter case on Windows) are normalized
    // before storage.  Fall back to the raw path if canonicalization fails.
    let mut dir = std::fs::canonicalize(start).unwrap_or_else(|_| start.to_path_buf());
    loop {
        for marker in MARKERS {
            if dir.join(marker).exists() {
                return dir.to_string_lossy().into_owned();
            }
        }
        if !dir.pop() {
            break;
        }
    }
    String::new()
}

/// Main tracking interface for recording and querying command history.
///
/// Manages SQLite database connection and provides methods for:
/// - Recording command executions with token counts and timing
/// - Querying aggregated statistics (summary, daily, weekly, monthly)
/// - Retrieving recent command history
///
/// # Database Location
///
/// - Linux: `~/.local/share/rtk/tracking.db`
/// - macOS: `~/Library/Application Support/rtk/tracking.db`
/// - Windows: `%APPDATA%\rtk\tracking.db`
///
/// # Examples
///
/// ```no_run
/// use rtk::tracking::{QueryScope, Tracker};
///
/// let tracker = Tracker::new()?;
/// tracker.record("ls -la", "rtk ls", 1000, 200, 50, "/home/user/myproject")?;
///
/// let summary = tracker.get_summary(&QueryScope::Global, 10)?;
/// println!("Total saved: {} tokens", summary.total_saved);
/// # Ok::<(), anyhow::Error>(())
/// ```
pub struct Tracker {
    conn: Connection,
}

/// Individual command record from tracking history.
///
/// Contains timestamp, command name, and savings metrics for a single execution.
#[derive(Debug)]
pub struct CommandRecord {
    /// UTC timestamp when command was executed
    pub timestamp: DateTime<Utc>,
    /// RTK command that was executed (e.g., "rtk ls")
    pub rtk_cmd: String,
    /// Number of tokens saved (input - output)
    pub saved_tokens: usize,
    /// Savings percentage ((saved / input) * 100)
    pub savings_pct: f64,
}

/// Aggregated statistics across all recorded commands.
///
/// Provides overall metrics and breakdowns by command and by day.
/// Returned by [`Tracker::get_summary`].
#[derive(Debug)]
pub struct GainSummary {
    /// Total number of commands recorded
    pub total_commands: usize,
    /// Total input tokens across all commands
    pub total_input: usize,
    /// Total output tokens across all commands
    pub total_output: usize,
    /// Total tokens saved (input - output)
    pub total_saved: usize,
    /// Average savings percentage across all commands
    pub avg_savings_pct: f64,
    /// Total execution time across all commands (milliseconds)
    pub total_time_ms: u64,
    /// Average execution time per command (milliseconds)
    pub avg_time_ms: u64,
    /// Top 10 commands by tokens saved: (cmd, count, saved, avg_pct, avg_time_ms)
    pub by_command: Vec<CommandStats>,
    /// Last 30 days of activity: (date, saved_tokens)
    pub by_day: Vec<(String, usize)>,
}

/// Daily statistics for token savings and execution metrics.
///
/// Serializable to JSON for export via `rtk gain --daily --format json`.
///
/// # JSON Schema
///
/// ```json
/// {
///   "date": "2026-02-03",
///   "commands": 42,
///   "input_tokens": 15420,
///   "output_tokens": 3842,
///   "saved_tokens": 11578,
///   "savings_pct": 75.08,
///   "total_time_ms": 8450,
///   "avg_time_ms": 201
/// }
/// ```
#[derive(Debug, Serialize)]
pub struct DayStats {
    /// ISO date (YYYY-MM-DD)
    pub date: String,
    /// Number of commands executed this day
    pub commands: usize,
    /// Total input tokens for this day
    pub input_tokens: usize,
    /// Total output tokens for this day
    pub output_tokens: usize,
    /// Total tokens saved this day
    pub saved_tokens: usize,
    /// Savings percentage for this day
    pub savings_pct: f64,
    /// Total execution time for this day (milliseconds)
    pub total_time_ms: u64,
    /// Average execution time per command (milliseconds)
    pub avg_time_ms: u64,
}

/// Weekly statistics for token savings and execution metrics.
///
/// Serializable to JSON for export via `rtk gain --weekly --format json`.
/// Weeks start on Sunday (SQLite default).
#[derive(Debug, Serialize)]
pub struct WeekStats {
    /// Week start date (YYYY-MM-DD)
    pub week_start: String,
    /// Week end date (YYYY-MM-DD)
    pub week_end: String,
    /// Number of commands executed this week
    pub commands: usize,
    /// Total input tokens for this week
    pub input_tokens: usize,
    /// Total output tokens for this week
    pub output_tokens: usize,
    /// Total tokens saved this week
    pub saved_tokens: usize,
    /// Savings percentage for this week
    pub savings_pct: f64,
    /// Total execution time for this week (milliseconds)
    pub total_time_ms: u64,
    /// Average execution time per command (milliseconds)
    pub avg_time_ms: u64,
}

/// Monthly statistics for token savings and execution metrics.
///
/// Serializable to JSON for export via `rtk gain --monthly --format json`.
#[derive(Debug, Serialize)]
pub struct MonthStats {
    /// Month identifier (YYYY-MM)
    pub month: String,
    /// Number of commands executed this month
    pub commands: usize,
    /// Total input tokens for this month
    pub input_tokens: usize,
    /// Total output tokens for this month
    pub output_tokens: usize,
    /// Total tokens saved this month
    pub saved_tokens: usize,
    /// Savings percentage for this month
    pub savings_pct: f64,
    /// Total execution time for this month (milliseconds)
    pub total_time_ms: u64,
    /// Average execution time per command (milliseconds)
    pub avg_time_ms: u64,
}

type CommandStats = (String, usize, usize, f64, u64);

impl Tracker {
    /// Create a new tracker instance.
    ///
    /// Opens or creates the SQLite database at the platform-specific location.
    /// Automatically creates the `commands` table if it doesn't exist and runs
    /// any necessary schema migrations.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Cannot determine database path
    /// - Cannot create parent directories
    /// - Cannot open/create SQLite database
    /// - Schema creation/migration fails
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use rtk::tracking::Tracker;
    ///
    /// let tracker = Tracker::new()?;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn new() -> Result<Self> {
        let db_path = get_db_path()?;
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(&db_path)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS commands (
                id INTEGER PRIMARY KEY,
                timestamp TEXT NOT NULL,
                original_cmd TEXT NOT NULL,
                rtk_cmd TEXT NOT NULL,
                input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                saved_tokens INTEGER NOT NULL,
                savings_pct REAL NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_timestamp ON commands(timestamp)",
            [],
        )?;

        // Migration: add exec_time_ms column if it doesn't exist
        let _ = conn.execute(
            "ALTER TABLE commands ADD COLUMN exec_time_ms INTEGER DEFAULT 0",
            [],
        );

        // Migration: add working_dir column if it doesn't exist
        let _ = conn.execute(
            "ALTER TABLE commands ADD COLUMN working_dir TEXT DEFAULT ''",
            [],
        );
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_working_dir ON commands(working_dir)",
            [],
        )?;

        Ok(Self { conn })
    }

    /// Create a tracker with a specific database path.
    ///
    /// Used by tests to avoid polluting the production database.
    #[cfg(test)]
    pub fn with_path(db_path: &std::path::Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(db_path)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS commands (
                id INTEGER PRIMARY KEY,
                timestamp TEXT NOT NULL,
                original_cmd TEXT NOT NULL,
                rtk_cmd TEXT NOT NULL,
                input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                saved_tokens INTEGER NOT NULL,
                savings_pct REAL NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_timestamp ON commands(timestamp)",
            [],
        )?;

        let _ = conn.execute(
            "ALTER TABLE commands ADD COLUMN exec_time_ms INTEGER DEFAULT 0",
            [],
        );

        let _ = conn.execute(
            "ALTER TABLE commands ADD COLUMN working_dir TEXT DEFAULT ''",
            [],
        );
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_working_dir ON commands(working_dir)",
            [],
        )?;

        Ok(Self { conn })
    }

    /// Record a command execution with token counts and timing.
    ///
    /// Calculates savings metrics and stores the record in the database.
    /// Automatically cleans up records older than 90 days after insertion.
    ///
    /// # Arguments
    ///
    /// - `original_cmd`: The standard command (e.g., "ls -la")
    /// - `rtk_cmd`: The RTK command used (e.g., "rtk ls")
    /// - `input_tokens`: Estimated tokens from standard command output
    /// - `output_tokens`: Actual tokens from RTK output
    /// - `exec_time_ms`: Execution time in milliseconds
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use rtk::tracking::Tracker;
    ///
    /// let tracker = Tracker::new()?;
    /// tracker.record("ls -la", "rtk ls", 1000, 200, 50, "/home/user/myproject")?;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn record(
        &self,
        original_cmd: &str,
        rtk_cmd: &str,
        input_tokens: usize,
        output_tokens: usize,
        exec_time_ms: u64,
        working_dir: &str,
    ) -> Result<()> {
        let saved = input_tokens.saturating_sub(output_tokens);
        let pct = if input_tokens > 0 {
            (saved as f64 / input_tokens as f64) * 100.0
        } else {
            0.0
        };

        self.conn.execute(
            "INSERT INTO commands (timestamp, original_cmd, rtk_cmd, input_tokens, output_tokens, saved_tokens, savings_pct, exec_time_ms, working_dir)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                Utc::now().to_rfc3339(),
                original_cmd,
                rtk_cmd,
                input_tokens as i64,
                output_tokens as i64,
                saved as i64,
                pct,
                exec_time_ms as i64,
                working_dir
            ],
        )?;

        self.cleanup_old()?;
        Ok(())
    }

    fn cleanup_old(&self) -> Result<()> {
        let cutoff = Utc::now() - chrono::Duration::days(HISTORY_DAYS);
        self.conn.execute(
            "DELETE FROM commands WHERE timestamp < ?1",
            params![cutoff.to_rfc3339()],
        )?;
        Ok(())
    }

    /// Get overall summary statistics, optionally scoped to a project.
    pub fn get_summary(&self, scope: &QueryScope, top_n: usize) -> Result<GainSummary> {
        let mut total_commands = 0usize;
        let mut total_input = 0usize;
        let mut total_output = 0usize;
        let mut total_saved = 0usize;
        let mut total_time_ms = 0u64;

        let (where_clause, scope_param) = scope_filter(scope);
        let sql = format!(
            "SELECT input_tokens, output_tokens, saved_tokens, exec_time_ms FROM commands{}",
            where_clause
        );
        let mut stmt = self.conn.prepare(&sql)?;

        let map_row = |row: &rusqlite::Row| -> rusqlite::Result<(usize, usize, usize, u64)> {
            Ok((
                row.get::<_, i64>(0)? as usize,
                row.get::<_, i64>(1)? as usize,
                row.get::<_, i64>(2)? as usize,
                row.get::<_, i64>(3)? as u64,
            ))
        };

        let rows = if let Some(ref dir) = scope_param {
            stmt.query_map(params![dir], &map_row)?
        } else {
            stmt.query_map([], &map_row)?
        };

        for row in rows {
            let (input, output, saved, time_ms) = row?;
            total_commands += 1;
            total_input += input;
            total_output += output;
            total_saved += saved;
            total_time_ms += time_ms;
        }

        let avg_savings_pct = if total_input > 0 {
            (total_saved as f64 / total_input as f64) * 100.0
        } else {
            0.0
        };

        let avg_time_ms = if total_commands > 0 {
            total_time_ms / total_commands as u64
        } else {
            0
        };

        let by_command = self.get_by_command(scope, top_n)?;
        let by_day = self.get_by_day(scope)?;

        Ok(GainSummary {
            total_commands,
            total_input,
            total_output,
            total_saved,
            avg_savings_pct,
            total_time_ms,
            avg_time_ms,
            by_command,
            by_day,
        })
    }

    fn get_by_command(&self, scope: &QueryScope, top_n: usize) -> Result<Vec<CommandStats>> {
        let (where_clause, scope_param) = scope_filter(scope);
        let limit_param = if scope_param.is_some() { "?2" } else { "?1" };
        let sql = format!(
            "SELECT rtk_cmd, COUNT(*), SUM(saved_tokens), AVG(savings_pct), AVG(exec_time_ms)
             FROM commands{}
             GROUP BY rtk_cmd
             ORDER BY SUM(saved_tokens) DESC
             LIMIT {}",
            where_clause, limit_param
        );
        let mut stmt = self.conn.prepare(&sql)?;

        let map_row = |row: &rusqlite::Row| -> rusqlite::Result<CommandStats> {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)? as usize,
                row.get::<_, i64>(2)? as usize,
                row.get::<_, f64>(3)?,
                row.get::<_, f64>(4)? as u64,
            ))
        };

        let rows = if let Some(ref dir) = scope_param {
            stmt.query_map(params![dir, top_n as i64], &map_row)?
        } else {
            stmt.query_map(params![top_n as i64], &map_row)?
        };

        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    fn get_by_day(&self, scope: &QueryScope) -> Result<Vec<(String, usize)>> {
        let (where_clause, scope_param) = scope_filter(scope);
        let sql = format!(
            "SELECT DATE(timestamp), SUM(saved_tokens)
             FROM commands{}
             GROUP BY DATE(timestamp)
             ORDER BY DATE(timestamp) DESC
             LIMIT 30",
            where_clause
        );
        let mut stmt = self.conn.prepare(&sql)?;

        let map_row = |row: &rusqlite::Row| -> rusqlite::Result<(String, usize)> {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize))
        };

        let rows = if let Some(ref dir) = scope_param {
            stmt.query_map(params![dir], &map_row)?
        } else {
            stmt.query_map([], &map_row)?
        };

        let mut result: Vec<_> = rows.collect::<Result<Vec<_>, _>>()?;
        result.reverse();
        Ok(result)
    }

    /// Get daily statistics for all recorded days.
    ///
    /// Returns one [`DayStats`] per day with commands executed, tokens saved,
    /// and execution time metrics. Results are ordered chronologically (oldest first).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use rtk::tracking::{QueryScope, Tracker};
    ///
    /// let tracker = Tracker::new()?;
    /// let days = tracker.get_all_days(&QueryScope::Global)?;
    /// for day in days.iter().take(7) {
    ///     println!("{}: {} commands, {} tokens saved",
    ///         day.date, day.commands, day.saved_tokens);
    /// }
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn get_all_days(&self, scope: &QueryScope) -> Result<Vec<DayStats>> {
        let (where_clause, scope_param) = scope_filter(scope);
        let sql = format!(
            "SELECT
                DATE(timestamp) as date,
                COUNT(*) as commands,
                SUM(input_tokens) as input,
                SUM(output_tokens) as output,
                SUM(saved_tokens) as saved,
                SUM(exec_time_ms) as total_time
             FROM commands{}
             GROUP BY DATE(timestamp)
             ORDER BY DATE(timestamp) DESC",
            where_clause
        );
        let mut stmt = self.conn.prepare(&sql)?;

        let map_row = |row: &rusqlite::Row| -> rusqlite::Result<DayStats> {
            let input = row.get::<_, i64>(2)? as usize;
            let saved = row.get::<_, i64>(4)? as usize;
            let commands = row.get::<_, i64>(1)? as usize;
            let total_time = row.get::<_, i64>(5)? as u64;
            let savings_pct = if input > 0 {
                (saved as f64 / input as f64) * 100.0
            } else {
                0.0
            };
            let avg_time_ms = if commands > 0 {
                total_time / commands as u64
            } else {
                0
            };

            Ok(DayStats {
                date: row.get(0)?,
                commands,
                input_tokens: input,
                output_tokens: row.get::<_, i64>(3)? as usize,
                saved_tokens: saved,
                savings_pct,
                total_time_ms: total_time,
                avg_time_ms,
            })
        };

        let rows = if let Some(ref dir) = scope_param {
            stmt.query_map(params![dir], &map_row)?
        } else {
            stmt.query_map([], &map_row)?
        };

        let mut result: Vec<_> = rows.collect::<Result<Vec<_>, _>>()?;
        result.reverse();
        Ok(result)
    }

    /// Get weekly statistics grouped by week.
    ///
    /// Returns one [`WeekStats`] per week with aggregated metrics.
    /// Weeks start on Sunday (SQLite default). Results ordered chronologically.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use rtk::tracking::{QueryScope, Tracker};
    ///
    /// let tracker = Tracker::new()?;
    /// let weeks = tracker.get_by_week(&QueryScope::Global)?;
    /// for week in weeks {
    ///     println!("{} to {}: {} tokens saved",
    ///         week.week_start, week.week_end, week.saved_tokens);
    /// }
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn get_by_week(&self, scope: &QueryScope) -> Result<Vec<WeekStats>> {
        let (where_clause, scope_param) = scope_filter(scope);
        let sql = format!(
            "SELECT
                DATE(timestamp, 'weekday 0', '-6 days') as week_start,
                DATE(timestamp, 'weekday 0') as week_end,
                COUNT(*) as commands,
                SUM(input_tokens) as input,
                SUM(output_tokens) as output,
                SUM(saved_tokens) as saved,
                SUM(exec_time_ms) as total_time
             FROM commands{}
             GROUP BY week_start
             ORDER BY week_start DESC",
            where_clause
        );
        let mut stmt = self.conn.prepare(&sql)?;

        let map_row = |row: &rusqlite::Row| -> rusqlite::Result<WeekStats> {
            let input = row.get::<_, i64>(3)? as usize;
            let saved = row.get::<_, i64>(5)? as usize;
            let commands = row.get::<_, i64>(2)? as usize;
            let total_time = row.get::<_, i64>(6)? as u64;
            let savings_pct = if input > 0 {
                (saved as f64 / input as f64) * 100.0
            } else {
                0.0
            };
            let avg_time_ms = if commands > 0 {
                total_time / commands as u64
            } else {
                0
            };

            Ok(WeekStats {
                week_start: row.get(0)?,
                week_end: row.get(1)?,
                commands,
                input_tokens: input,
                output_tokens: row.get::<_, i64>(4)? as usize,
                saved_tokens: saved,
                savings_pct,
                total_time_ms: total_time,
                avg_time_ms,
            })
        };

        let rows = if let Some(ref dir) = scope_param {
            stmt.query_map(params![dir], &map_row)?
        } else {
            stmt.query_map([], &map_row)?
        };

        let mut result: Vec<_> = rows.collect::<Result<Vec<_>, _>>()?;
        result.reverse();
        Ok(result)
    }

    /// Get monthly statistics grouped by month.
    ///
    /// Returns one [`MonthStats`] per month (YYYY-MM format) with aggregated metrics.
    /// Results ordered chronologically.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use rtk::tracking::{QueryScope, Tracker};
    ///
    /// let tracker = Tracker::new()?;
    /// let months = tracker.get_by_month(&QueryScope::Global)?;
    /// for month in months {
    ///     println!("{}: {} tokens saved ({:.1}%)",
    ///         month.month, month.saved_tokens, month.savings_pct);
    /// }
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn get_by_month(&self, scope: &QueryScope) -> Result<Vec<MonthStats>> {
        let (where_clause, scope_param) = scope_filter(scope);
        let sql = format!(
            "SELECT
                strftime('%Y-%m', timestamp) as month,
                COUNT(*) as commands,
                SUM(input_tokens) as input,
                SUM(output_tokens) as output,
                SUM(saved_tokens) as saved,
                SUM(exec_time_ms) as total_time
             FROM commands{}
             GROUP BY month
             ORDER BY month DESC",
            where_clause
        );
        let mut stmt = self.conn.prepare(&sql)?;

        let map_row = |row: &rusqlite::Row| -> rusqlite::Result<MonthStats> {
            let input = row.get::<_, i64>(2)? as usize;
            let saved = row.get::<_, i64>(4)? as usize;
            let commands = row.get::<_, i64>(1)? as usize;
            let total_time = row.get::<_, i64>(5)? as u64;
            let savings_pct = if input > 0 {
                (saved as f64 / input as f64) * 100.0
            } else {
                0.0
            };
            let avg_time_ms = if commands > 0 {
                total_time / commands as u64
            } else {
                0
            };

            Ok(MonthStats {
                month: row.get(0)?,
                commands,
                input_tokens: input,
                output_tokens: row.get::<_, i64>(3)? as usize,
                saved_tokens: saved,
                savings_pct,
                total_time_ms: total_time,
                avg_time_ms,
            })
        };

        let rows = if let Some(ref dir) = scope_param {
            stmt.query_map(params![dir], &map_row)?
        } else {
            stmt.query_map([], &map_row)?
        };

        let mut result: Vec<_> = rows.collect::<Result<Vec<_>, _>>()?;
        result.reverse();
        Ok(result)
    }

    /// Get recent command history.
    ///
    /// Returns up to `limit` most recent command records, ordered by timestamp (newest first).
    ///
    /// # Arguments
    ///
    /// - `limit`: Maximum number of records to return
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use rtk::tracking::{QueryScope, Tracker};
    ///
    /// let tracker = Tracker::new()?;
    /// let recent = tracker.get_recent(10, &QueryScope::Global)?;
    /// for cmd in recent {
    ///     println!("{}: {} saved {:.1}%",
    ///         cmd.timestamp, cmd.rtk_cmd, cmd.savings_pct);
    /// }
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn get_recent(&self, limit: usize, scope: &QueryScope) -> Result<Vec<CommandRecord>> {
        let (where_clause, scope_param) = scope_filter(scope);
        let limit_param = if scope_param.is_some() { "?2" } else { "?1" };
        let sql = format!(
            "SELECT timestamp, rtk_cmd, saved_tokens, savings_pct
             FROM commands{}
             ORDER BY timestamp DESC
             LIMIT {}",
            where_clause, limit_param
        );
        let mut stmt = self.conn.prepare(&sql)?;

        let map_row = |row: &rusqlite::Row| -> rusqlite::Result<CommandRecord> {
            Ok(CommandRecord {
                timestamp: DateTime::parse_from_rfc3339(&row.get::<_, String>(0)?)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                rtk_cmd: row.get(1)?,
                saved_tokens: row.get::<_, i64>(2)? as usize,
                savings_pct: row.get(3)?,
            })
        };

        let rows = if let Some(ref dir) = scope_param {
            stmt.query_map(params![dir, limit as i64], &map_row)?
        } else {
            stmt.query_map(params![limit as i64], &map_row)?
        };

        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

/// Build a SQL WHERE clause and optional parameter for scope filtering.
///
/// Returns `(" WHERE working_dir = ?1", Some(dir))` for Project scope,
/// or `("", None)` for Global scope. For queries that already have parameters,
/// the caller must adjust parameter numbering.
fn scope_filter(scope: &QueryScope) -> (String, Option<String>) {
    match scope {
        QueryScope::Project(dir) => (" WHERE working_dir = ?1".to_string(), Some(dir.clone())),
        QueryScope::Global => (String::new(), None),
    }
}

fn get_db_path() -> Result<PathBuf> {
    // Priority 1: Environment variable RTK_DB_PATH
    if let Ok(custom_path) = std::env::var("RTK_DB_PATH") {
        return Ok(PathBuf::from(custom_path));
    }

    // Priority 2: Configuration file
    if let Ok(config) = crate::config::Config::load() {
        if let Some(db_path) = config.tracking.database_path {
            return Ok(db_path);
        }
    }

    // Priority 3: Default platform-specific location
    let data_dir = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    Ok(data_dir.join("rtk").join("history.db"))
}

/// Estimate token count from text using ~4 chars = 1 token heuristic.
///
/// This is a fast approximation suitable for tracking purposes.
/// For precise counts, integrate with your LLM's tokenizer API.
///
/// # Formula
///
/// `tokens = ceil(chars / 4)`
///
/// # Examples
///
/// ```
/// use rtk::tracking::estimate_tokens;
///
/// assert_eq!(estimate_tokens(""), 0);
/// assert_eq!(estimate_tokens("abcd"), 1);  // 4 chars = 1 token
/// assert_eq!(estimate_tokens("abcde"), 2); // 5 chars = ceil(1.25) = 2
/// assert_eq!(estimate_tokens("hello world"), 3); // 11 chars = ceil(2.75) = 3
/// ```
pub fn estimate_tokens(text: &str) -> usize {
    // ~4 chars per token on average
    (text.len() as f64 / 4.0).ceil() as usize
}

/// Helper struct for timing command execution
/// Helper for timing command execution and tracking results.
///
/// Preferred API for tracking commands. Automatically measures execution time
/// and records token savings. Use instead of the deprecated [`track`] function.
///
/// # Examples
///
/// ```no_run
/// use rtk::tracking::TimedExecution;
///
/// let timer = TimedExecution::start();
/// let input = execute_standard_command()?;
/// let output = execute_rtk_command()?;
/// timer.track("ls -la", "rtk ls", &input, &output);
/// # Ok::<(), anyhow::Error>(())
/// ```
pub struct TimedExecution {
    start: Instant,
}

impl TimedExecution {
    /// Start timing a command execution.
    ///
    /// Creates a new timer that starts measuring elapsed time immediately.
    /// Call [`track`](Self::track) or [`track_passthrough`](Self::track_passthrough)
    /// when the command completes.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use rtk::tracking::TimedExecution;
    ///
    /// let timer = TimedExecution::start();
    /// // ... execute command ...
    /// timer.track("cmd", "rtk cmd", "input", "output");
    /// ```
    pub fn start() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    /// Track the command with elapsed time and token counts.
    ///
    /// Records the command execution with:
    /// - Elapsed time since [`start`](Self::start)
    /// - Token counts estimated from input/output strings
    /// - Calculated savings metrics
    ///
    /// # Arguments
    ///
    /// - `original_cmd`: Standard command (e.g., "ls -la")
    /// - `rtk_cmd`: RTK command used (e.g., "rtk ls")
    /// - `input`: Standard command output (for token estimation)
    /// - `output`: RTK command output (for token estimation)
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use rtk::tracking::TimedExecution;
    ///
    /// let timer = TimedExecution::start();
    /// let input = "long output...";
    /// let output = "short output";
    /// timer.track("ls -la", "rtk ls", input, output);
    /// ```
    pub fn track(&self, original_cmd: &str, rtk_cmd: &str, input: &str, output: &str) {
        let elapsed_ms = self.start.elapsed().as_millis() as u64;
        let input_tokens = estimate_tokens(input);
        let output_tokens = estimate_tokens(output);
        let working_dir = detect_project_root();

        if let Ok(tracker) = Tracker::new() {
            let _ = tracker.record(
                original_cmd,
                rtk_cmd,
                input_tokens,
                output_tokens,
                elapsed_ms,
                &working_dir,
            );
        }
    }

    /// Track passthrough commands (timing-only, no token counting).
    ///
    /// For commands that stream output or run interactively where output
    /// cannot be captured. Records execution time but sets tokens to 0
    /// (does not dilute savings statistics).
    ///
    /// # Arguments
    ///
    /// - `original_cmd`: Standard command (e.g., "git tag --list")
    /// - `rtk_cmd`: RTK command used (e.g., "rtk git tag --list")
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use rtk::tracking::TimedExecution;
    ///
    /// let timer = TimedExecution::start();
    /// // ... execute streaming command ...
    /// timer.track_passthrough("git tag", "rtk git tag");
    /// ```
    pub fn track_passthrough(&self, original_cmd: &str, rtk_cmd: &str) {
        let elapsed_ms = self.start.elapsed().as_millis() as u64;
        let working_dir = detect_project_root();
        // input_tokens=0, output_tokens=0 won't dilute savings statistics
        if let Ok(tracker) = Tracker::new() {
            let _ = tracker.record(original_cmd, rtk_cmd, 0, 0, elapsed_ms, &working_dir);
        }
    }
}

/// Format OsString args for tracking display.
///
/// Joins arguments with spaces, converting each to UTF-8 (lossy).
/// Useful for displaying command arguments in tracking records.
///
/// # Examples
///
/// ```
/// use std::ffi::OsString;
/// use rtk::tracking::args_display;
///
/// let args = vec![OsString::from("status"), OsString::from("--short")];
/// assert_eq!(args_display(&args), "status --short");
/// ```
pub fn args_display(args: &[OsString]) -> String {
    args.iter()
        .map(|a| a.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex;

    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    fn test_tracker() -> (Tracker, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let tracker = Tracker::with_path(&db_path).unwrap();
        (tracker, dir)
    }

    // 1. estimate_tokens — verify ~4 chars/token ratio
    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1); // 4 chars = 1 token
        assert_eq!(estimate_tokens("abcde"), 2); // 5 chars = ceil(1.25) = 2
        assert_eq!(estimate_tokens("a"), 1); // 1 char = ceil(0.25) = 1
        assert_eq!(estimate_tokens("12345678"), 2); // 8 chars = 2 tokens
    }

    // 2. args_display — format OsString vec
    #[test]
    fn test_args_display() {
        let args = vec![OsString::from("status"), OsString::from("--short")];
        assert_eq!(args_display(&args), "status --short");
        assert_eq!(args_display(&[]), "");

        let single = vec![OsString::from("log")];
        assert_eq!(args_display(&single), "log");
    }

    // 3. Tracker::record + get_recent — round-trip DB (temp DB)
    #[test]
    fn test_tracker_record_and_recent() {
        let (tracker, _dir) = test_tracker();

        tracker
            .record("git status", "rtk git status", 100, 20, 50, "/projects/foo")
            .expect("Failed to record");

        let recent = tracker
            .get_recent(10, &QueryScope::Global)
            .expect("Failed to get recent");
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].rtk_cmd, "rtk git status");
        assert_eq!(recent[0].saved_tokens, 80);
        assert_eq!(recent[0].savings_pct, 80.0);
    }

    // 4. track_passthrough doesn't dilute stats (input=0, output=0)
    #[test]
    fn test_track_passthrough_no_dilution() {
        let (tracker, _dir) = test_tracker();

        tracker
            .record("cmd1", "rtk cmd1", 1000, 200, 10, "/projects/foo")
            .expect("Failed to record cmd1");

        tracker
            .record("cmd2", "rtk cmd2 passthrough", 0, 0, 5, "/projects/foo")
            .expect("Failed to record passthrough");

        let recent = tracker
            .get_recent(20, &QueryScope::Global)
            .expect("Failed to get recent");
        assert_eq!(recent.len(), 2);

        let record1 = recent
            .iter()
            .find(|r| r.rtk_cmd == "rtk cmd1")
            .expect("cmd1 record not found");
        let record2 = recent
            .iter()
            .find(|r| r.rtk_cmd == "rtk cmd2 passthrough")
            .expect("passthrough record not found");

        assert_eq!(record1.saved_tokens, 800);
        assert_eq!(record1.savings_pct, 80.0);

        assert_eq!(record2.saved_tokens, 0);
        assert_eq!(record2.savings_pct, 0.0);
    }

    // 5. TimedExecution::track records with exec_time > 0 (temp DB via env var)
    #[test]
    fn test_timed_execution_records_time() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        std::env::set_var("RTK_DB_PATH", &db_path);

        let timer = TimedExecution::start();
        std::thread::sleep(std::time::Duration::from_millis(10));
        timer.track("test cmd", "rtk test", "raw input data", "filtered");

        let tracker = Tracker::with_path(&db_path).expect("Failed to create tracker");
        let recent = tracker
            .get_recent(5, &QueryScope::Global)
            .expect("Failed to get recent");
        assert!(recent.iter().any(|r| r.rtk_cmd == "rtk test"));

        std::env::remove_var("RTK_DB_PATH");
    }

    // 6. TimedExecution::track_passthrough records with 0 tokens (temp DB via env var)
    #[test]
    fn test_timed_execution_passthrough() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        std::env::set_var("RTK_DB_PATH", &db_path);

        let timer = TimedExecution::start();
        timer.track_passthrough("git tag", "rtk git tag (passthrough)");

        let tracker = Tracker::with_path(&db_path).expect("Failed to create tracker");
        let recent = tracker
            .get_recent(5, &QueryScope::Global)
            .expect("Failed to get recent");

        let pt = recent
            .iter()
            .find(|r| r.rtk_cmd.contains("passthrough"))
            .expect("Passthrough record not found");

        assert_eq!(pt.savings_pct, 0.0);
        assert_eq!(pt.saved_tokens, 0);

        std::env::remove_var("RTK_DB_PATH");
    }

    // 7. get_db_path respects environment variable RTK_DB_PATH
    #[test]
    fn test_custom_db_path_env() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let custom_path = "/tmp/rtk_test_custom.db";
        std::env::set_var("RTK_DB_PATH", custom_path);

        let db_path = get_db_path().expect("Failed to get db path");
        assert_eq!(db_path, PathBuf::from(custom_path));

        std::env::remove_var("RTK_DB_PATH");
    }

    // 8. get_db_path falls back to default when no custom config
    #[test]
    fn test_default_db_path() {
        let _lock = ENV_MUTEX.lock().unwrap();
        std::env::remove_var("RTK_DB_PATH");

        let db_path = get_db_path().expect("Failed to get db path");
        assert!(db_path.ends_with("rtk/history.db"));
    }

    // 9. detect_project_root finds .git directory
    #[test]
    fn test_detect_project_root_git() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        let sub = dir.path().join("src");
        std::fs::create_dir(&sub).unwrap();

        let root = detect_project_root_from(&sub);
        let expected = std::fs::canonicalize(dir.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(root, expected);
    }

    // 10. detect_project_root falls back to empty when no markers
    #[test]
    fn test_detect_project_root_no_markers() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("empty");
        std::fs::create_dir(&sub).unwrap();

        let root = detect_project_root_from(&sub);
        assert!(root.is_empty(), "Expected empty, got: {}", root);
    }

    // 11. working_dir is recorded in the database
    #[test]
    fn test_working_dir_recorded() {
        let (tracker, _dir) = test_tracker();

        tracker
            .record(
                "git status",
                "rtk git status",
                100,
                20,
                50,
                "/projects/myapp",
            )
            .expect("Failed to record");

        let mut stmt = tracker
            .conn
            .prepare("SELECT working_dir FROM commands WHERE rtk_cmd = 'rtk git status'")
            .unwrap();
        let dir: String = stmt.query_row([], |row| row.get(0)).unwrap();
        assert_eq!(dir, "/projects/myapp");
    }

    // 12. project scope filters to matching records only
    #[test]
    fn test_project_scope_filters() {
        let (tracker, _dir) = test_tracker();

        tracker
            .record("cmd1", "rtk cmd1", 100, 20, 5, "/projects/foo")
            .unwrap();
        tracker
            .record("cmd2", "rtk cmd2", 200, 40, 5, "/projects/bar")
            .unwrap();
        tracker
            .record("cmd3", "rtk cmd3", 300, 60, 5, "/projects/foo")
            .unwrap();

        let scope_foo = QueryScope::Project("/projects/foo".to_string());
        let summary = tracker.get_summary(&scope_foo, 10).unwrap();
        assert_eq!(summary.total_commands, 2);
        assert_eq!(summary.total_input, 400); // 100 + 300

        let recent = tracker.get_recent(10, &scope_foo).unwrap();
        assert_eq!(recent.len(), 2);
        assert!(recent.iter().all(|r| r.rtk_cmd != "rtk cmd2"));
    }

    // 13. global scope returns all records
    #[test]
    fn test_global_scope_returns_all() {
        let (tracker, _dir) = test_tracker();

        tracker
            .record("cmd1", "rtk cmd1", 100, 20, 5, "/projects/foo")
            .unwrap();
        tracker
            .record("cmd2", "rtk cmd2", 200, 40, 5, "/projects/bar")
            .unwrap();

        let summary = tracker.get_summary(&QueryScope::Global, 20).unwrap();
        assert_eq!(summary.total_commands, 2);
    }

    // 14. old records (working_dir='') appear in global but not project scope
    #[test]
    fn test_old_records_global_only() {
        let (tracker, _dir) = test_tracker();

        // Simulate old record with empty working_dir
        tracker
            .record("old cmd", "rtk old", 100, 20, 5, "")
            .unwrap();
        tracker
            .record("new cmd", "rtk new", 200, 40, 5, "/projects/foo")
            .unwrap();

        let global = tracker.get_summary(&QueryScope::Global, 20).unwrap();
        assert_eq!(global.total_commands, 2);

        let project = tracker
            .get_summary(&QueryScope::Project("/projects/foo".to_string()), 10)
            .unwrap();
        assert_eq!(project.total_commands, 1);
    }

    // 15. top_n limit works correctly
    #[test]
    fn test_top_n_limit() {
        let (tracker, _dir) = test_tracker();

        for i in 0..5 {
            tracker
                .record(
                    &format!("cmd{}", i),
                    &format!("rtk cmd{}", i),
                    100 * (i + 1),
                    20,
                    5,
                    "/projects/foo",
                )
                .unwrap();
        }

        let scope = QueryScope::Project("/projects/foo".to_string());
        let summary_2 = tracker.get_summary(&scope, 2).unwrap();
        assert_eq!(summary_2.by_command.len(), 2);

        let summary_10 = tracker.get_summary(&scope, 10).unwrap();
        assert_eq!(summary_10.by_command.len(), 5);
    }
}
