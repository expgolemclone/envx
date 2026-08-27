use crate::snapshot::Snapshot;
use crate::{EnvKey, EnvVar, EnvVarManager};
use ahash::AHashMap as HashMap;
use color_eyre::Result;
use color_eyre::eyre::eyre;
use std::fs;
use std::path::PathBuf;

pub struct SnapshotManager {
    storage_dir: PathBuf,
}

impl SnapshotManager {
    /// Creates a new `SnapshotManager`.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - The system data/config directory cannot be found
    /// - The snapshots directory cannot be created due to filesystem errors
    pub fn new() -> Result<Self> {
        let storage_dir = if cfg!(windows) {
            dirs::data_dir()
                .ok_or_else(|| eyre!("Could not find data directory"))?
                .join("envx")
                .join("snapshots")
        } else {
            dirs::config_dir()
                .ok_or_else(|| eyre!("Could not find config directory"))?
                .join("envx")
                .join("snapshots")
        };

        fs::create_dir_all(&storage_dir)?;
        Ok(Self { storage_dir })
    }

    /// Creates a new snapshot with the given name, description, and environment variables.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - There are file system errors when writing the snapshot file to disk
    /// - JSON serialization of the snapshot fails
    pub fn create(&self, name: String, description: Option<String>, vars: Vec<EnvVar>) -> Result<Snapshot> {
        let snapshot = Snapshot::from_vars(name, description, vars);
        self.save_snapshot(&snapshot)?;
        Ok(snapshot)
    }

    /// Lists all snapshots sorted by creation date (newest first).
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - There are file system errors when reading the snapshots directory
    /// - There are file system errors when reading individual snapshot files
    pub fn list(&self) -> Result<Vec<Snapshot>> {
        let mut snapshots = Vec::new();

        for entry in fs::read_dir(&self.storage_dir)? {
            let entry = entry?;
            if entry.path().extension().and_then(|s| s.to_str()) == Some("json") {
                let content = fs::read_to_string(entry.path())?;
                if let Ok(snapshot) = serde_json::from_str::<Snapshot>(&content) {
                    snapshots.push(snapshot);
                }
            }
        }

        // Sort by creation date (newest first)
        snapshots.sort_by_key(|snapshot| std::cmp::Reverse(snapshot.created_at));
        Ok(snapshots)
    }

    /// Gets a snapshot by ID or name.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - The snapshot cannot be found by ID or name
    /// - There are file system errors when reading the snapshot file
    /// - JSON deserialization fails for the snapshot file
    pub fn get(&self, id_or_name: &str) -> Result<Snapshot> {
        // Try by ID first
        let id_path = self.storage_dir.join(format!("{id_or_name}.json"));
        if id_path.exists() {
            let content = fs::read_to_string(&id_path)?;
            return Ok(serde_json::from_str(&content)?);
        }

        // Try by name
        for snapshot in self.list()? {
            if snapshot.name == id_or_name {
                return Ok(snapshot);
            }
        }

        Err(eyre!("Snapshot not found: {}", id_or_name))
    }

    /// Deletes a snapshot by ID or name.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - The snapshot cannot be found by ID or name
    /// - There are file system errors when deleting the snapshot file
    pub fn delete(&self, id_or_name: &str) -> Result<()> {
        let snapshot = self.get(id_or_name)?;
        let path = self.storage_dir.join(format!("{}.json", snapshot.id));
        fs::remove_file(path)?;
        Ok(())
    }

    /// Applies every environment variable from a snapshot to its recorded scope.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - The snapshot cannot be found by ID or name
    /// - There are file system errors when reading the snapshot file
    /// - JSON deserialization fails for the snapshot file
    /// - Setting environment variables in the manager fails
    pub fn restore(&self, id_or_name: &str, manager: &mut EnvVarManager) -> Result<()> {
        let snapshot = self.get(id_or_name)?;

        // Keep the loaded current state so each mutation can back up the value it replaces.
        for var in snapshot.variables {
            manager.set(var.scope, &var.name, &var.value, Some(var.kind))?;
        }

        Ok(())
    }

