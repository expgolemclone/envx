use crate::{EnvScope, EnvVar};
use chrono::{DateTime, Utc};
use color_eyre::Result;
use color_eyre::eyre::{WrapErr, eyre};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const BACKUP_EXTENSION: &str = "dpapi";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupEntry {
    pub scope: EnvScope,
    pub name: String,
    pub before: Option<EnvVar>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentBackup {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub entries: Vec<BackupEntry>,
}

#[derive(Debug, Clone)]
pub struct BackupSummary {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub entry_count: usize,
}

pub struct BackupManager {
    storage_dir: PathBuf,
}

impl BackupManager {
    /// Creates a backup manager using the current user's local data directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the local data directory is unavailable or cannot be created.
    pub fn new() -> Result<Self> {
        let storage_dir = dirs::data_local_dir()
            .ok_or_else(|| eyre!("Could not locate the local application data directory"))?
            .join("envx")
            .join("backups");
        Self::with_storage_dir(storage_dir)
    }

    /// Creates a backup manager using an explicit storage directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the storage directory cannot be created.
    pub fn with_storage_dir(storage_dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&storage_dir)
            .wrap_err_with(|| format!("Cannot create backup directory '{}'", storage_dir.display()))?;
        Ok(Self { storage_dir })
    }

    /// Encrypts and atomically stores a backup.
    ///
    /// # Errors
    ///
    /// Returns an error when serialization, DPAPI encryption, or file I/O fails.
    pub fn create(&self, entries: Vec<BackupEntry>) -> Result<String> {
        let backup = EnvironmentBackup {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: Utc::now(),
            entries,
        };
        let plaintext = serde_json::to_vec(&backup)?;
        let protected = protect(&plaintext)?;
        let destination = self.path_for(&backup.id);
        let temporary = self.storage_dir.join(format!("{}.tmp", uuid::Uuid::new_v4()));

        fs::write(&temporary, protected)
            .wrap_err_with(|| format!("Cannot write temporary backup '{}'", temporary.display()))?;
        if let Err(error) = fs::rename(&temporary, &destination) {
            let _ = fs::remove_file(&temporary);
            return Err(error).wrap_err_with(|| format!("Cannot finalize backup '{}'", destination.display()));
        }
        Ok(backup.id)
    }

    /// Loads and decrypts a backup by UUID.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid UUID, missing file, decryption failure, or invalid payload.
    pub fn load(&self, id: &str) -> Result<EnvironmentBackup> {
        validate_id(id)?;
        let path = self.path_for(id);
        let protected = fs::read(&path).wrap_err_with(|| format!("Cannot read backup '{}'", path.display()))?;
        let plaintext = unprotect(&protected)?;
        let backup: EnvironmentBackup =
            serde_json::from_slice(&plaintext).wrap_err_with(|| format!("Backup '{id}' has invalid content"))?;
        if backup.id != id {
            return Err(eyre!("Backup id does not match its filename"));
        }
        Ok(backup)
    }

    /// Lists decryptable backups from newest to oldest.
    ///
    /// # Errors
    ///
    /// Returns an error when the directory cannot be read or any backup cannot be decrypted.
    pub fn list(&self) -> Result<Vec<BackupSummary>> {
        let mut summaries = Vec::new();
        for entry in fs::read_dir(&self.storage_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some(BACKUP_EXTENSION) {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|name| name.to_str()) else {
                return Err(eyre!("Backup filename is not valid Unicode"));
            };
            let backup = self.load(id)?;
            summaries.push(BackupSummary {
                id: backup.id,
                created_at: backup.created_at,
                entry_count: backup.entries.len(),
            });
        }
        summaries.sort_by_key(|summary| std::cmp::Reverse(summary.created_at));
        Ok(summaries)
    }

    fn path_for(&self, id: &str) -> PathBuf {
        self.storage_dir.join(format!("{id}.{BACKUP_EXTENSION}"))
    }
}

fn validate_id(id: &str) -> Result<()> {
    uuid::Uuid::parse_str(id)
        .map(|_| ())
        .map_err(|_| eyre!("Invalid backup id '{id}'"))
}

#[cfg(windows)]
fn protect(plaintext: &[u8]) -> Result<Vec<u8>> {
    use std::ptr;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData};

    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(plaintext.len()).wrap_err("Backup is too large for DPAPI")?,
        pbData: plaintext.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };
    let success = unsafe {
        CryptProtectData(
            &raw const input,
            ptr::null(),
            ptr::null(),
            ptr::null_mut(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &raw mut output,
        )
    };
    if success == 0 {
        return Err(std::io::Error::last_os_error()).wrap_err("DPAPI failed to encrypt the backup");
    }
    let protected = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe { LocalFree(output.pbData.cast()) };
    Ok(protected)
}

#[cfg(windows)]
fn unprotect(protected: &[u8]) -> Result<Vec<u8>> {
    use std::ptr;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptUnprotectData,
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(protected.len()).wrap_err("Backup is too large for DPAPI")?,
        pbData: protected.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };
    let success = unsafe {
        CryptUnprotectData(
            &raw const input,
            ptr::null_mut(),
            ptr::null(),
            ptr::null_mut(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &raw mut output,
        )
    };
    if success == 0 {
        return Err(std::io::Error::last_os_error()).wrap_err("DPAPI failed to decrypt the backup");
    }
    let plaintext = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe { LocalFree(output.pbData.cast()) };
    Ok(plaintext)
}

#[cfg(not(windows))]
fn protect(_plaintext: &[u8]) -> Result<Vec<u8>> {
    Err(eyre!("DPAPI backups are only supported on Windows"))
}

#[cfg(not(windows))]
fn unprotect(_protected: &[u8]) -> Result<Vec<u8>> {
    Err(eyre!("DPAPI backups are only supported on Windows"))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::{EnvValueKind, EnvVar};

    #[test]
    fn dpapi_round_trip_preserves_secret_names_and_registry_kind() {
        let directory = tempfile::tempdir().expect("tempdir");
        let manager = BackupManager::with_storage_dir(directory.path().to_path_buf()).expect("manager");
        let entry = BackupEntry {
            scope: EnvScope::User,
            name: "TOKEN=secret".to_string(),
            before: Some(EnvVar {
                name: "TOKEN=secret".to_string(),
                value: String::new(),
                scope: EnvScope::User,
                kind: EnvValueKind::ExpandString,
                modified: Utc::now(),
                original_value: None,
            }),
        };
        let id = manager.create(vec![entry]).expect("create");
        let loaded = manager.load(&id).expect("load");
        assert_eq!(loaded.entries[0].name, "TOKEN=secret");
        assert_eq!(
            loaded.entries[0].before.as_ref().expect("before").kind,
            EnvValueKind::ExpandString
        );
    }
}
