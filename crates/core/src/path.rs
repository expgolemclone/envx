use color_eyre::Result;
use color_eyre::eyre::WrapErr;
use std::collections::HashSet;

/// Manages PATH-like environment variables
pub struct PathManager {
    entries: Vec<String>,
    separator: char,
}

impl PathManager {
    #[must_use]
    pub fn new(path_value: &str) -> Self {
        let separator = if cfg!(windows) { ';' } else { ':' };
        let entries = path_value
            .split(separator)
            .filter(|s| !s.is_empty())
            .map(std::string::ToString::to_string)
            .collect();

        Self { entries, separator }
    }

    #[must_use]
    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Tests whether an equivalent path is present after environment expansion and normalization.
    ///
    /// # Errors
    ///
    /// Returns an error when Windows cannot expand an environment expression.
    pub fn contains(&self, path: &str) -> Result<bool> {
        let normalized = Self::normalize_path(path)?;
        for entry in &self.entries {
            if Self::normalize_path(entry)? == normalized {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Finds an equivalent path after environment expansion and normalization.
    ///
    /// # Errors
    ///
    /// Returns an error when Windows cannot expand an environment expression.
    pub fn find_index(&self, path: &str) -> Result<Option<usize>> {
        let normalized = Self::normalize_path(path)?;
        for (index, entry) in self.entries.iter().enumerate() {
            if Self::normalize_path(entry)? == normalized {
                return Ok(Some(index));
            }
        }
        Ok(None)
    }

    pub fn add_first(&mut self, path: String) {
        self.entries.insert(0, path);
    }

    pub fn add_last(&mut self, path: String) {
        self.entries.push(path);
    }

    /// Removes the first equivalent path.
    ///
    /// # Errors
    ///
    /// Returns an error when Windows cannot expand an environment expression.
    pub fn remove_first(&mut self, pattern: &str) -> Result<usize> {
        if let Some(idx) = self.find_index(pattern)? {
            self.entries.remove(idx);
            Ok(1)
        } else {
            Ok(0)
        }
    }

    /// Removes every equivalent path.
    ///
    /// # Errors
    ///
    /// Returns an error when Windows cannot expand an environment expression.
    pub fn remove_all(&mut self, pattern: &str) -> Result<usize> {
        let normalized = Self::normalize_path(pattern)?;
        let original_len = self.entries.len();

        let normalized_entries: Vec<String> = self
            .entries
            .iter()
            .map(|entry| Self::normalize_path(entry))
            .collect::<Result<_>>()?;

        // Keep only entries that don't match the normalized pattern
        let mut new_entries = Vec::new();
        for (i, entry) in self.entries.iter().enumerate() {
            if normalized_entries[i] != normalized {
                new_entries.push(entry.clone());
            }
        }
        self.entries = new_entries;

        Ok(original_len - self.entries.len())
    }

    /// Moves an entry from one position to another in the PATH entries.
    ///
    /// # Errors
    ///
    /// Returns an error if either `from` or `to` index is out of bounds.
    pub fn move_entry(&mut self, from: usize, to: usize) -> Result<()> {
        if from >= self.entries.len() || to >= self.entries.len() {
            return Err(color_eyre::eyre::eyre!("Index out of bounds"));
        }

        if from == to {
            return Ok(()); // No-op if moving to same position
        }

        let entry = self.entries.remove(from);

        self.entries.insert(to, entry);

        Ok(())
    }

    /// Returns entries whose expanded path does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error when Windows cannot expand an environment expression.
    pub fn get_invalid(&self) -> Result<Vec<String>> {
        let mut invalid = Vec::new();
        for entry in &self.entries {
            if !Self::resolved_path(entry)?.exists() {
                invalid.push(entry.clone());
            }
        }
        Ok(invalid)
    }

    /// Removes entries whose expanded path does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error when Windows cannot expand an environment expression.
    pub fn remove_invalid(&mut self) -> Result<usize> {
        let original_len = self.entries.len();
        let mut valid = Vec::new();
        for entry in &self.entries {
            if Self::resolved_path(entry)?.exists() {
                valid.push(entry.clone());
            }
        }
        self.entries = valid;
        Ok(original_len - self.entries.len())
    }

    /// Returns repeated entries after environment expansion and normalization.
    ///
    /// # Errors
    ///
    /// Returns an error when Windows cannot expand an environment expression.
    pub fn get_duplicates(&self) -> Result<Vec<String>> {
        let mut seen = HashSet::new();
        let mut duplicates = Vec::new();

        for entry in &self.entries {
            let normalized = Self::normalize_path(entry)?;
            if !seen.insert(normalized.clone()) {
                duplicates.push(entry.clone());
            }
        }

        Ok(duplicates)
    }

    /// Removes repeated entries while preserving either the first or last occurrence.
    ///
    /// # Errors
    ///
    /// Returns an error when Windows cannot expand an environment expression.
    pub fn deduplicate(&mut self, keep_first: bool) -> Result<usize> {
        let mut seen = HashSet::new();
        let original_len = self.entries.len();

        if keep_first {
            // Keep first occurrence
            let mut deduped = Vec::new();
            for entry in &self.entries {
                let normalized = Self::normalize_path(entry)?;
                if seen.insert(normalized) {
                    deduped.push(entry.clone());
                }
            }
            self.entries = deduped;
        } else {
            // Keep last occurrence
            let mut deduped = Vec::new();
            for entry in self.entries.iter().rev() {
                let normalized = Self::normalize_path(entry)?;
                if seen.insert(normalized) {
                    deduped.push(entry.clone());
                }
            }
            deduped.reverse();
            self.entries = deduped;
        }

        Ok(original_len - self.entries.len())
    }

    #[must_use]
    #[allow(clippy::inherent_to_string)]
    pub fn to_string(&self) -> String {
        self.entries.join(&self.separator.to_string())
    }

    /// Resolves a path for comparison or existence checks without changing its stored representation.
    ///
    /// # Errors
    ///
    /// Returns an error when Windows cannot expand an environment expression.
    pub fn resolved_path(path: &str) -> Result<std::path::PathBuf> {
        #[cfg(windows)]
        {
            Ok(std::path::PathBuf::from(expand_windows_environment(path)?))
        }
        #[cfg(not(windows))]
        {
            Ok(std::path::PathBuf::from(path))
        }
    }

    fn normalize_path(path: &str) -> Result<String> {
        let mut normalized = Self::resolved_path(path)?.to_string_lossy().into_owned();

        // Remove trailing slashes
        while normalized.ends_with('/') || normalized.ends_with('\\') {
            normalized.pop();
        }

        // On Windows, normalize to lowercase for case-insensitive comparison
        #[cfg(windows)]
        {
            normalized = normalized.to_lowercase();
        }

        // Convert forward slashes to backslashes on Windows
        #[cfg(windows)]
        {
            normalized = normalized.replace('/', "\\");
        }

        // Convert backslashes to forward slashes on Unix
        #[cfg(unix)]
        {
            normalized = normalized.replace('\\', "/");
        }

        Ok(normalized)
    }
}

#[cfg(windows)]
fn expand_windows_environment(value: &str) -> Result<String> {
    use windows_sys::Win32::System::Environment::ExpandEnvironmentStringsW;

    let input: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    let required = unsafe { ExpandEnvironmentStringsW(input.as_ptr(), std::ptr::null_mut(), 0) };
    if required == 0 {
        return Err(std::io::Error::last_os_error()).wrap_err("Failed to size expanded PATH entry");
    }
    let mut output = vec![0u16; required as usize];
    let written = unsafe { ExpandEnvironmentStringsW(input.as_ptr(), output.as_mut_ptr(), required) };
    if written == 0 || written > required {
        return Err(std::io::Error::last_os_error()).wrap_err("Failed to expand PATH entry");
    }
    output.truncate(written.saturating_sub(1) as usize);
    Ok(String::from_utf16_lossy(&output))
}

// ...existing code...

#[cfg(test)]
mod tests {
    use super::*;

    // Helper function to create a PathManager with test data
    fn create_test_manager() -> PathManager {
        let path = if cfg!(windows) {
            "C:\\Windows;C:\\Program Files;C:\\Users\\Test;C:\\Windows;D:\\Tools"
        } else {
            "/usr/bin:/usr/local/bin:/home/user/bin:/usr/bin:/opt/tools"
        };
        PathManager::new(path)
    }

    #[test]
    fn test_new_empty() {
        let mgr = PathManager::new("");
        assert!(mgr.is_empty());
        assert_eq!(mgr.len(), 0);
    }

    #[test]
    fn test_new_with_paths() {
        let mgr = create_test_manager();
        assert!(!mgr.is_empty());
        assert_eq!(mgr.len(), 5);
    }

    #[test]
    fn test_new_filters_empty_entries() {
        let path = if cfg!(windows) {
            "C:\\Windows;;C:\\Program Files;;;D:\\Tools;"
        } else {
            "/usr/bin::/usr/local/bin:::/opt/tools:"
        };
        let mgr = PathManager::new(path);
        assert_eq!(mgr.len(), 3);
    }

    #[test]
    fn test_separator_detection() {
        let mgr = PathManager::new("");
        if cfg!(windows) {
            assert_eq!(mgr.separator, ';');
        } else {
            assert_eq!(mgr.separator, ':');
        }
    }

    #[test]
    fn test_entries() {
        let mgr = create_test_manager();
        let entries = mgr.entries();
        assert_eq!(entries.len(), 5);
        if cfg!(windows) {
            assert!(entries.contains(&"C:\\Windows".to_string()));
            assert!(entries.contains(&"C:\\Program Files".to_string()));
        } else {
            assert!(entries.contains(&"/usr/bin".to_string()));
            assert!(entries.contains(&"/usr/local/bin".to_string()));
        }
    }

    #[test]
    fn test_contains() {
        let mgr = create_test_manager();
        if cfg!(windows) {
            assert!(mgr.contains("C:\\Windows").unwrap());
            assert!(mgr.contains("c:\\windows").unwrap()); // Case insensitive on Windows
            assert!(mgr.contains("C:/Windows").unwrap()); // Forward slash normalization
            assert!(!mgr.contains("C:\\NonExistent").unwrap());
        } else {
            assert!(mgr.contains("/usr/bin").unwrap());
            assert!(mgr.contains("/usr/bin/").unwrap()); // Trailing slash normalization
            assert!(!mgr.contains("/nonexistent").unwrap());
        }
    }

    #[test]
    fn test_contains_with_trailing_slashes() {
        let mgr = create_test_manager();
        if cfg!(windows) {
            assert!(mgr.contains("C:\\Windows\\").unwrap());
            assert!(mgr.contains("C:\\Windows/").unwrap());
        } else {
            assert!(mgr.contains("/usr/bin/").unwrap());
        }
    }

    #[test]
    fn test_find_index() {
        let mgr = create_test_manager();
        if cfg!(windows) {
            assert_eq!(mgr.find_index("C:\\Windows").unwrap(), Some(0));
            assert_eq!(mgr.find_index("C:\\Program Files").unwrap(), Some(1));
            assert_eq!(mgr.find_index("D:\\Tools").unwrap(), Some(4));
            assert_eq!(mgr.find_index("C:\\NonExistent").unwrap(), None);
        } else {
            assert_eq!(mgr.find_index("/usr/bin").unwrap(), Some(0));
            assert_eq!(mgr.find_index("/opt/tools").unwrap(), Some(4));
            assert_eq!(mgr.find_index("/nonexistent").unwrap(), None);
        }
    }

    #[test]
    fn test_add_first() {
        let mut mgr = create_test_manager();
        let original_len = mgr.len();

        if cfg!(windows) {
            mgr.add_first("C:\\NewPath".to_string());
            assert_eq!(mgr.entries()[0], "C:\\NewPath");
        } else {
            mgr.add_first("/new/path".to_string());
            assert_eq!(mgr.entries()[0], "/new/path");
        }
        assert_eq!(mgr.len(), original_len + 1);
    }

    #[test]
    fn test_add_last() {
        let mut mgr = create_test_manager();
        let original_len = mgr.len();

        if cfg!(windows) {
            mgr.add_last("C:\\NewPath".to_string());
            assert_eq!(mgr.entries()[mgr.len() - 1], "C:\\NewPath");
        } else {
            mgr.add_last("/new/path".to_string());
            assert_eq!(mgr.entries()[mgr.len() - 1], "/new/path");
        }
        assert_eq!(mgr.len(), original_len + 1);
    }

    #[test]
    fn test_remove_first() {
        let mut mgr = create_test_manager();
        let original_len = mgr.len();

        if cfg!(windows) {
            let removed = mgr.remove_first("C:\\Windows").unwrap();
            assert_eq!(removed, 1);
            assert_eq!(mgr.len(), original_len - 1);
            // Should only remove first occurrence
            assert!(mgr.contains("C:\\Windows").unwrap()); // Second occurrence still there

            let removed = mgr.remove_first("C:\\NonExistent").unwrap();
            assert_eq!(removed, 0);
            assert_eq!(mgr.len(), original_len - 1);
        } else {
            let removed = mgr.remove_first("/usr/bin").unwrap();
            assert_eq!(removed, 1);
            assert_eq!(mgr.len(), original_len - 1);
            // Should only remove first occurrence
            assert!(mgr.contains("/usr/bin").unwrap()); // Second occurrence still there
        }
    }

    #[test]
    fn test_remove_all() {
        let mut mgr = create_test_manager();

        if cfg!(windows) {
            let removed = mgr.remove_all("C:\\Windows").unwrap();
            assert_eq!(removed, 2); // There are two C:\Windows entries
            assert!(!mgr.contains("C:\\Windows").unwrap());
            assert_eq!(mgr.len(), 3);
        } else {
            let removed = mgr.remove_all("/usr/bin").unwrap();
            assert_eq!(removed, 2); // There are two /usr/bin entries
            assert!(!mgr.contains("/usr/bin").unwrap());
            assert_eq!(mgr.len(), 3);
        }
    }

    #[test]
    fn test_remove_all_nonexistent() {
        let mut mgr = create_test_manager();
        let original_len = mgr.len();

        let removed = mgr.remove_all("NonExistent").unwrap();
        assert_eq!(removed, 0);
        assert_eq!(mgr.len(), original_len);
    }

    #[test]
    fn test_move_entry() {
        let mut mgr = create_test_manager();
        let first = mgr.entries()[0].clone();
        let second = mgr.entries()[1].clone();

        // Move first to second position
        assert!(mgr.move_entry(0, 1).is_ok());
        assert_eq!(mgr.entries()[0], second);
        assert_eq!(mgr.entries()[1], first);

        // Move back
        assert!(mgr.move_entry(1, 0).is_ok());
        assert_eq!(mgr.entries()[0], first);
        assert_eq!(mgr.entries()[1], second);
    }

    #[test]
    fn test_move_entry_to_end() {
        let mut mgr = create_test_manager();
        let first = mgr.entries()[0].clone();
        let last_idx = mgr.len() - 1;

        assert!(mgr.move_entry(0, last_idx).is_ok());
        assert_eq!(mgr.entries()[last_idx], first);
    }

    #[test]
    fn test_move_entry_out_of_bounds() {
        let mut mgr = create_test_manager();

        assert!(mgr.move_entry(10, 0).is_err());
        assert!(mgr.move_entry(0, 10).is_err());
        assert!(mgr.move_entry(10, 10).is_err());
    }

    #[test]
    fn test_get_duplicates() {
        let mgr = create_test_manager();
        let duplicates = mgr.get_duplicates().unwrap();

        if cfg!(windows) {
            assert_eq!(duplicates.len(), 1);
            assert_eq!(duplicates[0], "C:\\Windows");
        } else {
            assert_eq!(duplicates.len(), 1);
            assert_eq!(duplicates[0], "/usr/bin");
        }
    }

    #[test]
    fn test_get_duplicates_no_dupes() {
        let path = if cfg!(windows) {
            "C:\\Path1;C:\\Path2;C:\\Path3"
        } else {
            "/path1:/path2:/path3"
        };
        let mgr = PathManager::new(path);
        let duplicates = mgr.get_duplicates().unwrap();
        assert!(duplicates.is_empty());
    }

    #[test]
    fn test_get_duplicates_case_insensitive_windows() {
        if cfg!(windows) {
            let mgr = PathManager::new("C:\\Windows;c:\\windows;C:\\WINDOWS");
            let duplicates = mgr.get_duplicates().unwrap();
            assert_eq!(duplicates.len(), 2); // First one is not a duplicate
        }
    }

    #[cfg(windows)]
    #[test]
    fn expands_environment_tokens_for_checks_without_rewriting_raw_entries() {
        let user_profile = std::env::var("USERPROFILE").expect("USERPROFILE");
        let raw = "%USERPROFILE%";
        let manager = PathManager::new(&format!("{raw};{user_profile}"));

        assert_eq!(manager.get_duplicates().unwrap(), vec![user_profile]);
        assert!(manager.get_invalid().unwrap().is_empty());
        assert!(manager.to_string().starts_with(raw));
    }

    #[test]
    fn test_deduplicate_keep_first() {
        let mut mgr = create_test_manager();
        let removed = mgr.deduplicate(true).unwrap();

        assert_eq!(removed, 1); // One duplicate removed
        assert_eq!(mgr.len(), 4);

        // Check no duplicates remain
        let duplicates = mgr.get_duplicates().unwrap();
        assert!(duplicates.is_empty());

        // Verify first occurrence was kept
        if cfg!(windows) {
            assert_eq!(mgr.entries()[0], "C:\\Windows");
        } else {
            assert_eq!(mgr.entries()[0], "/usr/bin");
        }
    }

    #[test]
    fn test_deduplicate_keep_last() {
        let mut mgr = create_test_manager();
        let removed = mgr.deduplicate(false).unwrap();

        assert_eq!(removed, 1); // One duplicate removed
        assert_eq!(mgr.len(), 4);

        // Check no duplicates remain
        let duplicates = mgr.get_duplicates().unwrap();
        assert!(duplicates.is_empty());

        // Verify last occurrence was kept
        if cfg!(windows) {
            // C:\Windows was at index 0 and 3, so after dedup keeping last, it should be at index 2
            assert!(mgr.contains("C:\\Windows").unwrap());
            assert_eq!(mgr.find_index("C:\\Windows").unwrap(), Some(2));
        } else {
            assert!(mgr.contains("/usr/bin").unwrap());
            assert_eq!(mgr.find_index("/usr/bin").unwrap(), Some(2));
        }
    }

    #[test]
    fn test_to_string() {
        let mgr = create_test_manager();
        let result = mgr.to_string();

        if cfg!(windows) {
            // On Windows, paths are separated by semicolons
            assert!(result.contains(';'));
            // Windows paths can contain colons (e.g., C:), so don't check for absence of colons
            assert!(result.contains("C:\\Windows"));
            assert!(result.contains("C:\\Program Files"));

            // Verify the separator is used correctly by counting occurrences
            let separator_count = result.matches(';').count();
            assert_eq!(separator_count, mgr.len() - 1); // n-1 separators for n entries
        } else {
            // On Unix, paths are separated by colons
            assert!(result.contains(':'));
            assert!(!result.contains(';'));
            assert!(result.contains("/usr/bin"));
            assert!(result.contains("/usr/local/bin"));

            // Verify the separator is used correctly by counting occurrences
            let separator_count = result.matches(':').count();
            assert_eq!(separator_count, mgr.len() - 1); // n-1 separators for n entries
        }
    }

    #[test]
    fn test_to_string_empty() {
        let mgr = PathManager::new("");
        assert_eq!(mgr.to_string(), "");
    }

    #[test]
    fn test_to_string_single_entry() {
        let mut mgr = PathManager::new("");
        if cfg!(windows) {
            mgr.add_first("C:\\Single".to_string());
            assert_eq!(mgr.to_string(), "C:\\Single");
        } else {
            mgr.add_first("/single".to_string());
            assert_eq!(mgr.to_string(), "/single");
        }
    }

    #[test]
    fn test_normalize_path_trailing_slashes() {
        if cfg!(windows) {
            assert_eq!(PathManager::normalize_path("C:\\Path\\").unwrap(), "c:\\path");
            assert_eq!(PathManager::normalize_path("C:\\Path/").unwrap(), "c:\\path");
            assert_eq!(PathManager::normalize_path("C:\\Path\\\\").unwrap(), "c:\\path");
        } else {
            assert_eq!(PathManager::normalize_path("/path/").unwrap(), "/path");
            assert_eq!(PathManager::normalize_path("/path//").unwrap(), "/path");
        }
    }

    #[test]
    fn test_normalize_path_case_sensitivity() {
        if cfg!(windows) {
            // Windows: case-insensitive
            assert_eq!(
                PathManager::normalize_path("C:\\Path").unwrap(),
                PathManager::normalize_path("c:\\path").unwrap()
            );
            assert_eq!(
                PathManager::normalize_path("C:\\PATH").unwrap(),
                PathManager::normalize_path("c:\\path").unwrap()
            );
        } else {
            // Unix: case-sensitive
            assert_ne!(
                PathManager::normalize_path("/Path").unwrap(),
                PathManager::normalize_path("/path").unwrap()
            );
            assert_ne!(
                PathManager::normalize_path("/PATH").unwrap(),
                PathManager::normalize_path("/path").unwrap()
            );
        }
    }

    #[test]
    fn test_normalize_path_slash_conversion() {
        if cfg!(windows) {
            // Windows: convert forward slashes to backslashes
            assert_eq!(
                PathManager::normalize_path("C:/Path/To/Dir").unwrap(),
                "c:\\path\\to\\dir"
            );
            assert_eq!(
                PathManager::normalize_path("C:\\Path/To\\Dir").unwrap(),
                "c:\\path\\to\\dir"
            );
        } else {
            // Unix: convert backslashes to forward slashes
            assert_eq!(PathManager::normalize_path("/path\\to\\dir").unwrap(), "/path/to/dir");
            assert_eq!(PathManager::normalize_path("/path\\to/dir").unwrap(), "/path/to/dir");
        }
    }

    // Note: get_invalid() and remove_invalid() tests would require actual filesystem
    // operations or mocking, which is beyond the scope of unit tests.
    // These would be better as integration tests.

    #[test]
    fn test_complex_scenario() {
        let mut mgr = PathManager::new("");

        // Build a complex PATH
        if cfg!(windows) {
            mgr.add_last("C:\\Windows".to_string());
            mgr.add_last("C:\\Program Files".to_string());
            mgr.add_first("C:\\Priority".to_string());
            mgr.add_last("C:\\Windows".to_string()); // Duplicate
            mgr.add_last("c:\\program files".to_string()); // Case variant duplicate

            assert_eq!(mgr.len(), 5);

            // Remove duplicates
            let removed = mgr.deduplicate(true).unwrap();
            assert_eq!(removed, 2);
            assert_eq!(mgr.len(), 3);

            // Verify order
            assert_eq!(mgr.entries()[0], "C:\\Priority");
            assert_eq!(mgr.entries()[1], "C:\\Windows");
            assert_eq!(mgr.entries()[2], "C:\\Program Files");
        } else {
            mgr.add_last("/usr/bin".to_string());
            mgr.add_last("/usr/local/bin".to_string());
            mgr.add_first("/priority".to_string());
            mgr.add_last("/usr/bin".to_string()); // Duplicate
            mgr.add_last("/usr/local/bin/".to_string()); // Trailing slash duplicate

            assert_eq!(mgr.len(), 5);

            // Remove duplicates
            let removed = mgr.deduplicate(true).unwrap();
            assert_eq!(removed, 2);
            assert_eq!(mgr.len(), 3);

            // Verify order
            assert_eq!(mgr.entries()[0], "/priority");
            assert_eq!(mgr.entries()[1], "/usr/bin");
            assert_eq!(mgr.entries()[2], "/usr/local/bin");
        }
    }
}
