use crate::monitor::handle_monitor;
use crate::replace::{FindReplaceArgs, ReplaceArgs};
use crate::wizard::{list_templates as list_templates_func, run_wizard};
use crate::{
    CleanupArgs, DepsArgs, DocsArgs, MonitorArgs, ProfileArgs, ProjectArgs, RenameArgs, SnapshotArgs, WatchArgs,
    handle_cleanup, handle_deps, handle_docs, handle_find_replace, handle_list_command, handle_path_command,
    handle_profile, handle_project, handle_rename, handle_replace, handle_snapshot, handle_watch,
};
use clap::{Parser, Subcommand, ValueEnum};
use color_eyre::Result;
use color_eyre::eyre::eyre;
use envx_core::{Analyzer, BackupManager, EnvScope, EnvVarManager, ExportFormat, Exporter, ImportFormat, Importer};
use serde::Serialize;
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ScopeArg {
    Process,
    User,
    System,
}

impl From<ScopeArg> for EnvScope {
    fn from(scope: ScopeArg) -> Self {
        match scope {
            ScopeArg::Process => Self::Process,
            ScopeArg::User => Self::User,
            ScopeArg::System => Self::System,
        }
    }
}

#[derive(Parser)]
#[command(name = "envx", about = "Scope-aware environment variable manager", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    Init {
        #[arg(short, long)]
        template: Option<String>,
        #[arg(short, long, default_value = "true")]
        wizard: bool,
        #[arg(long)]
        list_templates: bool,
    },
    List {
        #[arg(short, long, value_enum)]
        scope: Option<ScopeArg>,
        #[arg(short = 'q', long)]
        query: Option<String>,
        #[arg(short, long, default_value = "table")]
        format: String,
        #[arg(long, default_value = "name")]
        sort: String,
        #[arg(long)]
        names_only: bool,
        #[arg(short, long)]
        limit: Option<usize>,
        #[arg(long)]
        stats: bool,
        #[arg(long)]
        reveal: bool,
    },
    Get {
        pattern: String,
        #[arg(short, long, value_enum)]
        scope: Option<ScopeArg>,
        #[arg(short, long, default_value = "simple")]
        format: String,
        #[arg(long)]
        reveal: bool,
    },
    Set {
        #[arg(long, value_enum)]
        scope: ScopeArg,
        name: String,
        value: String,
    },
    Delete {
        #[arg(long, value_enum)]
        scope: ScopeArg,
        pattern: String,
        #[arg(short, long)]
        force: bool,
    },
    Analyze {
        #[arg(short, long, default_value = "all")]
        analysis_type: String,
        #[arg(long, value_enum)]
        scope: Option<ScopeArg>,
    },
    #[command(visible_alias = "ui")]
    Tui,
    Path {
        #[command(subcommand)]
        action: Option<PathAction>,
        #[arg(long, value_enum)]
        scope: ScopeArg,
        #[arg(short = 'v', long, default_value = "PATH")]
        var: String,
        #[arg(long, global = true)]
        apply: bool,
    },
    Export {
        file: String,
        #[arg(short = 'v', long)]
        vars: Vec<String>,
        #[arg(short, long)]
        format: Option<String>,
        #[arg(short, long, value_enum)]
        scope: ScopeArg,
        #[arg(short, long)]
        metadata: bool,
        #[arg(long)]
        force: bool,
    },
    Import {
        file: String,
        #[arg(long, value_enum)]
        scope: ScopeArg,
        #[arg(short = 'v', long)]
        vars: Vec<String>,
        #[arg(short, long)]
        format: Option<String>,
        #[arg(long)]
        prefix: Option<String>,
        #[arg(long)]
        overwrite: bool,
        #[arg(short = 'n', long)]
        dry_run: bool,
    },
    Snapshot(SnapshotArgs),
    Profile(ProfileArgs),
    Project(ProjectArgs),
    Rename(RenameArgs),
    Replace(ReplaceArgs),
    FindReplace(FindReplaceArgs),
    Watch(WatchArgs),
    Monitor(MonitorArgs),
    Docs(DocsArgs),
    Deps(DepsArgs),
    Cleanup(CleanupArgs),
    Doctor {
        #[arg(long, value_enum)]
        scope: ScopeArg,
    },
    Repair {
        #[command(subcommand)]
        command: RepairCommand,
    },
    Backup {
        #[command(subcommand)]
        command: BackupCommand,
    },
}

