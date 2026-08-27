use ahash::AHashMap as HashMap;
use chrono::Local;
use clap::Args;
use clap::ValueEnum;
use color_eyre::Result;
use comfy_table::Table;
use comfy_table::presets::UTF8_FULL;
use envx_core::{EnvScope, EnvVarManager};
use serde::Serialize;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, ValueEnum)]
pub enum OutputFormat {
    /// Live terminal output
    Live,
    /// Compact output
    Compact,
    /// JSON lines format
    JsonLines,
}

#[derive(Debug, Clone, ValueEnum)]
pub enum SourceFilter {
    #[value(name = "system")]
    System,
    #[value(name = "user")]
    User,
    #[value(name = "process")]
    Process,
}

impl From<SourceFilter> for EnvScope {
    fn from(filter: SourceFilter) -> Self {
        match filter {
            SourceFilter::System => EnvScope::System,
            SourceFilter::User => EnvScope::User,
            SourceFilter::Process => EnvScope::Process,
        }
    }
}

#[derive(Args)]
pub struct MonitorArgs {
    /// Variables to monitor (monitor all if not specified)
    #[arg(value_name = "VARIABLE")]
    pub vars: Vec<String>,

    /// Log file path
    #[arg(short, long)]
    pub log: Option<PathBuf>,

    /// Show only changes (hide unchanged variables)
    #[arg(long)]
    pub changes_only: bool,

    /// Filter by scope
    #[arg(short, long, value_enum)]
    pub scope: Option<SourceFilter>,

    /// Output format
    #[arg(short, long, value_enum, default_value = "live")]
    pub format: OutputFormat,

    /// Check interval in seconds
    #[arg(long, default_value = "2")]
    pub interval: u64,

    /// Show initial state
    #[arg(long)]
    pub show_initial: bool,

    /// Export report on exit
    #[arg(long)]
    pub export_report: Option<PathBuf>,

    /// Reveal variable values in terminal and output files
    #[arg(long)]
    pub reveal: bool,
}

struct MonitorState {
    initial: HashMap<String, String>,
    current: HashMap<String, String>,
    changes: Vec<ChangeRecord>,
    start_time: chrono::DateTime<Local>,
}

#[derive(Debug, Clone, Serialize)]
struct ChangeRecord {
    timestamp: chrono::DateTime<Local>,
    variable: String,
    change_type: String,
    old_value: Option<String>,
    new_value: Option<String>,
}

/// Handles the monitor command to track environment variable changes.
///
/// # Errors
///
/// Returns an error if:
/// - Failed to load environment variables
/// - Failed to set up Ctrl+C handler
/// - Failed to write to log file (if specified)
/// - Failed to export report (if specified)
pub fn handle_monitor(args: MonitorArgs) -> Result<()> {
    let mut manager = EnvVarManager::new();
    manager.load_all()?;

    let mut state = MonitorState {
        initial: collect_variables(&manager, &args),
        current: HashMap::new(),
        changes: Vec::new(),
        start_time: Local::now(),
    };

    print_monitor_header(&args);

    if args.show_initial {
        print_initial_state(&state.initial, args.reveal);
    }

    // Set up Ctrl+C handler
    let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        r.store(false, std::sync::atomic::Ordering::SeqCst);
    })?;

    // Monitoring loop
    while running.load(std::sync::atomic::Ordering::SeqCst) {
        std::thread::sleep(Duration::from_secs(args.interval));

        let mut current_manager = EnvVarManager::new();
        current_manager.load_all()?;

        state.current = collect_variables(&current_manager, &args);

        let changes = detect_changes(&state);

        if !changes.is_empty() || !args.changes_only {
            display_changes(&changes, &args);

            // Log changes
            for change in changes {
                state.changes.push(change.clone());

                if let Some(log_path) = &args.log {
                    log_change(log_path, &change, args.reveal)?;
                }
            }
        }

        // Update state for next iteration
        for (name, value) in &state.current {
            state.initial.insert(name.clone(), value.clone());
        }
    }

    // Generate final report if requested
    if let Some(report_path) = args.export_report {
        export_report(&state, &report_path, args.reveal)?;
        println!("\n📊 Report exported to: {}", report_path.display());
    }

    print_monitor_summary(&state);

    Ok(())
}

