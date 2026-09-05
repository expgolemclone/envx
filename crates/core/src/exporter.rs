use crate::EnvVar;
use color_eyre::Result;
use std::fs;

#[path = "exporter_impl.rs"]
mod implementation;

pub use implementation::ExportFormat;

/// Exports environment variables while keeping dotenv escaping compatible with the importer.
pub struct Exporter {
    variables: Vec<EnvVar>,
    include_metadata: bool,
}

impl Exporter {
    #[must_use]
    pub const fn new(variables: Vec<EnvVar>, include_metadata: bool) -> Self {
        Self {
            variables,
            include_metadata,
        }
    }

    #[must_use]
    pub fn count(&self) -> usize {
        self.variables.len()
    }

    /// Exports environment variables to a file in the specified format.
    ///
    /// # Errors
    ///
    /// Returns an error when the destination cannot be written or a delegated serializer fails.
    pub fn export_to_file(&self, path: &str, format: ExportFormat) -> Result<()> {
        if matches!(format, ExportFormat::DotEnv) {
            fs::write(path, self.to_dotenv())?;
            return Ok(());
        }

        implementation::Exporter::new(self.variables.clone(), self.include_metadata)
            .export_to_file(path, format)
    }

    fn to_dotenv(&self) -> String {
        let mut lines = Vec::new();

        if self.include_metadata {
            lines.push("# Environment variables exported by envx".to_string());
            lines.push(format!(
                "# Date: {}",
                chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
            ));
            lines.push(format!("# Count: {}", self.variables.len()));
            lines.push(String::new());
        }

        for var in &self.variables {
            if self.include_metadata {
                lines.push(format!(
                    "# Scope: {:?}, Modified: {}",
                    var.scope,
                    var.modified.format("%Y-%m-%d %H:%M:%S")
                ));
            }

            let needs_quotes = var.value.contains(' ')
                || var.value.contains('=')
                || var.value.contains('#')
                || var.value.contains('"')
                || var.value.contains('\'')
                || var.value.contains('\n')
                || var.value.contains('\r')
                || var.value.contains('\t');

            if needs_quotes {
                // Escape literal backslashes first. This keeps Windows paths such as
                // C:\new folder\temp from being reinterpreted as \n or \t by the importer.
                let escaped_value = var
                    .value
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('\n', "\\n")
                    .replace('\r', "\\r")
                    .replace('\t', "\\t");

                lines.push(format!("{}=\"{}\"", var.name, escaped_value));
            } else {
                lines.push(format!("{}={}", var.name, var.value));
            }
        }

        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EnvScope, EnvValueKind, ImportFormat, Importer};
    use chrono::Utc;
    use std::collections::HashMap;
    use tempfile::NamedTempFile;

    fn test_var(name: &str, value: &str) -> EnvVar {
        EnvVar {
            name: name.to_string(),
            value: value.to_string(),
            scope: EnvScope::User,
            kind: EnvValueKind::String,
            modified: Utc::now(),
            original_value: None,
        }
    }

    #[test]
    fn dotenv_round_trip_preserves_windows_paths_and_escapes() {
        let expected = [
            ("WINDOWS_PATH", r"C:\new folder\temp"),
            ("LITERAL_BACKSLASH", r"regex\w+ with space"),
            ("NEWLINE", "line1\nline2"),
            ("TAB", "left\tright"),
            ("QUOTE", "say \"hello\""),
        ];
        let variables = expected
            .iter()
            .map(|(name, value)| test_var(name, value))
            .collect();
        let exporter = Exporter::new(variables, false);
        let file = NamedTempFile::with_suffix(".env").unwrap();
        let path = file.path().to_str().unwrap();

        exporter.export_to_file(path, ExportFormat::DotEnv).unwrap();

        let mut importer = Importer::new();
        importer.import_from_file(path, ImportFormat::DotEnv).unwrap();
        let imported: HashMap<_, _> = importer.get_variables().into_iter().collect();

        for (name, value) in expected {
            assert_eq!(imported.get(name).map(String::as_str), Some(value));
        }
    }

    #[test]
    fn dotenv_escapes_backslashes_before_control_sequences() {
        let exporter = Exporter::new(
            vec![test_var("WINDOWS_PATH", r"C:\new folder\temp")],
            false,
        );

        assert_eq!(
            exporter.to_dotenv(),
            "WINDOWS_PATH=\"C:\\\\new folder\\\\temp\""
        );
    }
}