#[derive(Subcommand)]
pub enum PathAction {
    Add {
        directory: String,
        #[arg(short, long)]
        first: bool,
        #[arg(short, long)]
        create: bool,
    },
    Remove {
        directory: String,
        #[arg(short, long)]
        all: bool,
    },
    Clean {
        #[arg(short, long)]
        dedupe: bool,
    },
    Dedupe {
        #[arg(short, long)]
        keep_first: bool,
    },
    Check {
        #[arg(short, long)]
        verbose: bool,
    },
    List {
        #[arg(short, long)]
        numbered: bool,
        #[arg(short, long)]
        check: bool,
    },
    Move {
        from: String,
        to: String,
    },
}

#[derive(Subcommand)]
pub enum RepairCommand {
    InvalidNames {
        #[arg(long, value_enum)]
        scope: ScopeArg,
        #[arg(long)]
        apply: bool,
    },
}

#[derive(Subcommand)]
pub enum BackupCommand {
    List,
    Restore {
        id: String,
        #[arg(long)]
        apply: bool,
    },
}

#[derive(Serialize)]
struct GetOutput<'a> {
    scope: EnvScope,
    name: String,
    value: &'a str,
}

/// Executes one parsed CLI command.
///
/// # Errors
///
/// Returns an error when command validation, environment access, backup, or file I/O fails.
#[allow(clippy::too_many_lines)]
pub fn execute(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::List {
            scope,
            query,
            format,
            sort,
            names_only,
            limit,
            stats,
            reveal,
        } => {
            handle_list_command(
                scope.map(Into::into),
                query.as_deref(),
                &format,
                &sort,
                names_only,
                limit,
                stats,
                reveal,
            )?;
        }
        Commands::Get {
            pattern,
            scope,
            format,
            reveal,
        } => {
            handle_get_command(&pattern, scope.map(Into::into), &format, reveal)?;
        }
        Commands::Set { scope, name, value } => handle_set_command(scope.into(), &name, &value)?,
        Commands::Delete { scope, pattern, force } => handle_delete_command(scope.into(), &pattern, force)?,
        Commands::Analyze { analysis_type, scope } => handle_analyze_command(&analysis_type, scope.map(Into::into))?,
        Commands::Tui => envx_tui::run()?,
        Commands::Path {
            action,
            scope,
            var,
            apply,
        } => handle_path_command(action, scope.into(), &var, apply)?,
        Commands::Export {
            file,
            vars,
            format,
            scope,
            metadata,
            force,
        } => {
            handle_export(&file, &vars, format.as_deref(), scope.into(), metadata, force)?;
        }
        Commands::Import {
            file,
            scope,
            vars,
            format,
            prefix,
            overwrite,
            dry_run,
        } => {
            handle_import(
                &file,
                scope.into(),
                &vars,
                format.as_deref(),
                prefix.as_ref(),
                overwrite,
                dry_run,
            )?;
        }
        Commands::Snapshot(args) => handle_snapshot(args)?,
        Commands::Profile(args) => handle_profile(args)?,
        Commands::Project(args) => handle_project(args)?,
        Commands::Rename(args) => handle_rename(&args)?,
        Commands::Replace(args) => handle_replace(&args)?,
        Commands::FindReplace(args) => handle_find_replace(&args)?,
        Commands::Watch(args) => handle_watch(&args)?,
        Commands::Monitor(args) => handle_monitor(args)?,
        Commands::Docs(args) => handle_docs(args)?,
        Commands::Deps(args) => handle_deps(&args)?,
        Commands::Cleanup(args) => handle_cleanup(&args)?,
        Commands::Doctor { scope } => handle_doctor(scope.into())?,
        Commands::Repair { command } => handle_repair(&command)?,
        Commands::Backup { command } => handle_backup(command)?,
        Commands::Init {
            template,
            wizard: _,
            list_templates,
        } => {
            if list_templates {
                list_templates_func()?;
            } else {
                run_wizard(template)?;
            }
        }
    }
    Ok(())
}

fn handle_get_command(pattern: &str, scope: Option<EnvScope>, format: &str, reveal: bool) -> Result<()> {
    let mut manager = EnvVarManager::new();
    manager.load_all()?;
    let variables = manager.get_pattern(pattern, scope);
    if variables.is_empty() {
        return Err(eyre!("No variables found matching '{pattern}'"));
    }
    let output: Vec<_> = variables
        .iter()
        .map(|variable| GetOutput {
            scope: variable.scope,
            name: crate::list::safe_name(&variable.name),
            value: crate::list::displayed_value(variable, reveal),
        })
        .collect();
    if format == "json" {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        for variable in output {
            println!("{} {} = {}", variable.scope, variable.name, variable.value);
        }
    }
    Ok(())
}