fn collect_variables(manager: &EnvVarManager, args: &MonitorArgs) -> HashMap<String, String> {
    manager
        .list(args.scope.clone().map(Into::into))
        .into_iter()
        .filter(|var| {
            // Filter by variable names if specified
            (args.vars.is_empty() || args.vars.iter().any(|v| var.name.contains(v))) &&
            // Filter by scope if specified
            (args.scope.is_none() || args.scope.as_ref().map(|s| EnvScope::from(s.clone())) == Some(var.scope))
        })
        .map(|var| (format!("{}:{}", var.scope, var.name), var.value.clone()))
        .collect()
}

fn detect_changes(state: &MonitorState) -> Vec<ChangeRecord> {
    let mut changes = Vec::new();
    let timestamp = Local::now();

    // Check for modifications and additions
    for (name, value) in &state.current {
        match state.initial.get(name) {
            Some(old_value) if old_value != value => {
                changes.push(ChangeRecord {
                    timestamp,
                    variable: name.clone(),
                    change_type: "modified".to_string(),
                    old_value: Some(old_value.clone()),
                    new_value: Some(value.clone()),
                });
            }
            None => {
                changes.push(ChangeRecord {
                    timestamp,
                    variable: name.clone(),
                    change_type: "added".to_string(),
                    old_value: None,
                    new_value: Some(value.clone()),
                });
            }
            _ => {} // No change
        }
    }

    // Check for deletions
    for (name, value) in &state.initial {
        if !state.current.contains_key(name) {
            changes.push(ChangeRecord {
                timestamp,
                variable: name.clone(),
                change_type: "deleted".to_string(),
                old_value: Some(value.clone()),
                new_value: None,
            });
        }
    }

    changes
}

fn display_changes(changes: &[ChangeRecord], args: &MonitorArgs) {
    match args.format {
        OutputFormat::Live => {
            for change in changes.iter().map(|change| output_change(change, args.reveal)) {
                let time = change.timestamp.format("%H:%M:%S");
                match change.change_type.as_str() {
                    "added" => {
                        println!(
                            "[{}] ➕ {} = '{}'",
                            time,
                            change.variable,
                            change.new_value.as_ref().unwrap_or(&String::new())
                        );
                    }
                    "modified" => {
                        println!(
                            "[{}] 🔄 {} changed from '{}' to '{}'",
                            time,
                            change.variable,
                            change.old_value.as_ref().unwrap_or(&String::new()),
                            change.new_value.as_ref().unwrap_or(&String::new())
                        );
                    }
                    "deleted" => {
                        println!(
                            "[{}] ❌ {} deleted (was: '{}')",
                            time,
                            change.variable,
                            change.old_value.as_ref().unwrap_or(&String::new())
                        );
                    }
                    _ => {}
                }
            }
        }
        OutputFormat::Compact => {
            for change in changes.iter().map(|change| output_change(change, args.reveal)) {
                println!(
                    "{} {} {}",
                    change.timestamp.format("%Y-%m-%d %H:%M:%S"),
                    change.change_type.to_uppercase(),
                    change.variable
                );
            }
        }
        OutputFormat::JsonLines => {
            for change in changes.iter().map(|change| output_change(change, args.reveal)) {
                if let Ok(json) = serde_json::to_string(&change) {
                    println!("{json}");
                }
            }
        }
    }
}

fn log_change(path: &PathBuf, change: &ChangeRecord, reveal: bool) -> Result<()> {
    use std::fs::OpenOptions;
    use std::io::Write;

    let mut file = OpenOptions::new().create(true).append(true).open(path)?;

    writeln!(file, "{}", serde_json::to_string(&output_change(change, reveal))?)?;
    Ok(())
}

