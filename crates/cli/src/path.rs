use crate::PathAction;
use color_eyre::Result;
use color_eyre::eyre::eyre;
use envx_core::{EnvScope, EnvVarManager, PathManager};

/// Executes a PATH inspection or exact-scope mutation.
///
/// # Errors
///
/// Returns an error when loading, expansion, validation, backup, or persistence fails.
#[allow(clippy::too_many_lines)]
pub fn handle_path_command(action: Option<PathAction>, scope: EnvScope, name: &str, apply: bool) -> Result<()> {
    let mut manager = EnvVarManager::new();
    manager.load_all()?;
    let variable = manager.get(scope, name).cloned().ok_or_else(|| {
        eyre!(
            "Environment variable '{}' was not found in {scope} scope",
            crate::list::safe_name(name)
        )
    })?;
    let mut paths = PathManager::new(&variable.value);

    match action {
        None => list(&paths, false, false)?,
        Some(PathAction::List { numbered, check }) => list(&paths, numbered, check)?,
        Some(PathAction::Check { verbose }) => check(&paths, verbose)?,
        Some(PathAction::Add {
            directory,
            first,
            create,
        }) => {
            let resolved = PathManager::resolved_path(&directory)?;
            if !resolved.exists() {
                if create && apply {
                    std::fs::create_dir_all(&resolved)?;
                } else if create {
                    println!("Would create '{}'.", resolved.display());
                } else {
                    return Err(eyre!("Directory '{}' does not exist", resolved.display()));
                }
            }
            if paths.contains(&directory)? {
                return Err(eyre!(
                    "Directory is already present in {scope} {}",
                    crate::list::safe_name(name)
                ));
            }
            if first {
                paths.add_first(directory.clone());
            } else {
                paths.add_last(directory.clone());
            }
            persist(&mut manager, scope, name, &paths, apply)?;
            println!("{} add '{}'.", action_word(apply), directory);
        }
        Some(PathAction::Remove { directory, all }) => {
            let removed = if all {
                paths.remove_all(&directory)?
            } else {
                paths.remove_first(&directory)?
            };
            if removed == 0 {
                return Err(eyre!(
                    "Directory is not present in {scope} {}",
                    crate::list::safe_name(name)
                ));
            }
            persist(&mut manager, scope, name, &paths, apply)?;
            println!(
                "{} remove {removed} occurrence(s) of '{}'.",
                action_word(apply),
                directory
            );
        }
        Some(PathAction::Clean { dedupe }) => {
            let invalid = paths.get_invalid()?;
            let duplicates = if dedupe { paths.get_duplicates()? } else { Vec::new() };
            for path in &invalid {
                println!("Missing: {path}");
            }
            for path in &duplicates {
                println!("Duplicate: {path}");
            }
            if apply {
                paths.remove_invalid()?;
                if dedupe {
                    paths.deduplicate(true)?;
                }
            }
            persist(&mut manager, scope, name, &paths, apply)?;
            if !apply {
                println!("Dry run. Add --apply to clean.");
            }
        }
        Some(PathAction::Dedupe { keep_first }) => {
            let duplicates = paths.get_duplicates()?;
            for path in &duplicates {
                println!("Duplicate: {path}");
            }
            if duplicates.is_empty() {
                println!("No duplicate entries.");
                return Ok(());
            }
            if apply {
                paths.deduplicate(keep_first)?;
            }
            persist(&mut manager, scope, name, &paths, apply)?;
            if !apply {
                println!("Dry run. Add --apply to deduplicate.");
            }
        }
        Some(PathAction::Move { from, to }) => {
            let from_index = match from.parse::<usize>() {
                Ok(index) => index,
                Err(_) => paths
                    .find_index(&from)?
                    .ok_or_else(|| eyre!("Path '{from}' was not found"))?,
            };
            let to_index = match to.as_str() {
                "first" => 0,
                "last" => paths.len().checked_sub(1).ok_or_else(|| eyre!("PATH is empty"))?,
                _ => to.parse::<usize>().map_err(|_| eyre!("Invalid destination '{to}'"))?,
            };
            paths.move_entry(from_index, to_index)?;
            persist(&mut manager, scope, name, &paths, apply)?;
            println!("{} move entry {from_index} to {to_index}.", action_word(apply));
        }
    }
    for id in manager.take_last_backup_ids() {
        println!("Backup: {id}");
    }
    Ok(())
}

fn persist(manager: &mut EnvVarManager, scope: EnvScope, name: &str, paths: &PathManager, apply: bool) -> Result<()> {
    if apply {
        let kind = manager.get(scope, name).map(|variable| variable.kind);
        manager.set(scope, name, &paths.to_string(), kind)?;
    }
    Ok(())
}

fn list(paths: &PathManager, numbered: bool, verify: bool) -> Result<()> {
    for (index, path) in paths.entries().iter().enumerate() {
        let prefix = if numbered {
            format!("[{index:3}] ")
        } else {
            String::new()
        };
        if verify {
            let resolved = PathManager::resolved_path(path)?;
            if !resolved.exists() {
                println!("{prefix}{path} [NOT FOUND: {}]", resolved.display());
                continue;
            }
            if !resolved.is_dir() {
                println!("{prefix}{path} [NOT A DIRECTORY: {}]", resolved.display());
                continue;
            }
        }
        println!("{prefix}{path}");
    }
    Ok(())
}

fn check(paths: &PathManager, verbose: bool) -> Result<()> {
    if verbose {
        list(paths, true, true)?;
    }
    println!("PATH entries: {}", paths.len());
    println!("Duplicates: {}", paths.get_duplicates()?.len());
    println!("Missing: {}", paths.get_invalid()?.len());
    Ok(())
}

const fn action_word(apply: bool) -> &'static str {
    if apply { "Applied" } else { "Would" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_word_distinguishes_dry_run() {
        assert_eq!(action_word(false), "Would");
        assert_eq!(action_word(true), "Applied");
    }
}
