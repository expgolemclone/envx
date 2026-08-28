# Ordinary variables

Use this route for targeted inspection, creation, replacement, or deletion of ordinary environment variables.

## Inspect

Use exact redacted queries whenever the name is known:

```powershell
envx get NAME --format json
envx get NAME --scope user --format json
```

Use a scoped list only for discovery. Add `--query TEXT`, `--names-only`, or `--limit N` to keep the result narrow:

```powershell
envx list --scope user --format json --query TEXT --limit N
envx list --scope user --names-only --query TEXT --limit N
```

Add `--reveal` only under the explicit reveal policy in `SKILL.md`.

## Set

Inspect the exact name across scopes first, choose one scope using the shared rules, then set it:

```powershell
envx set --scope user NAME VALUE
```

Setting replaces an existing value in that exact scope. A persistent Windows mutation prints the encrypted backup ID.
Verify the exact name and scope afterward without revealing its value unless explicitly requested.

## Delete

Resolve and inspect every match before deleting. Use an exact name by default and `--force` to avoid an interactive
prompt after the target is known:

```powershell
envx delete --scope user NAME --force
```

Use wildcard deletion only when the user requested a matching set and the preceding redacted query established all
affected names. Never change scope after an access failure.