fn print_monitor_header(args: &MonitorArgs) {
    println!("📊 Environment Variable Monitor");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    if args.vars.is_empty() {
        println!("Monitoring: All variables");
    } else {
        println!("Monitoring: {}", args.vars.join(", "));
    }

    if let Some(scope) = &args.scope {
        println!("Scope filter: {scope:?}");
    }

    println!("Check interval: {} seconds", args.interval);
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("Press Ctrl+C to stop\n");
}

fn print_initial_state(vars: &HashMap<String, String>, reveal: bool) {
    if vars.is_empty() {
        println!("No variables match the criteria\n");
        return;
    }

    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_header(vec!["Variable", "Initial Value"]);

    for (name, value) in vars {
        let display_value = if reveal {
            value.clone()
        } else {
            "[REDACTED]".to_string()
        };
        table.add_row(vec![safe_scoped_name(name), display_value]);
    }

    println!("Initial State:\n{table}\n");
}

fn print_monitor_summary(state: &MonitorState) {
    let duration = Local::now().signed_duration_since(state.start_time);

    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("📊 Monitoring Summary");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("Duration: {}", format_duration(duration));
    println!("Total changes: {}", state.changes.len());

    let mut added = 0;
    let mut modified = 0;
    let mut deleted = 0;

    for change in &state.changes {
        match change.change_type.as_str() {
            "added" => added += 1,
            "modified" => modified += 1,
            "deleted" => deleted += 1,
            _ => {}
        }
    }

    println!("  ➕ Added: {added}");
    println!("  🔄 Modified: {modified}");
    println!("  ❌ Deleted: {deleted}");
}

