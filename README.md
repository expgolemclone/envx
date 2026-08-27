# envx

`envx` is a scope-aware environment variable manager written in Rust. It keeps process, user, and system variables separate, requires an explicit scope for mutations, and redacts values in CLI output unless disclosure is requested.

## Safety model

- `process`, `user`, and `system` entries with the same name remain distinct.
- Mutating commands require `--scope process`, `--scope user`, or `--scope system`.
- Read commands display `[REDACTED]` by default. Add `--reveal` only when the terminal output is safe.
- Names containing `=` are never printed in full because malformed names can contain embedded secrets.
- Persistent Windows mutations create a DPAPI CurrentUser encrypted backup before changing the registry.
- PATH mutations are dry runs unless `--apply` is present.
- PATH checks expand Windows forms such as `%USERPROFILE%` for comparison and existence checks while preserving the original stored text.
- Errors opening the requested scope are returned directly. A failed system mutation never falls back to user scope.

User variables are stored in `HKCU\Environment`. System variables are stored in `HKLM\System\CurrentControlSet\Control\Session Manager\Environment` and normally require an elevated terminal. Process scope only changes the running `envx` process.

## Install

```bash
git clone https://github.com/expgolemclone/envx.git
cd envx
cargo install --path crates/envx
```

The published Cargo package is named `envex`.

```bash
cargo install envex
```

## Core usage

List all scopes without exposing values.

```bash
envx list
envx list --scope user
envx get PATH --scope user
envx get PATH --scope user --reveal
```

Set and delete in one exact scope.

```bash
envx set --scope user MY_VAR "value"
envx delete --scope user MY_VAR
envx delete --scope user "TEMP_*" --force
```

Export and import require a scope so formats keyed only by variable name cannot collapse equal names from different scopes.

```bash
envx export --scope user variables.json --format json
envx import --scope user variables.json --overwrite
envx import --scope process .env --format env --dry-run
```

## Diagnose and repair malformed names

`doctor` reports counts and safe name references. `repair invalid-names` is a dry run until `--apply` is added.

```bash
envx doctor --scope user
envx repair invalid-names --scope user
envx repair invalid-names --scope user --apply
```

The repair operation is batched. If a deletion fails, already deleted entries are restored before the error is returned.

## PATH management

Every PATH command targets one exact scope. Read and check commands do not need `--apply`.

```bash
envx path --scope user list --numbered --check
envx path --scope user check --verbose
envx path --scope user dedupe --keep-first
envx path --scope user dedupe --keep-first --apply
envx path --scope user clean --dedupe
envx path --scope user clean --dedupe --apply
envx path --scope user add "C:\Tools"
envx path --scope user add "C:\Tools" --apply
```

Use an elevated terminal for a system PATH mutation.

```bash
envx path --scope system dedupe --keep-first --apply
```

## Encrypted backups

Backups are stored under the current user's local application data directory in `envx\backups`. Their payloads are encrypted with Windows DPAPI and can only be decrypted in the corresponding Windows user context.

```bash
envx backup list
envx backup restore <BACKUP_ID> --apply
```

A restore creates another encrypted backup of the state it is about to replace, so the restore itself can be undone.

## Profiles, projects, watch, and monitoring

Commands that apply values also require a scope.

```bash
envx profile apply development --scope process
envx profile show development
envx profile show development --reveal
envx project apply --scope process
envx watch .env --scope user
envx monitor --scope user
envx monitor --scope user --reveal
```

Snapshot output is redacted unless `--reveal` is supplied. Snapshots preserve variables that share a name across different scopes.

```bash
envx snapshot create before-change
envx snapshot show before-change
envx snapshot diff before-change after-change --reveal
```

Run `envx --help` or `envx <COMMAND> --help` for the complete command surface.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

## License

MIT. See [LICENSE](LICENSE).
