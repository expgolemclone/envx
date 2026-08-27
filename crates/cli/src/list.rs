use color_eyre::Result;
use color_eyre::eyre::eyre;
use comfy_table::{Attribute, Cell, Color, ContentArrangement, Table};
use envx_core::{EnvScope, EnvVar, EnvVarManager};
use serde::Serialize;

#[derive(Serialize)]
struct DisplayVariable<'a> {
    scope: EnvScope,
    name: String,
    value: &'a str,
}

#[allow(clippy::too_many_arguments)]
/// Lists environment variables with values redacted unless explicitly revealed.
///
/// # Errors
///
/// Returns an error when environment loading, output serialization, or option validation fails.
pub fn handle_list_command(
    scope: Option<EnvScope>,
    query: Option<&str>,
    format: &str,
    sort: &str,
    names_only: bool,
    limit: Option<usize>,
    stats: bool,
    reveal: bool,
) -> Result<()> {
    let mut manager = EnvVarManager::new();
    manager.load_all()?;
    let mut variables = if let Some(query) = query {
        manager.search(query, scope)
    } else {
        manager.list(scope)
    };

    match sort {
        "name" => variables.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then(left.scope.to_string().cmp(&right.scope.to_string()))
        }),
        "value" => variables.sort_by(|left, right| left.value.cmp(&right.value)),
        "scope" => variables.sort_by(|left, right| {
            left.scope
                .to_string()
                .cmp(&right.scope.to_string())
                .then(left.name.cmp(&right.name))
        }),
        invalid => return Err(eyre!("Invalid sort field '{invalid}'. Use name, value, or scope")),
    }

    let total = variables.len();
    if let Some(limit) = limit {
        variables.truncate(limit);
    }
    if stats {
        println!("Total: {total}");
        for scope in [EnvScope::System, EnvScope::User, EnvScope::Process] {
            println!("{scope}: {}", manager.filter_by_scope(scope).len());
        }
    }
    if names_only {
        for variable in variables {
            println!("{} {}", variable.scope, safe_name(&variable.name));
        }
        return Ok(());
    }

    match format {
        "json" => {
            let display: Vec<_> = variables
                .iter()
                .map(|variable| DisplayVariable {
                    scope: variable.scope,
                    name: safe_name(&variable.name),
                    value: displayed_value(variable, reveal),
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&display)?);
        }
        "simple" | "compact" => {
            for variable in variables {
                println!(
                    "{} {} = {}",
                    variable.scope,
                    safe_name(&variable.name),
                    displayed_value(variable, reveal)
                );
            }
        }
        "table" => print_table(&variables, reveal),
        invalid => return Err(eyre!("Invalid format '{invalid}'. Use table, json, simple, or compact")),
    }
    Ok(())
}

fn print_table(variables: &[&EnvVar], reveal: bool) {
    let mut table = Table::new();
    table
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            Cell::new("Scope").add_attribute(Attribute::Bold).fg(Color::Cyan),
            Cell::new("Name").add_attribute(Attribute::Bold).fg(Color::Cyan),
            Cell::new("Value").add_attribute(Attribute::Bold).fg(Color::Cyan),
        ]);
    for variable in variables {
        table.add_row(vec![
            Cell::new(variable.scope),
            Cell::new(safe_name(&variable.name)),
            Cell::new(displayed_value(variable, reveal)),
        ]);
    }
    println!("{table}");
}

pub fn displayed_value(variable: &EnvVar, reveal: bool) -> &str {
    if reveal && !variable.name.contains('=') {
        &variable.value
    } else {
        "[REDACTED]"
    }
}

pub fn safe_name(name: &str) -> String {
    name.split_once('=')
        .map_or_else(|| name.to_string(), |(prefix, _)| format!("{prefix}=<redacted>"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use envx_core::EnvValueKind;

    #[test]
    fn invalid_name_never_reveals_embedded_value() {
        let variable = EnvVar {
            name: "TOKEN=secret".to_string(),
            value: String::new(),
            scope: EnvScope::User,
            kind: EnvValueKind::String,
            modified: Utc::now(),
            original_value: None,
        };
        assert_eq!(safe_name(&variable.name), "TOKEN=<redacted>");
        assert_eq!(displayed_value(&variable, true), "[REDACTED]");
    }
}