    /// Compares two snapshots and returns the differences between them.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - Either snapshot cannot be found by ID or name
    /// - There are file system errors when reading snapshot files
    /// - JSON deserialization fails for the snapshot files
    pub fn diff(&self, snapshot1: &str, snapshot2: &str) -> Result<SnapshotDiff> {
        let snap1 = self.get(snapshot1)?;
        let snap2 = self.get(snapshot2)?;

        let mut diff = SnapshotDiff::default();

        let vars1: HashMap<EnvKey, &EnvVar> = snap1
            .variables
            .iter()
            .map(|variable| (EnvKey::new(variable.scope, &variable.name), variable))
            .collect();
        let vars2: HashMap<EnvKey, &EnvVar> = snap2
            .variables
            .iter()
            .map(|variable| (EnvKey::new(variable.scope, &variable.name), variable))
            .collect();

        // Find added and modified.
        for (key, var2) in &vars2 {
            match vars1.get(key) {
                Some(var1) => {
                    if var1.value != var2.value {
                        diff.modified.push(((*var1).clone(), (*var2).clone()));
                    }
                }
                None => {
                    diff.added.push((*var2).clone());
                }
            }
        }

        // Find removed.
        for (key, var1) in &vars1 {
            if !vars2.contains_key(key) {
                diff.removed.push((*var1).clone());
            }
        }

        Ok(diff)
    }

    fn save_snapshot(&self, snapshot: &Snapshot) -> color_eyre::Result<()> {
        let path = self.storage_dir.join(format!("{}.json", snapshot.id));
        let content = serde_json::to_string_pretty(snapshot)?;
        fs::write(path, content)?;
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct SnapshotDiff {
    pub added: Vec<EnvVar>,
    pub removed: Vec<EnvVar>,
    pub modified: Vec<(EnvVar, EnvVar)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EnvScope, EnvVar};
    use chrono::Utc;
    use tempfile::TempDir;

    fn create_test_snapshot_manager() -> (SnapshotManager, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let storage_dir = temp_dir.path().join("snapshots");
        fs::create_dir_all(&storage_dir).unwrap();

        let manager = SnapshotManager { storage_dir };
        (manager, temp_dir)
    }

    fn create_test_env_var(name: &str, value: &str) -> EnvVar {
        EnvVar {
            name: name.to_string(),
            value: value.to_string(),
            scope: EnvScope::Process,
            kind: crate::EnvValueKind::String,
            modified: Utc::now(),
            original_value: None,
        }
    }

    fn create_test_env_manager() -> EnvVarManager {
        let mut manager = EnvVarManager::new();
        manager.set(EnvScope::Process, "VAR1", "value1", None).unwrap();
        manager.set(EnvScope::Process, "VAR2", "value2", None).unwrap();
        manager.set(EnvScope::Process, "VAR3", "value3", None).unwrap();
        manager
    }

    #[test]
    fn test_snapshot_manager_new() {
        // Test with temporary directory to avoid system dependencies
        let temp_dir = TempDir::new().unwrap();
        let storage_dir = temp_dir.path().join("envx").join("snapshots");

        // Manually create the manager with test directory
        let manager = SnapshotManager {
            storage_dir: storage_dir.clone(),
        };

        // Verify storage directory is set correctly
        assert_eq!(manager.storage_dir, storage_dir);
    }

    #[test]
    fn test_create_snapshot() {
        let (manager, _temp) = create_test_snapshot_manager();

        let vars = vec![
            create_test_env_var("TEST_VAR1", "test_value1"),
            create_test_env_var("TEST_VAR2", "test_value2"),
        ];

        let result = manager.create("test-snapshot".to_string(), Some("Test description".to_string()), vars);

        assert!(result.is_ok());
        let snapshot = result.unwrap();

        assert_eq!(snapshot.name, "test-snapshot");
        assert_eq!(snapshot.description, Some("Test description".to_string()));
        assert_eq!(snapshot.variables.len(), 2);
        assert!(snapshot.variables.iter().any(|variable| variable.name == "TEST_VAR1"));
        assert!(snapshot.variables.iter().any(|variable| variable.name == "TEST_VAR2"));

        // Verify snapshot was saved to disk
        let snapshot_path = manager.storage_dir.join(format!("{}.json", snapshot.id));
        assert!(snapshot_path.exists());
    }

    #[test]
    fn test_create_snapshot_without_description() {
        let (manager, _temp) = create_test_snapshot_manager();

        let vars = vec![create_test_env_var("TEST_VAR", "test_value")];
        let result = manager.create("no-desc".to_string(), None, vars);

        assert!(result.is_ok());
        assert!(result.unwrap().description.is_none());
    }

    #[test]
    fn test_list_snapshots_empty() {
        let (manager, _temp) = create_test_snapshot_manager();

        let result = manager.list();
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_list_snapshots_multiple() {
        let (manager, _temp) = create_test_snapshot_manager();

        // Create multiple snapshots
        let vars = vec![create_test_env_var("VAR", "value")];
        manager.create("snap1".to_string(), None, vars.clone()).unwrap();

        // Add a small delay to ensure different timestamps
        std::thread::sleep(std::time::Duration::from_millis(10));

        manager.create("snap2".to_string(), None, vars.clone()).unwrap();
        manager.create("snap3".to_string(), None, vars).unwrap();

        let snapshots = manager.list().unwrap();
        assert_eq!(snapshots.len(), 3);

        // Verify they are sorted by creation date (newest first)
        assert_eq!(snapshots[0].name, "snap3");
        assert_eq!(snapshots[1].name, "snap2");
        assert_eq!(snapshots[2].name, "snap1");
    }

    #[test]
    fn test_list_snapshots_handles_invalid_files() {
        let (manager, _temp) = create_test_snapshot_manager();

        // Create a valid snapshot
        let vars = vec![create_test_env_var("VAR", "value")];
        manager.create("valid".to_string(), None, vars).unwrap();

        // Create an invalid JSON file
        let invalid_path = manager.storage_dir.join("invalid.json");
        fs::write(invalid_path, "{ invalid json }").unwrap();

        // Create a non-JSON file
        let non_json_path = manager.storage_dir.join("not-json.txt");
        fs::write(non_json_path, "some content").unwrap();

        // List should only return valid snapshots
        let snapshots = manager.list().unwrap();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].name, "valid");
    }

