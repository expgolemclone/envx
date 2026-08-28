---
name: envx
description: >-
  Manage scoped environment variables, PATH entries, imports, exports, diagnostics, repairs, and backups through the
  envx CLI. Use for environment-variable inspection or changes that envx supports. Do not use for envx profiles,
  projects, snapshots, monitoring, bulk rewrites, or unrelated application configuration.
---

# envx

Use the installed `envx` CLI as a black box. Do not inspect its source, use its TUI, or replace it with direct registry,
shell-profile, or environment-file mutations. If `envx` cannot complete the operation, report its error and stop instead
of falling back to another scope or mechanism.

## Route the request

Read only the references needed for the current request:

- For listing, reading, setting, or deleting ordinary variables, read
  [references/variables.md](references/variables.md).
- For `PATH` or another path-list variable, read [references/path.md](references/path.md).
- For `.env`, JSON, YAML, text, PowerShell, or shell import and export, read [references/files.md](references/files.md).
- For diagnosis, invalid-name repair, or encrypted backup recovery, read
  [references/recovery.md](references/recovery.md).

The supported routes exclude profiles, projects, snapshots, watch, monitor, rename, replace, find-replace, dependency
analysis, cleanup, and TUI operations.

## Shared decisions

- Honor an explicit `process`, `user`, or `system` scope.
- Otherwise choose the scope from the intended consumer and lifetime. Use `user` for persistent settings used by the
  current account and developer tools. Use `system` only for machine-wide consumers, services, or other accounts. A
  standalone `envx` process mutation cannot update its parent or a later process, so use `process` only when that
  limitation is explicitly acceptable.
- When an existing name occurs in more than one scope, treat each occurrence as distinct and select the one serving the
  intended consumer. If the intended consumer still does not identify one occurrence, ask before mutating.
- Never retry a failed `system` operation in `user` scope.
- Omit `--reveal` unless the user explicitly asks to see values. When revealing, query only the requested names and
  scopes and do not expose unrelated values.
- Prefer narrow queries, exact names, redacted JSON, and bounded output. Do not dump the complete environment when a
  targeted query answers the request.

## Mutations

A requested end state authorizes the envx mutations needed to reach it. Inspection and diagnosis requests remain
read-only.

- Inspect the exact target before a mutation.
- For commands with a preview mode, inspect the preview and then apply the same target and options when it matches the
  requested end state.
- For commands without preview, mutate only the inspected exact name and scope.
- Verify the resulting names, scopes, or PATH structure with a read-only command. Redacted verification proves presence
  and scope, not secret-value equality.
- Preserve and report every backup ID printed by envx.
