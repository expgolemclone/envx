use crate::EnvxError;
use crate::backup::{BackupEntry, BackupManager};
use chrono::{DateTime, Utc};
use color_eyre::Result;
use color_eyre::eyre::{WrapErr, eyre};
use indexmap::IndexMap;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnvScope {
    Process,
    User,
    System,
}

impl EnvScope {
    #[must_use]
    pub const fn is_persistent(self) -> bool {
        matches!(self, Self::User | Self::System)
    }
}

impl fmt::Display for EnvScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Process => "process",
            Self::User => "user",
            Self::System => "system",
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EnvValueKind {
    #[default]
    String,
    ExpandString,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EnvKey {
    pub scope: EnvScope,
    canonical_name: String,
}

impl EnvKey {
    #[must_use]
    pub fn new(scope: EnvScope, name: &str) -> Self {
        Self {
            scope,
            canonical_name: canonical_name(name),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
    pub scope: EnvScope,
    pub kind: EnvValueKind,
    pub modified: DateTime<Utc>,
    pub original_value: Option<String>,
}

pub trait EnvBackend: Send + Sync {
    /// Loads every variable in one scope.
    ///
    /// # Errors
    ///
    /// Returns an error when the backing store cannot be read or decoded.
    fn load(&self, scope: EnvScope) -> Result<Vec<EnvVar>>;

    /// Verifies that a scope can be written before a mutation starts.
    ///
    /// # Errors
    ///
    /// Returns an error when the scope is unsupported or write access is unavailable.
    fn preflight_write(&self, scope: EnvScope) -> Result<()>;

    /// Writes one variable to an exact scope.
    ///
    /// # Errors
    ///
    /// Returns an error when the backing store rejects the write.
    fn set(&self, scope: EnvScope, name: &str, value: &str, kind: EnvValueKind) -> Result<()>;

    /// Deletes one variable from an exact scope.
    ///
    /// # Errors
    ///
    /// Returns an error when the backing store rejects the deletion.
    fn delete(&self, scope: EnvScope, name: &str) -> Result<()>;
}

#[derive(Default)]
pub struct SystemEnvBackend;

impl SystemEnvBackend {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl EnvBackend for SystemEnvBackend {
    fn load(&self, scope: EnvScope) -> Result<Vec<EnvVar>> {
        if scope == EnvScope::Process {
            return std::env::vars_os()
                .map(|(name, value)| {
                    let name = name
                        .into_string()
                        .map_err(|_| eyre!("Environment variable name is not valid Unicode"))?;
                    let value = value
                        .into_string()
                        .map_err(|_| eyre!("Environment variable '{}' is not valid Unicode", safe_name(&name)))?;
                    Ok(EnvVar {
                        name,
                        value,
                        scope,
                        kind: EnvValueKind::String,
                        modified: Utc::now(),
                        original_value: None,
                    })
                })
                .collect();
        }

        #[cfg(windows)]
        {
            load_registry_scope(scope)
        }

        #[cfg(not(windows))]
        {
            let _ = scope;
            Ok(Vec::new())
        }
    }

    fn preflight_write(&self, scope: EnvScope) -> Result<()> {
        if scope == EnvScope::Process {
            return Ok(());
        }

        #[cfg(windows)]
        {
            open_registry_scope(scope, winreg::enums::KEY_SET_VALUE).map(|_| ())
        }

        #[cfg(not(windows))]
        {
            Err(eyre!(
                "Persistent environment scope '{scope}' is unsupported on this platform"
            ))
        }
    }

    fn set(&self, scope: EnvScope, name: &str, value: &str, kind: EnvValueKind) -> Result<()> {
        if scope == EnvScope::Process {
            unsafe { std::env::set_var(name, value) };
            return Ok(());
        }

        #[cfg(windows)]
        {
            set_registry_value(scope, name, value, kind)?;
            broadcast_environment_change();
            Ok(())
        }

        #[cfg(not(windows))]
        {
            let _ = (name, value, kind);
            Err(eyre!(
                "Persistent environment scope '{scope}' is unsupported on this platform"
            ))
        }
    }

    fn delete(&self, scope: EnvScope, name: &str) -> Result<()> {
        if scope == EnvScope::Process {
            unsafe { std::env::remove_var(name) };
            return Ok(());
        }

        #[cfg(windows)]
        {
            let key = open_registry_scope(scope, winreg::enums::KEY_SET_VALUE)?;
            key.delete_value(name)
                .wrap_err_with(|| format!("Failed to delete '{}' from {scope} scope", safe_name(name)))?;
            broadcast_environment_change();
            Ok(())
        }

        #[cfg(not(windows))]
        {
            let _ = name;
            Err(eyre!(
                "Persistent environment scope '{scope}' is unsupported on this platform"
            ))
        }
    }
}

pub struct EnvVarManager {
    pub vars: IndexMap<EnvKey, EnvVar>,
    pub history: Vec<crate::history::HistoryEntry>,
    backend: Arc<dyn EnvBackend>,
    backups_enabled: bool,
    last_backup_ids: Vec<String>,
}

impl Default for EnvVarManager {
    fn default() -> Self {
        Self::new()
    }
}

impl EnvVarManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            vars: IndexMap::new(),
            history: Vec::new(),
            backend: Arc::new(SystemEnvBackend::new()),
            backups_enabled: true,
            last_backup_ids: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_backend(backend: Arc<dyn EnvBackend>) -> Self {
        Self {
            vars: IndexMap::new(),
            history: Vec::new(),
            backend,
            backups_enabled: false,
            last_backup_ids: Vec::new(),
        }
    }

    /// Reloads all scopes supported by the backend.
    ///
    /// # Errors
    ///
    /// Returns an error when any scope cannot be read or decoded.
    pub fn load_all(&mut self) -> Result<()> {
        self.vars.clear();
        for scope in [EnvScope::Process, EnvScope::System, EnvScope::User] {
            for variable in self.backend.load(scope)? {
                self.vars.insert(EnvKey::new(scope, &variable.name), variable);
            }
        }
        Ok(())
    }

    /// Reloads one scope without disturbing the others.
    ///
    /// # Errors
    ///
    /// Returns an error when the requested scope cannot be read or decoded.
    pub fn reload_scope(&mut self, scope: EnvScope) -> Result<()> {
        self.vars.retain(|key, _| key.scope != scope);
        for variable in self.backend.load(scope)? {
            self.vars.insert(EnvKey::new(scope, &variable.name), variable);
        }
        Ok(())
    }

    #[must_use]
    pub fn get(&self, scope: EnvScope, name: &str) -> Option<&EnvVar> {
        self.vars.get(&EnvKey::new(scope, name))
    }

    #[must_use]
    pub fn get_any(&self, name: &str) -> Vec<&EnvVar> {
        [EnvScope::Process, EnvScope::User, EnvScope::System]
            .into_iter()
            .filter_map(|scope| self.get(scope, name))
            .collect()
    }

    #[must_use]
    pub fn get_pattern(&self, pattern: &str, scope: Option<EnvScope>) -> Vec<&EnvVar> {
        if pattern.starts_with('/') && pattern.ends_with('/') && pattern.len() > 2 {
            self.get_regex(&pattern[1..pattern.len() - 1], scope)
        } else if pattern.contains('*') || pattern.contains('?') {
            self.get_wildcard(pattern, scope)
        } else if let Some(scope) = scope {
            self.get(scope, pattern).into_iter().collect()
        } else {
            self.get_any(pattern)
        }
    }

    #[must_use]
    pub fn get_wildcard(&self, pattern: &str, scope: Option<EnvScope>) -> Vec<&EnvVar> {
        self.get_regex(&wildcard_to_regex(pattern), scope)
    }

    #[must_use]
    pub fn get_regex(&self, pattern: &str, scope: Option<EnvScope>) -> Vec<&EnvVar> {
        let Ok(regex) = Regex::new(pattern) else {
            return Vec::new();
        };
        self.list(scope)
            .into_iter()
            .filter(|variable| regex.is_match(&variable.name))
            .collect()
    }

    #[must_use]
    pub fn get_prefix(&self, prefix: &str, scope: Option<EnvScope>) -> Vec<&EnvVar> {
        self.list(scope)
            .into_iter()
            .filter(|variable| variable.name.starts_with(prefix))
            .collect()
    }

    #[must_use]
    pub fn get_suffix(&self, suffix: &str, scope: Option<EnvScope>) -> Vec<&EnvVar> {
        self.list(scope)
            .into_iter()
            .filter(|variable| variable.name.ends_with(suffix))
            .collect()
    }

    #[must_use]
    pub fn get_containing(&self, substring: &str, scope: Option<EnvScope>) -> Vec<&EnvVar> {
        let lower = substring.to_lowercase();
        self.list(scope)
            .into_iter()
            .filter(|variable| variable.name.to_lowercase().contains(&lower))
            .collect()
    }

    #[must_use]
    pub fn list(&self, scope: Option<EnvScope>) -> Vec<&EnvVar> {
        self.vars
            .values()
            .filter(|variable| scope.is_none_or(|wanted| variable.scope == wanted))
            .collect()
    }

    #[must_use]
    pub fn filter_by_scope(&self, scope: EnvScope) -> Vec<&EnvVar> {
        self.list(Some(scope))
    }

    #[must_use]
    pub fn search(&self, query: &str, scope: Option<EnvScope>) -> Vec<&EnvVar> {
        let lower = query.to_lowercase();
        self.list(scope)
            .into_iter()
            .filter(|variable| {
                variable.name.to_lowercase().contains(&lower) || variable.value.to_lowercase().contains(&lower)
            })
            .collect()
    }

    /// Sets one variable in the requested scope.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid name, backup failure, denied access, or backend write failure.
    pub fn set(
        &mut self,
        scope: EnvScope,
        name: &str,
        value: &str,
        requested_kind: Option<EnvValueKind>,
    ) -> Result<()> {
        validate_name(name)?;
        self.backend.preflight_write(scope)?;
        let old = self.get(scope, name).cloned();
        let kind = requested_kind
            .or_else(|| old.as_ref().map(|variable| variable.kind))
            .unwrap_or_default();
        self.create_backup(&[(scope, name.to_string())])?;
        self.backend.set(scope, name, value, kind)?;
        self.history
            .push(crate::history::HistoryEntry::new(crate::history::HistoryAction::Set {
                scope,
                name: name.to_string(),
                old_value: old.as_ref().map(|variable| variable.value.clone()),
                new_value: value.to_string(),
            }));
        self.vars.insert(
            EnvKey::new(scope, name),
            EnvVar {
                name: name.to_string(),
                value: value.to_string(),
                scope,
                kind,
                modified: Utc::now(),
                original_value: old.map(|variable| variable.value),
            },
        );
        Ok(())
    }

    /// Deletes one variable from the requested scope.
    ///
    /// # Errors
    ///
    /// Returns an error when the variable is absent or the backup or backend deletion fails.
    pub fn delete(&mut self, scope: EnvScope, name: &str) -> Result<()> {
        self.delete_many(scope, &[name.to_string()])
    }

    /// Deletes several variables as one rollback-protected batch.
    ///
    /// # Errors
    ///
    /// Returns an error when a target is absent, backup fails, or deletion and rollback fail.
    pub fn delete_many(&mut self, scope: EnvScope, names: &[String]) -> Result<()> {
        if names.is_empty() {
            return Ok(());
        }
        self.backend.preflight_write(scope)?;
        let before: Vec<EnvVar> = names
            .iter()
            .map(|name| {
                self.get(scope, name)
                    .cloned()
                    .ok_or_else(|| EnvxError::VarNotFound(format!("{scope}:{}", safe_name(name))))
            })
            .collect::<std::result::Result<_, _>>()?;
        self.create_backup(&names.iter().map(|name| (scope, name.clone())).collect::<Vec<_>>())?;

        let mut deleted: Vec<EnvVar> = Vec::new();
        for variable in &before {
            if let Err(error) = self.backend.delete(scope, &variable.name) {
                for restored in deleted.iter().rev() {
                    self.backend
                        .set(restored.scope, &restored.name, &restored.value, restored.kind)
                        .wrap_err("Mutation failed and rollback also failed")?;
                }
                return Err(error);
            }
            deleted.push(variable.clone());
        }

        for variable in before {
            self.vars.swap_remove(&EnvKey::new(scope, &variable.name));
            self.history.push(crate::history::HistoryEntry::new(
                crate::history::HistoryAction::Delete {
                    scope,
                    name: variable.name,
                    old_value: variable.value,
                },
            ));
        }
        Ok(())
    }

    /// Renames variables matching a pattern within one scope.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid pattern or name, a collision, or a failed mutation.
    pub fn rename(&mut self, scope: EnvScope, pattern: &str, replacement: &str) -> Result<Vec<(String, String)>> {
        let matching = self.matching_owned(scope, pattern)?;
        if matching.is_empty() {
            return Err(EnvxError::VarNotFound(format!("{scope}:{pattern}")).into());
        }
        let (old_prefix, old_suffix) = split_wildcard_pattern(pattern)?;
        let (new_prefix, new_suffix) = split_wildcard_pattern(replacement)?;
        let mut renamed = Vec::new();
        for variable in matching {
            if variable.name.contains('=') {
                return Err(eyre!(
                    "Cannot rename variables with invalid names. Run repair invalid-names first"
                ));
            }
            let new_name = if pattern.contains('*') {
                let middle = &variable.name[old_prefix.len()..variable.name.len() - old_suffix.len()];
                format!("{new_prefix}{middle}{new_suffix}")
            } else {
                replacement.to_string()
            };
            validate_name(&new_name)?;
            if self.get(scope, &new_name).is_some() {
                return Err(eyre!(
                    "Cannot rename '{}' to '{}': target already exists",
                    safe_name(&variable.name),
                    safe_name(&new_name)
                ));
            }
            self.set(scope, &new_name, &variable.value, Some(variable.kind))?;
            self.delete(scope, &variable.name)?;
            renamed.push((variable.name, new_name));
        }
        Ok(renamed)
    }

    /// Replaces complete values for variables matching a pattern in one scope.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid pattern, missing match, or failed mutation.
    pub fn replace(
        &mut self,
        scope: EnvScope,
        pattern: &str,
        new_value: &str,
    ) -> Result<Vec<(String, String, String)>> {
        let matching = self.matching_owned(scope, pattern)?;
        if matching.is_empty() {
            return Err(EnvxError::VarNotFound(format!("{scope}:{pattern}")).into());
        }
        let mut replaced = Vec::new();
        for variable in matching {
            self.set(scope, &variable.name, new_value, Some(variable.kind))?;
            replaced.push((variable.name, variable.value, new_value.to_string()));
        }
        Ok(replaced)
    }

    /// Replaces text within matching variable values in one scope.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid pattern or failed mutation.
    pub fn find_replace(
        &mut self,
        scope: EnvScope,
        search: &str,
        replacement: &str,
        pattern: Option<&str>,
    ) -> Result<Vec<(String, String, String)>> {
        let candidates = if let Some(pattern) = pattern {
            self.matching_owned(scope, pattern)?
        } else {
            self.list(Some(scope)).into_iter().cloned().collect()
        };
        let mut replaced = Vec::new();
        for variable in candidates
            .into_iter()
            .filter(|variable| variable.value.contains(search))
        {
            let new_value = variable.value.replace(search, replacement);
            self.set(scope, &variable.name, &new_value, Some(variable.kind))?;
            replaced.push((variable.name, variable.value, new_value));
        }
        Ok(replaced)
    }

    /// Restores an encrypted backup and creates an undo backup first.
    ///
    /// # Errors
    ///
    /// Returns an error for a missing or invalid backup, denied access, or failed restore and rollback.
    pub fn restore_backup(&mut self, id: &str) -> Result<()> {
        let backup = BackupManager::new()?.load(id)?;
        for scope in backup.entries.iter().map(|entry| entry.scope) {
            self.backend.preflight_write(scope)?;
        }

        let targets: Vec<_> = backup
            .entries
            .iter()
            .map(|entry| (entry.scope, entry.name.clone()))
            .collect();
        self.create_backup(&targets)?;

        let current: Vec<_> = backup
            .entries
            .iter()
            .map(|entry| self.get(entry.scope, &entry.name).cloned())
            .collect();
        for (applied, (entry, current_value)) in backup.entries.iter().zip(&current).enumerate() {
            let result = if let Some(variable) = &entry.before {
                self.backend
                    .set(entry.scope, &entry.name, &variable.value, variable.kind)
            } else if current_value.is_some() {
                self.backend.delete(entry.scope, &entry.name)
            } else {
                Ok(())
            };
            if let Err(error) = result {
                for (rollback_entry, rollback_value) in backup.entries[..applied].iter().zip(&current[..applied]).rev()
                {
                    if let Some(variable) = rollback_value {
                        self.backend
                            .set(
                                rollback_entry.scope,
                                &rollback_entry.name,
                                &variable.value,
                                variable.kind,
                            )
                            .wrap_err("Backup restore failed and rollback also failed")?;
                    } else if rollback_entry.before.is_some() {
                        self.backend
                            .delete(rollback_entry.scope, &rollback_entry.name)
                            .wrap_err("Backup restore failed and rollback also failed")?;
                    }
                }
                return Err(error);
            }
        }
        self.load_all()
    }

    pub fn take_last_backup_ids(&mut self) -> Vec<String> {
        std::mem::take(&mut self.last_backup_ids)
    }

    fn matching_owned(&self, scope: EnvScope, pattern: &str) -> Result<Vec<EnvVar>> {
        if pattern.matches('*').count() > 1 {
            return Err(eyre!("Multiple wildcards are not supported"));
        }
        Ok(self.get_pattern(pattern, Some(scope)).into_iter().cloned().collect())
    }

    fn create_backup(&mut self, targets: &[(EnvScope, String)]) -> Result<()> {
        if !self.backups_enabled || targets.iter().all(|(scope, _)| !scope.is_persistent()) {
            return Ok(());
        }
        let entries = targets
            .iter()
            .map(|(scope, name)| BackupEntry {
                scope: *scope,
                name: name.clone(),
                before: self.get(*scope, name).cloned(),
            })
            .collect();
        let id = BackupManager::new()?.create(entries)?;
        self.last_backup_ids.push(id);
        Ok(())
    }
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(EnvxError::InvalidName("Variable name cannot be empty".to_string()).into());
    }
    if name.contains('=') {
        return Err(EnvxError::InvalidName(format!("Variable name '{}' cannot contain '='", safe_name(name))).into());
    }
    if name.contains('\0') {
        return Err(EnvxError::InvalidName("Variable name cannot contain a null character".to_string()).into());
    }
    Ok(())
}

fn canonical_name(name: &str) -> String {
    #[cfg(windows)]
    {
        name.to_lowercase()
    }
    #[cfg(not(windows))]
    {
        name.to_string()
    }
}

fn safe_name(name: &str) -> String {
    name.split_once('=')
        .map_or_else(|| name.to_string(), |(prefix, _)| format!("{prefix}=<redacted>"))
}

fn wildcard_to_regex(pattern: &str) -> String {
    let mut regex = String::from("^");
    for character in pattern.chars() {
        match character {
            '*' => regex.push_str(".*"),
            '?' => regex.push('.'),
            '.' | '+' | '^' | '$' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '\\' => {
                regex.push('\\');
                regex.push(character);
            }
            _ => regex.push(character),
        }
    }
    regex.push('$');
    regex
}

/// Splits a pattern containing at most one wildcard into prefix and suffix parts.
///
/// # Errors
///
/// Returns an error when the pattern contains more than one wildcard.
pub fn split_wildcard_pattern(pattern: &str) -> Result<(String, String)> {
    if let Some(position) = pattern.find('*') {
        let suffix = pattern[position + 1..].to_string();
        if suffix.contains('*') {
            return Err(eyre!("Multiple wildcards are not supported"));
        }
        Ok((pattern[..position].to_string(), suffix))
    } else {
        Ok((pattern.to_string(), String::new()))
    }
}

#[cfg(windows)]
fn registry_location(scope: EnvScope) -> Result<(winreg::HKEY, &'static str)> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    match scope {
        EnvScope::User => Ok((HKEY_CURRENT_USER, "Environment")),
        EnvScope::System => Ok((
            HKEY_LOCAL_MACHINE,
            "System\\CurrentControlSet\\Control\\Session Manager\\Environment",
        )),
        EnvScope::Process => Err(eyre!("Process scope does not have a registry location")),
    }
}

#[cfg(windows)]
fn open_registry_scope(scope: EnvScope, flags: u32) -> Result<winreg::RegKey> {
    let (root, subkey) = registry_location(scope)?;
    winreg::RegKey::predef(root)
        .open_subkey_with_flags(subkey, flags)
        .wrap_err_with(|| format!("Cannot open {scope} environment registry key with requested access"))
}

#[cfg(windows)]
fn load_registry_scope(scope: EnvScope) -> Result<Vec<EnvVar>> {
    use winreg::enums::{KEY_READ, REG_EXPAND_SZ, REG_SZ};
    use winreg::types::FromRegValue;
    let key = open_registry_scope(scope, KEY_READ)?;
    key.enum_values()
        .map(|item| {
            let (name, raw) = item?;
            let kind = match raw.vtype {
                REG_SZ => EnvValueKind::String,
                REG_EXPAND_SZ => EnvValueKind::ExpandString,
                unsupported => {
                    return Err(eyre!(
                        "Unsupported registry value type {unsupported:?} for {scope} variable '{}'",
                        safe_name(&name)
                    ));
                }
            };
            let value = String::from_reg_value(&raw)
                .wrap_err_with(|| format!("Invalid string data for {scope} variable '{}'", safe_name(&name)))?;
            Ok(EnvVar {
                name,
                value,
                scope,
                kind,
                modified: Utc::now(),
                original_value: None,
            })
        })
        .collect()
}

#[cfg(windows)]
fn set_registry_value(scope: EnvScope, name: &str, value: &str, kind: EnvValueKind) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use winreg::RegValue;
    use winreg::enums::{KEY_SET_VALUE, REG_EXPAND_SZ, REG_SZ};
    let key = open_registry_scope(scope, KEY_SET_VALUE)?;
    let bytes = std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    let raw = RegValue {
        bytes,
        vtype: match kind {
            EnvValueKind::String => REG_SZ,
            EnvValueKind::ExpandString => REG_EXPAND_SZ,
        },
    };
    key.set_raw_value(name, &raw)
        .wrap_err_with(|| format!("Failed to set '{}' in {scope} scope", safe_name(name)))
}