fn handle_set_command(scope: EnvScope, name: &str, value: &str) -> Result<()> {
    let mut manager = EnvVarManager::new();
    manager.load_all()?;
    manager.set(scope, name, value, None)?;
    println!("Set {scope} variable '{name}'.");
    print_backups(&mut manager);
    Ok(())
}

fn handle_delete_command(scope: EnvScope, pattern: &str, force: bool) -> Result<()> {
    let mut manager = EnvVarManager::new();
    manager.load_all()?;
    let names: Vec<String> = manager
        .get_pattern(pattern, Some(scope))
        .into_iter()
        .map(|variable| variable.name.clone())
        .collect();
    if names.is_empty() {
        return Err(eyre!("No {scope} variables match '{pattern}'"));
    }
    if !force {
        println!("About to delete {} {scope} variable(s):", names.len());
        for name in &names {
            println!("  {}", crate::list::safe_name(name));
        }
        print!("Continue? [y/N]: ");
        std::io::stdout().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !answer.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled.");
            return Ok(());
        }
    }
    manager.delete_many(scope, &names)?;
    println!("Deleted {} {scope} variable(s).", names.len());
    print_backups(&mut manager);
    Ok(())
}

fn handle_analyze_command(analysis_type: &str, scope: Option<EnvScope>) -> Result<()> {
    let mut manager = EnvVarManager::new();
    manager.load_all()?;
    let analyzer = Analyzer::new(manager.list(scope).into_iter().cloned().collect());
    if matches!(analysis_type, "duplicates" | "all") {
        for (name, variables) in analyzer.find_duplicates() {
            println!("{}: {} instances", crate::list::safe_name(&name), variables.len());
        }
    }
    if matches!(analysis_type, "invalid" | "all") {
        for (name, _result) in analyzer.validate_all().into_iter().filter(|(_, result)| !result.valid) {
            println!("Invalid: {}", crate::list::safe_name(&name));
        }
    }
    Ok(())
}