    #[test]
    fn test_get_snapshot_by_id() {
        let (manager, _temp) = create_test_snapshot_manager();

        let vars = vec![create_test_env_var("VAR", "value")];
        let created = manager.create("test".to_string(), None, vars).unwrap();

        let retrieved = manager.get(&created.id).unwrap();
        assert_eq!(retrieved.id, created.id);
        assert_eq!(retrieved.name, created.name);
    }

    #[test]
    fn test_get_snapshot_by_name() {
        let (manager, _temp) = create_test_snapshot_manager();

        let vars = vec![create_test_env_var("VAR", "value")];
        manager.create("test-name".to_string(), None, vars).unwrap();

        let retrieved = manager.get("test-name").unwrap();
        assert_eq!(retrieved.name, "test-name");
    }

    #[test]
    fn test_get_snapshot_not_found() {
        let (manager, _temp) = create_test_snapshot_manager();

        let result = manager.get("nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Snapshot not found"));
    }

    #[test]
    fn test_get_snapshot_prefers_id_over_name() {
        let (manager, _temp) = create_test_snapshot_manager();

        // Create two snapshots where one's name matches another's ID
        let vars = vec![create_test_env_var("VAR", "value")];
        let snap1 = manager.create("first".to_string(), None, vars.clone()).unwrap();

        // Create second snapshot with name equal to first snapshot's ID
        manager.create(snap1.id.clone(), None, vars).unwrap();

        // Getting by snap1.id should return snap1, not the one named with snap1.id
        let retrieved = manager.get(&snap1.id).unwrap();
        assert_eq!(retrieved.name, "first");
    }

    #[test]
    fn test_delete_snapshot() {
        let (manager, _temp) = create_test_snapshot_manager();

        let vars = vec![create_test_env_var("VAR", "value")];
        let snapshot = manager.create("to-delete".to_string(), None, vars).unwrap();

        // Verify it exists
        assert!(manager.get(&snapshot.id).is_ok());

        // Delete it
        let result = manager.delete(&snapshot.id);
        assert!(result.is_ok());

        // Verify it's gone
        assert!(manager.get(&snapshot.id).is_err());

        // Verify file is deleted
        let snapshot_path = manager.storage_dir.join(format!("{}.json", snapshot.id));
        assert!(!snapshot_path.exists());
    }

    #[test]
    fn test_delete_snapshot_by_name() {
        let (manager, _temp) = create_test_snapshot_manager();

        let vars = vec![create_test_env_var("VAR", "value")];
        manager.create("delete-by-name".to_string(), None, vars).unwrap();

        let result = manager.delete("delete-by-name");
        assert!(result.is_ok());
        assert!(manager.get("delete-by-name").is_err());
    }

    #[test]
    fn test_delete_nonexistent_snapshot() {
        let (manager, _temp) = create_test_snapshot_manager();

        let result = manager.delete("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_restore_snapshot() {
        let (manager, _temp) = create_test_snapshot_manager();
        let mut env_manager = create_test_env_manager();

        // Create snapshot
        let vars = vec![
            create_test_env_var("NEW_VAR1", "new_value1"),
            create_test_env_var("NEW_VAR2", "new_value2"),
        ];
        let snapshot = manager.create("to-restore".to_string(), None, vars).unwrap();

        // Restore it
        let result = manager.restore(&snapshot.id, &mut env_manager);
        assert!(result.is_ok());

        // Existing variables remain available and snapshot variables are applied.
        assert_eq!(env_manager.get(EnvScope::Process, "VAR1").unwrap().value, "value1");
        assert_eq!(env_manager.get(EnvScope::Process, "VAR2").unwrap().value, "value2");
        assert_eq!(env_manager.get(EnvScope::Process, "VAR3").unwrap().value, "value3");

        assert_eq!(
            env_manager.get(EnvScope::Process, "NEW_VAR1").unwrap().value,
            "new_value1"
        );
        assert_eq!(
            env_manager.get(EnvScope::Process, "NEW_VAR2").unwrap().value,
            "new_value2"
        );
    }

    #[test]
    fn test_restore_nonexistent_snapshot() {
        let (manager, _temp) = create_test_snapshot_manager();
        let mut env_manager = create_test_env_manager();

        let result = manager.restore("nonexistent", &mut env_manager);
        assert!(result.is_err());
    }

    #[test]
    fn test_diff_snapshots_no_changes() {
        let (manager, _temp) = create_test_snapshot_manager();

        let vars = vec![
            create_test_env_var("VAR1", "value1"),
            create_test_env_var("VAR2", "value2"),
        ];

        let snap1 = manager.create("snap1".to_string(), None, vars.clone()).unwrap();
        let snap2 = manager.create("snap2".to_string(), None, vars).unwrap();

        let diff = manager.diff(&snap1.id, &snap2.id).unwrap();
        assert!(diff.added.is_empty());
        assert!(diff.removed.is_empty());
        assert!(diff.modified.is_empty());
    }

    #[test]
    fn test_diff_snapshots_with_changes() {
        let (manager, _temp) = create_test_snapshot_manager();

        let vars1 = vec![
            create_test_env_var("VAR1", "value1"),
            create_test_env_var("VAR2", "old_value"),
            create_test_env_var("VAR3", "value3"),
        ];

        let vars2 = vec![
            create_test_env_var("VAR1", "value1"),    // Same
            create_test_env_var("VAR2", "new_value"), // Modified
            create_test_env_var("VAR4", "value4"),    // Added
        ];

        let snap1 = manager.create("snap1".to_string(), None, vars1).unwrap();
        let snap2 = manager.create("snap2".to_string(), None, vars2).unwrap();

        let diff = manager.diff(&snap1.id, &snap2.id).unwrap();

        // Check added
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.added[0].name, "VAR4");
        assert_eq!(diff.added[0].value, "value4");

        // Check removed
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.removed[0].name, "VAR3");
        assert_eq!(diff.removed[0].value, "value3");

        // Check modified
        assert_eq!(diff.modified.len(), 1);
        let (old, new) = &diff.modified[0];
        assert_eq!(old.name, "VAR2");
        assert_eq!(old.value, "old_value");
        assert_eq!(new.value, "new_value");
    }

    #[test]
    fn test_diff_nonexistent_snapshots() {
        let (manager, _temp) = create_test_snapshot_manager();

        let result = manager.diff("nonexistent1", "nonexistent2");
        assert!(result.is_err());
    }

    #[test]
    fn test_save_snapshot_creates_pretty_json() {
        let (manager, _temp) = create_test_snapshot_manager();

        let vars = vec![create_test_env_var("TEST_VAR", "test_value")];
        let snapshot = manager
            .create("pretty-test".to_string(), Some("Pretty JSON test".to_string()), vars)
            .unwrap();

        // Read the saved file
        let snapshot_path = manager.storage_dir.join(format!("{}.json", snapshot.id));
        let content = fs::read_to_string(snapshot_path).unwrap();

        // Verify it's pretty-printed (contains indentation)
        assert!(content.contains("\n  "));
        assert!(content.contains("\"name\": \"pretty-test\""));
        assert!(content.contains("\"description\": \"Pretty JSON test\""));
    }

    #[test]
    fn test_concurrent_operations() {
        let (manager, _temp) = create_test_snapshot_manager();

        // Create multiple snapshots in quick succession
        let mut snapshot_ids = Vec::new();
        for i in 0..5 {
            let vars = vec![create_test_env_var(&format!("VAR{i}"), &format!("value{i}"))];
            let snapshot = manager.create(format!("concurrent-{i}"), None, vars).unwrap();
            snapshot_ids.push(snapshot.id);
        }

        // Verify all can be retrieved
        for id in &snapshot_ids {
            assert!(manager.get(id).is_ok());
        }

        // Verify list returns all
        let snapshots = manager.list().unwrap();
        assert_eq!(snapshots.len(), 5);
    }
}
