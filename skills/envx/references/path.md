# PATH variables

Use this route for `PATH` or another path-list variable selected with `--var NAME`. Every command requires one exact
scope.

## Inspect

List and validate before changing the path list:

```powershell
envx path --scope user list --numbered --check
envx path --scope user check --verbose
```

Resolve a move from the numbered list. Preserve the stored path text even when envx shows an expanded path during
validation.

## Preview and apply

PATH mutations are previews until `--apply` is added. Run the applicable command without `--apply`, inspect the proposed
result, and repeat the same command with `--apply`:

```powershell
envx path --scope user add 'C:\Tools'
envx path --scope user remove 'C:\Tools' --all
envx path --scope user move FROM TO
envx path --scope user dedupe --keep-first
envx path --scope user clean --dedupe
```

Use `--first` only when ordering requires the new directory to win command resolution. Use `--create` only when creating
a missing directory is part of the requested end state. Prefer `--keep-first` for duplicate cleanup because earlier PATH
entries control command resolution.

After applying, repeat the numbered list and check. Report the backup ID from a persistent mutation. A failed
system-scope operation is an error, not permission to change user PATH instead.