fn handle_export(
    file: &str,
    patterns: &[String],
    format: Option<&str>,
    scope: EnvScope,
    metadata: bool,
    force: bool,
) -> Result<()> {
    if Path::new(file).exists() && !force {
        return Err(eyre!("File '{file}' already exists. Use --force to overwrite"));
    }
    let mut manager = EnvVarManager::new();
    manager.load_all()?;
    let variables: Vec<_> = if patterns.is_empty() {
        manager.list(Some(scope)).into_iter().cloned().collect()
    } else {
        patterns
            .iter()
            .flat_map(|pattern| manager.get_pattern(pattern, Some(scope)))
            .cloned()
            .collect()
    };
    if variables.iter().any(|variable| variable.name.contains('=')) {
        return Err(eyre!(
            "Export refused because the selection contains invalid names. Run repair invalid-names first"
        ));
    }
    let format = match format {
        Some("env") => ExportFormat::DotEnv,
        Some("json") => ExportFormat::Json,
        Some("yaml" | "yml") => ExportFormat::Yaml,
        Some("txt" | "text") => ExportFormat::Text,
        Some("ps1" | "powershell") => ExportFormat::PowerShell,
        Some("sh" | "bash") => ExportFormat::Shell,
        Some(invalid) => return Err(eyre!("Unsupported export format '{invalid}'")),
        None => ExportFormat::from_extension(file)?,
    };
    let exporter = Exporter::new(variables, metadata);
    exporter.export_to_file(file, format)?;
    println!("Exported {} variables to '{file}'.", exporter.count());
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn handle_import(
    file: &str,
    scope: EnvScope,
    patterns: &[String],
    format: Option<&str>,
    prefix: Option<&String>,
    overwrite: bool,
    dry_run: bool,
) -> Result<()> {
    if !Path::new(file).exists() {
        return Err(eyre!("File '{file}' does not exist"));
    }
    let format = match format {
        Some("env") => ImportFormat::DotEnv,
        Some("json") => ImportFormat::Json,
        Some("yaml" | "yml") => ImportFormat::Yaml,
        Some("txt" | "text") => ImportFormat::Text,
        Some(invalid) => return Err(eyre!("Unsupported import format '{invalid}'")),
        None => ImportFormat::from_extension(file)?,
    };
    let mut importer = Importer::new();
    importer.import_from_file(file, format)?;
    if !patterns.is_empty() {
        importer.filter_by_patterns(patterns);
    }
    if let Some(prefix) = prefix {
        importer.add_prefix(prefix);
    }
    let variables = importer.get_variables();
    let mut manager = EnvVarManager::new();
    manager.load_all()?;
    let conflicts: Vec<_> = variables
        .iter()
        .filter(|(name, _)| manager.get(scope, name).is_some())
        .map(|(name, _)| name.clone())
        .collect();
    if !conflicts.is_empty() && !overwrite {
        return Err(eyre!(
            "{} variables already exist in {scope} scope. Use --overwrite",
            conflicts.len()
        ));
    }
    if dry_run {
        for (name, _) in &variables {
            println!("Would import {scope} {} = [REDACTED]", crate::list::safe_name(name));
        }
        return Ok(());
    }
    for (name, value) in variables {
        manager.set(scope, &name, &value, None)?;
    }
    println!("Imported variables into {scope} scope.");
    print_backups(&mut manager);
    Ok(())
}

fn handle_doctor(scope: EnvScope) -> Result<()> {
    let mut manager = EnvVarManager::new();
    manager.load_all()?;
    let invalid: Vec<_> = manager
        .list(Some(scope))
        .into_iter()
        .filter(|variable| variable.name.contains('='))
        .collect();
    println!("Invalid names: {}", invalid.len());
    for variable in invalid {
        println!("  {}", crate::list::safe_name(&variable.name));
    }
    if let Some(path) = manager.get(scope, "PATH") {
        let paths = envx_core::PathManager::new(&path.value);
        println!("PATH entries: {}", paths.len());
        println!("PATH duplicates: {}", paths.get_duplicates()?.len());
        println!("PATH missing: {}", paths.get_invalid()?.len());
    }
    Ok(())
}

fn handle_repair(command: &RepairCommand) -> Result<()> {
    match command {
        RepairCommand::InvalidNames { scope, apply } => {
            let scope = (*scope).into();
            let mut manager = EnvVarManager::new();
            manager.load_all()?;
            let names: Vec<_> = manager
                .list(Some(scope))
                .into_iter()
                .filter(|variable| variable.name.contains('='))
                .map(|variable| variable.name.clone())
                .collect();
            println!("Invalid names found: {}", names.len());
            for name in &names {
                println!("  {}", crate::list::safe_name(name));
            }
            if *apply && !names.is_empty() {
                manager.delete_many(scope, &names)?;
                println!("Removed {} invalid {scope} entries.", names.len());
                print_backups(&mut manager);
            } else if !*apply {
                println!("Dry run. Add --apply to repair.");
            }
        }
    }
    Ok(())
}

fn handle_backup(command: BackupCommand) -> Result<()> {
    match command {
        BackupCommand::List => {
            for backup in BackupManager::new()?.list()? {
                println!(
                    "{} {} {} entries",
                    backup.id,
                    backup.created_at.format("%Y-%m-%d %H:%M:%S UTC"),
                    backup.entry_count
                );
            }
        }
        BackupCommand::Restore { id, apply } => {
            if !apply {
                return Err(eyre!("Restore is a mutation. Add --apply"));
            }
            let mut manager = EnvVarManager::new();
            manager.load_all()?;
            manager.restore_backup(&id)?;
            println!("Restored backup '{id}'.");
            print_backups(&mut manager);
        }
    }
    Ok(())
}

fn print_backups(manager: &mut EnvVarManager) {
    for id in manager.take_last_backup_ids() {
        println!("Backup: {id}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutation_commands_require_scope() {
        assert!(Cli::try_parse_from(["envx", "set", "NAME", "value"]).is_err());
        assert!(Cli::try_parse_from(["envx", "delete", "NAME"]).is_err());
        assert!(Cli::try_parse_from(["envx", "import", "vars.env"]).is_err());
        assert!(Cli::try_parse_from(["envx", "export", "vars.env"]).is_err());
        assert!(Cli::try_parse_from(["envx", "set", "--scope", "user", "NAME", "value"]).is_ok());
    }

    #[test]
    fn read_commands_allow_all_scopes() {
        assert!(Cli::try_parse_from(["envx", "list"]).is_ok());
        assert!(Cli::try_parse_from(["envx", "get", "PATH"]).is_ok());
    }
}