fn format_duration(duration: chrono::Duration) -> String {
    let hours = duration.num_hours();
    let minutes = duration.num_minutes() % 60;
    let seconds = duration.num_seconds() % 60;

    if hours > 0 {
        format!("{hours}h {minutes}m {seconds}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

fn export_report(state: &MonitorState, path: &PathBuf, reveal: bool) -> Result<()> {
    #[derive(Serialize)]
    struct Report {
        start_time: chrono::DateTime<Local>,
        end_time: chrono::DateTime<Local>,
        duration_seconds: i64,
        total_changes: usize,
        changes_by_type: HashMap<String, usize>,
        changes: Vec<ChangeRecord>,
    }

    let mut changes_by_type = HashMap::new();
    for change in &state.changes {
        *changes_by_type.entry(change.change_type.clone()).or_insert(0) += 1;
    }

    let report = Report {
        start_time: state.start_time,
        end_time: Local::now(),
        duration_seconds: Local::now().signed_duration_since(state.start_time).num_seconds(),
        total_changes: state.changes.len(),
        changes_by_type,
        changes: state
            .changes
            .iter()
            .map(|change| output_change(change, reveal))
            .collect(),
    };

    let json = serde_json::to_string_pretty(&report)?;
    std::fs::write(path, json)?;

    Ok(())
}

fn output_change(change: &ChangeRecord, reveal: bool) -> ChangeRecord {
    ChangeRecord {
        timestamp: change.timestamp,
        variable: safe_scoped_name(&change.variable),
        change_type: change.change_type.clone(),
        old_value: change.old_value.as_ref().map(|value| {
            if reveal {
                value.clone()
            } else {
                "[REDACTED]".to_string()
            }
        }),
        new_value: change.new_value.as_ref().map(|value| {
            if reveal {
                value.clone()
            } else {
                "[REDACTED]".to_string()
            }
        }),
    }
}

fn safe_scoped_name(name: &str) -> String {
    name.split_once(':').map_or_else(
        || crate::list::safe_name(name),
        |(scope, variable)| format!("{scope}:{}", crate::list::safe_name(variable)),
    )
}

// Add this at the end of the file

#[cfg(test)]
mod tests {
    use super::*;
    use ahash::AHashMap as HashMap;
    use envx_core::{EnvKey, EnvValueKind, EnvVar, EnvVarManager};

    // Helper function to create a test environment variable
    fn create_test_env_var(name: &str, value: &str, scope: EnvScope) -> EnvVar {
        EnvVar {
            name: name.to_string(),
            value: value.to_string(),
            scope,
            kind: EnvValueKind::String,
            modified: chrono::Utc::now(),
            original_value: None,
        }
    }

    fn insert_test_env_var(manager: &mut EnvVarManager, name: &str, value: &str, scope: EnvScope) {
        manager
            .vars
            .insert(EnvKey::new(scope, name), create_test_env_var(name, value, scope));
    }

    // Helper function to create a test manager with predefined variables
    fn create_test_manager() -> EnvVarManager {
        let mut manager = EnvVarManager::new();

        // Add test variables with different scopes.
        insert_test_env_var(&mut manager, "SYSTEM_VAR", "system_value", EnvScope::System);
        insert_test_env_var(&mut manager, "USER_VAR", "user_value", EnvScope::User);
        insert_test_env_var(&mut manager, "PROCESS_VAR", "process_value", EnvScope::Process);
        insert_test_env_var(&mut manager, "SHELL_VAR", "shell_value", EnvScope::Process);
        insert_test_env_var(&mut manager, "APP_VAR", "app_value", EnvScope::Process);
        insert_test_env_var(&mut manager, "TEST_API_KEY", "secret123", EnvScope::User);
        insert_test_env_var(
            &mut manager,
            "DATABASE_URL",
            "postgres://localhost:5432",
            EnvScope::User,
        );

        manager
    }

    #[test]
    fn test_collect_variables_all() {
        let manager = create_test_manager();
        let args = MonitorArgs {
            vars: vec![],
            log: None,
            changes_only: false,
            scope: None,
            format: OutputFormat::Live,
            interval: 2,
            show_initial: false,
            export_report: None,
            reveal: false,
        };

        let result = collect_variables(&manager, &args);

        // Should collect all variables
        assert_eq!(result.len(), 7);
        assert_eq!(result.get("system:SYSTEM_VAR"), Some(&"system_value".to_string()));
        assert_eq!(result.get("user:USER_VAR"), Some(&"user_value".to_string()));
        assert_eq!(result.get("process:PROCESS_VAR"), Some(&"process_value".to_string()));
        assert_eq!(result.get("process:SHELL_VAR"), Some(&"shell_value".to_string()));
        assert_eq!(result.get("process:APP_VAR"), Some(&"app_value".to_string()));
        assert_eq!(result.get("user:TEST_API_KEY"), Some(&"secret123".to_string()));
        assert_eq!(
            result.get("user:DATABASE_URL"),
            Some(&"postgres://localhost:5432".to_string())
        );
    }

    #[test]
    fn test_collect_variables_with_name_filter() {
        let manager = create_test_manager();
        let args = MonitorArgs {
            vars: vec!["API".to_string(), "DATABASE".to_string()],
            log: None,
            changes_only: false,
            scope: None,
            format: OutputFormat::Live,
            interval: 2,
            show_initial: false,
            export_report: None,
            reveal: false,
        };

        let result = collect_variables(&manager, &args);

        // Should only collect variables containing "API" or "DATABASE"
        assert_eq!(result.len(), 2);
        assert_eq!(result.get("user:TEST_API_KEY"), Some(&"secret123".to_string()));
        assert_eq!(
            result.get("user:DATABASE_URL"),
            Some(&"postgres://localhost:5432".to_string())
        );
        assert!(!result.contains_key("system:SYSTEM_VAR"));
    }

    #[test]
    fn test_collect_variables_with_source_filter() {
        let manager = create_test_manager();
        let args = MonitorArgs {
            vars: vec![],
            log: None,
            changes_only: false,
            scope: Some(SourceFilter::User),
            format: OutputFormat::Live,
            interval: 2,
            show_initial: false,
            export_report: None,
            reveal: false,
        };

        let result = collect_variables(&manager, &args);

        // Should only collect User source variables
        assert_eq!(result.len(), 3);
        assert_eq!(result.get("user:USER_VAR"), Some(&"user_value".to_string()));
        assert_eq!(result.get("user:TEST_API_KEY"), Some(&"secret123".to_string()));
        assert_eq!(
            result.get("user:DATABASE_URL"),
            Some(&"postgres://localhost:5432".to_string())
        );
        assert!(!result.contains_key("system:SYSTEM_VAR"));
        assert!(!result.contains_key("process:PROCESS_VAR"));
    }

    #[test]
    fn test_collect_variables_with_combined_filters() {
        let manager = create_test_manager();
        let args = MonitorArgs {
            vars: vec!["VAR".to_string()],
            log: None,
            changes_only: false,
            scope: Some(SourceFilter::System),
            format: OutputFormat::Live,
            interval: 2,
            show_initial: false,
            export_report: None,
            reveal: false,
        };

        let result = collect_variables(&manager, &args);

        // Should only collect System source variables containing "VAR"
        assert_eq!(result.len(), 1);
        assert_eq!(result.get("system:SYSTEM_VAR"), Some(&"system_value".to_string()));
    }

    #[test]
    fn test_collect_variables_empty_result() {
        let manager = create_test_manager();
        let args = MonitorArgs {
            vars: vec!["NONEXISTENT".to_string()],
            log: None,
            changes_only: false,
            scope: None,
            format: OutputFormat::Live,
            interval: 2,
            show_initial: false,
            export_report: None,
            reveal: false,
        };

        let result = collect_variables(&manager, &args);

        // Should return empty map
        assert!(result.is_empty());
    }

    #[test]
    fn test_detect_changes_no_changes() {
        let state = MonitorState {
            initial: HashMap::from([
                ("VAR1".to_string(), "value1".to_string()),
                ("VAR2".to_string(), "value2".to_string()),
            ]),
            current: HashMap::from([
                ("VAR1".to_string(), "value1".to_string()),
                ("VAR2".to_string(), "value2".to_string()),
            ]),
            changes: vec![],
            start_time: Local::now(),
        };

        let changes = detect_changes(&state);

        // No changes should be detected
        assert!(changes.is_empty());
    }

    #[test]
    fn output_is_redacted_and_invalid_names_are_sanitized_by_default() {
        let change = ChangeRecord {
            timestamp: Local::now(),
            variable: "user:TOKEN=embedded-secret".to_string(),
            change_type: "modified".to_string(),
            old_value: Some("old-secret".to_string()),
            new_value: Some("new-secret".to_string()),
        };

        let output = output_change(&change, false);
        assert_eq!(output.variable, "user:TOKEN=<redacted>");
        assert_eq!(output.old_value.as_deref(), Some("[REDACTED]"));
        assert_eq!(output.new_value.as_deref(), Some("[REDACTED]"));
    }

    #[test]
    fn test_detect_changes_modifications() {
        let state = MonitorState {
            initial: HashMap::from([
                ("VAR1".to_string(), "old_value".to_string()),
                ("VAR2".to_string(), "value2".to_string()),
            ]),
            current: HashMap::from([
                ("VAR1".to_string(), "new_value".to_string()),
                ("VAR2".to_string(), "value2".to_string()),
            ]),
            changes: vec![],
            start_time: Local::now(),
        };

        let changes = detect_changes(&state);

        // Should detect one modification
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].variable, "VAR1");
        assert_eq!(changes[0].change_type, "modified");
        assert_eq!(changes[0].old_value, Some("old_value".to_string()));
        assert_eq!(changes[0].new_value, Some("new_value".to_string()));
    }

    #[test]
    fn test_detect_changes_additions() {
        let state = MonitorState {
            initial: HashMap::from([("VAR1".to_string(), "value1".to_string())]),
            current: HashMap::from([
                ("VAR1".to_string(), "value1".to_string()),
                ("VAR2".to_string(), "new_var_value".to_string()),
                ("VAR3".to_string(), "another_new".to_string()),
            ]),
            changes: vec![],
            start_time: Local::now(),
        };

        let changes = detect_changes(&state);

        // Should detect two additions
        assert_eq!(changes.len(), 2);

        let added_vars: Vec<&str> = changes
            .iter()
            .filter(|c| c.change_type == "added")
            .map(|c| c.variable.as_str())
            .collect();

        assert!(added_vars.contains(&"VAR2"));
        assert!(added_vars.contains(&"VAR3"));

        for change in changes {
            assert_eq!(change.change_type, "added");
            assert!(change.old_value.is_none());
            assert!(change.new_value.is_some());
        }
    }

    #[test]
    fn test_detect_changes_deletions() {
        let state = MonitorState {
            initial: HashMap::from([
                ("VAR1".to_string(), "value1".to_string()),
                ("VAR2".to_string(), "value2".to_string()),
                ("VAR3".to_string(), "value3".to_string()),
            ]),
            current: HashMap::from([("VAR2".to_string(), "value2".to_string())]),
            changes: vec![],
            start_time: Local::now(),
        };

        let changes = detect_changes(&state);

        // Should detect two deletions
        assert_eq!(changes.len(), 2);

        let deleted_vars: Vec<&str> = changes
            .iter()
            .filter(|c| c.change_type == "deleted")
            .map(|c| c.variable.as_str())
            .collect();

        assert!(deleted_vars.contains(&"VAR1"));
        assert!(deleted_vars.contains(&"VAR3"));

        for change in changes {
            if change.change_type == "deleted" {
                assert!(change.old_value.is_some());
                assert!(change.new_value.is_none());
            }
        }
    }

    #[test]
    fn test_detect_changes_mixed() {
        let state = MonitorState {
            initial: HashMap::from([
                ("MODIFIED".to_string(), "old".to_string()),
                ("DELETED".to_string(), "will_be_removed".to_string()),
                ("UNCHANGED".to_string(), "same".to_string()),
            ]),
            current: HashMap::from([
                ("MODIFIED".to_string(), "new".to_string()),
                ("UNCHANGED".to_string(), "same".to_string()),
                ("ADDED".to_string(), "brand_new".to_string()),
            ]),
            changes: vec![],
            start_time: Local::now(),
        };

        let changes = detect_changes(&state);

        // Should detect 3 changes: 1 modified, 1 added, 1 deleted
        assert_eq!(changes.len(), 3);

        let change_map: HashMap<String, &ChangeRecord> = changes.iter().map(|c| (c.variable.clone(), c)).collect();

        // Check modified
        let modified = change_map.get("MODIFIED").unwrap();
        assert_eq!(modified.change_type, "modified");
        assert_eq!(modified.old_value, Some("old".to_string()));
        assert_eq!(modified.new_value, Some("new".to_string()));

        // Check added
        let added = change_map.get("ADDED").unwrap();
        assert_eq!(added.change_type, "added");
        assert!(added.old_value.is_none());
        assert_eq!(added.new_value, Some("brand_new".to_string()));

        // Check deleted
        let deleted = change_map.get("DELETED").unwrap();
        assert_eq!(deleted.change_type, "deleted");
        assert_eq!(deleted.old_value, Some("will_be_removed".to_string()));
        assert!(deleted.new_value.is_none());
    }

    #[test]
    fn test_detect_changes_empty_states() {
        // Test with empty initial state
        let state = MonitorState {
            initial: HashMap::new(),
            current: HashMap::from([
                ("NEW1".to_string(), "value1".to_string()),
                ("NEW2".to_string(), "value2".to_string()),
            ]),
            changes: vec![],
            start_time: Local::now(),
        };

        let changes = detect_changes(&state);
        assert_eq!(changes.len(), 2);
        assert!(changes.iter().all(|c| c.change_type == "added"));

        // Test with empty current state
        let state2 = MonitorState {
            initial: HashMap::from([
                ("OLD1".to_string(), "value1".to_string()),
                ("OLD2".to_string(), "value2".to_string()),
            ]),
            current: HashMap::new(),
            changes: vec![],
            start_time: Local::now(),
        };

        let changes2 = detect_changes(&state2);
        assert_eq!(changes2.len(), 2);
        assert!(changes2.iter().all(|c| c.change_type == "deleted"));
    }

    #[test]
    fn test_detect_changes_special_characters() {
        let state = MonitorState {
            initial: HashMap::from([
                ("PATH/WITH/SLASH".to_string(), "value1".to_string()),
                ("VAR=WITH=EQUALS".to_string(), "value2".to_string()),
                ("UNICODE_变量".to_string(), "旧值".to_string()),
            ]),
            current: HashMap::from([
                ("PATH/WITH/SLASH".to_string(), "value1_modified".to_string()),
                ("VAR=WITH=EQUALS".to_string(), "value2".to_string()),
                ("UNICODE_变量".to_string(), "新值".to_string()),
            ]),
            changes: vec![],
            start_time: Local::now(),
        };

        let changes = detect_changes(&state);

        assert_eq!(changes.len(), 2);

        let unicode_change = changes.iter().find(|c| c.variable == "UNICODE_变量").unwrap();
        assert_eq!(unicode_change.old_value, Some("旧值".to_string()));
        assert_eq!(unicode_change.new_value, Some("新值".to_string()));
    }

    #[test]
    fn test_detect_changes_case_sensitive() {
        let state = MonitorState {
            initial: HashMap::from([
                ("lowercase".to_string(), "value1".to_string()),
                ("UPPERCASE".to_string(), "value2".to_string()),
            ]),
            current: HashMap::from([
                ("lowercase".to_string(), "value1".to_string()),
                ("UPPERCASE".to_string(), "value2".to_string()),
                ("Lowercase".to_string(), "different".to_string()), // Different case
            ]),
            changes: vec![],
            start_time: Local::now(),
        };

        let changes = detect_changes(&state);

        // Should detect the new variable with different case as an addition
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].variable, "Lowercase");
        assert_eq!(changes[0].change_type, "added");
    }

    #[test]
    fn test_detect_changes_empty_values() {
        let state = MonitorState {
            initial: HashMap::from([
                ("EMPTY_TO_VALUE".to_string(), String::new()),
                ("VALUE_TO_EMPTY".to_string(), "something".to_string()),
                ("EMPTY_TO_EMPTY".to_string(), String::new()),
            ]),
            current: HashMap::from([
                ("EMPTY_TO_VALUE".to_string(), "now_has_value".to_string()),
                ("VALUE_TO_EMPTY".to_string(), String::new()),
                ("EMPTY_TO_EMPTY".to_string(), String::new()),
            ]),
            changes: vec![],
            start_time: Local::now(),
        };

        let changes = detect_changes(&state);

        // Should detect 2 changes (empty to empty is not a change)
        assert_eq!(changes.len(), 2);

        let empty_to_value = changes.iter().find(|c| c.variable == "EMPTY_TO_VALUE").unwrap();
        assert_eq!(empty_to_value.old_value, Some(String::new()));
        assert_eq!(empty_to_value.new_value, Some("now_has_value".to_string()));

        let value_to_empty = changes.iter().find(|c| c.variable == "VALUE_TO_EMPTY").unwrap();
        assert_eq!(value_to_empty.old_value, Some("something".to_string()));
        assert_eq!(value_to_empty.new_value, Some(String::new()));
    }

    #[test]
    fn test_detect_changes_timestamp_consistency() {
        let state = MonitorState {
            initial: HashMap::from([("VAR1".to_string(), "old".to_string())]),
            current: HashMap::from([
                ("VAR1".to_string(), "new".to_string()),
                ("VAR2".to_string(), "added".to_string()),
            ]),
            changes: vec![],
            start_time: Local::now(),
        };

        let before = Local::now();
        let changes = detect_changes(&state);
        let after = Local::now();

        // All changes should have the same timestamp
        assert!(changes.len() >= 2);
        let first_timestamp = changes[0].timestamp;
        assert!(changes.iter().all(|c| c.timestamp == first_timestamp));

        // Timestamp should be within the test execution window
        assert!(first_timestamp >= before && first_timestamp <= after);
    }
}