#[cfg(windows)]
fn broadcast_environment_change() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
    };
    let environment: Vec<u16> = "Environment\0".encode_utf16().collect();
    let mut result = 0usize;
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            environment.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            5_000,
            &raw mut result,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeBackend {
        values: Mutex<HashMap<(EnvScope, String), (String, EnvValueKind)>>,
        deny_system: bool,
    }

    impl EnvBackend for FakeBackend {
        fn load(&self, scope: EnvScope) -> Result<Vec<EnvVar>> {
            Ok(self
                .values
                .lock()
                .expect("lock")
                .iter()
                .filter(|((entry_scope, _), _)| *entry_scope == scope)
                .map(|((_, name), (value, kind))| EnvVar {
                    name: name.clone(),
                    value: value.clone(),
                    scope,
                    kind: *kind,
                    modified: Utc::now(),
                    original_value: None,
                })
                .collect())
        }

        fn preflight_write(&self, scope: EnvScope) -> Result<()> {
            if self.deny_system && scope == EnvScope::System {
                Err(eyre!("access denied"))
            } else {
                Ok(())
            }
        }

        fn set(&self, scope: EnvScope, name: &str, value: &str, kind: EnvValueKind) -> Result<()> {
            self.values
                .lock()
                .expect("lock")
                .insert((scope, name.to_string()), (value.to_string(), kind));
            Ok(())
        }

        fn delete(&self, scope: EnvScope, name: &str) -> Result<()> {
            self.values
                .lock()
                .expect("lock")
                .remove(&(scope, name.to_string()))
                .ok_or_else(|| eyre!("not found"))?;
            Ok(())
        }
    }

    fn manager_with_duplicate_names() -> EnvVarManager {
        let backend = Arc::new(FakeBackend::default());
        backend
            .set(EnvScope::Process, "Path", "process", EnvValueKind::String)
            .expect("set");
        backend
            .set(EnvScope::User, "Path", "user", EnvValueKind::ExpandString)
            .expect("set");
        backend
            .set(EnvScope::System, "Path", "system", EnvValueKind::ExpandString)
            .expect("set");
        let mut manager = EnvVarManager::with_backend(backend);
        manager.load_all().expect("load");
        manager
    }

    #[test]
    fn preserves_same_name_in_each_scope() {
        let manager = manager_with_duplicate_names();
        assert_eq!(manager.get_any("Path").len(), 3);
        assert_eq!(manager.get(EnvScope::User, "PATH").expect("user").value, "user");
        assert_eq!(manager.get(EnvScope::System, "path").expect("system").value, "system");
    }

    #[test]
    fn mutation_only_changes_requested_scope() {
        let mut manager = manager_with_duplicate_names();
        manager.set(EnvScope::User, "Path", "new-user", None).expect("set user");
        assert_eq!(manager.get(EnvScope::User, "Path").expect("user").value, "new-user");
        assert_eq!(
            manager.get(EnvScope::User, "Path").expect("user").kind,
            EnvValueKind::ExpandString
        );
        assert_eq!(manager.get(EnvScope::System, "Path").expect("system").value, "system");
    }

    #[test]
    fn denied_system_write_does_not_fall_back() {
        let backend = Arc::new(FakeBackend {
            deny_system: true,
            ..FakeBackend::default()
        });
        let mut manager = EnvVarManager::with_backend(backend);
        assert!(manager.set(EnvScope::System, "TEST", "value", None).is_err());
        assert!(manager.get(EnvScope::User, "TEST").is_none());
    }

    #[test]
    fn rejects_invalid_names() {
        let backend = Arc::new(FakeBackend::default());
        let mut manager = EnvVarManager::with_backend(backend);
        assert!(manager.set(EnvScope::User, "NAME=value", "", None).is_err());
    }
}
