# File import and export

Use this route for scoped transfer between envx and `.env`, JSON, YAML, text, PowerShell, or shell files. Select the
format from a supported extension, or pass `--format` when the extension is absent or ambiguous.

## Import

Always run a redacted dry run first:

```powershell
envx import FILE --scope user --dry-run
envx import FILE --scope user
```

Use repeated `--vars PATTERN` options to limit the import and `--prefix PREFIX` only when requested. If the first dry
run reports conflicts and replacement is required, repeat the dry run with `--overwrite`. Apply only after that
preview succeeds, using the same file, scope, format, filters, prefix, and overwrite choice, then verify the imported
names without revealing values.

## Export

Inspect the scoped selection before export because exported files contain plaintext values:

```powershell
envx export FILE --scope user --vars NAME
```

Omit `--vars` only when the request intentionally covers the entire selected scope. Do not add `--force` unless
replacing the exact destination is required. Report the destination and exported count without printing file contents.
