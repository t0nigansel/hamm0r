# analyzer/ — manifest and publish recipe

This folder is the public face of the analyz0r install pipeline. hamm0r
clients fetch [`manifest.json`](manifest.json) from
`https://raw.githubusercontent.com/t0nigansel/hamm0r/main/analyzer/manifest.json`
on every install attempt; the manifest in turn points at bundle zips
hosted as GitHub Release assets.

## Schema

`manifest.json` is parsed by `AnalyzerManifest` in
[`crates/hamm0r/src/commands/analyzer_setup/manifest.rs`](../crates/hamm0r/src/commands/analyzer_setup/manifest.rs).
The current shape:

```json
{
  "version": 1,
  "generated_at": "2026-05-05T00:00:00Z",
  "minimum_hamm0r_version": "0.1.0",
  "variants": [
    {
      "id": "qwen2.5-3b-q4-windows",
      "label": "Qwen2.5 3B Q4_K_M (Windows x86-64)",
      "os": "windows",
      "arch": "x86_64",
      "hardware": "x86_64_avx2",
      "recommended": true,
      "model_id": "qwen2.5-3b-q4",
      "bundle": {
        "url": "https://github.com/t0nigansel/hamm0r/releases/download/analyzer-v0.1.0/analyz0r-windows-x86_64.zip",
        "sha256": "<lowercase hex sha256 of the zip>",
        "size_bytes": 0
      }
    }
  ]
}
```

Field rules:

- `version` must equal `1` (current schema). Bumping requires a Rust
  reader change in `manifest.rs`.
- `os` ∈ `windows` | `macos` | `linux`. Used to pick the entrypoint name
  (`bin/analyz0r.exe` on Windows, `bin/analyz0r` elsewhere).
- `hardware` ∈ `apple_silicon` | `x86_64_avx2` | `generic`. Drives the
  recommended-variant pick in the UI based on detected host hardware.
- `bundle.sha256` is lowercase hex without prefix; the installer
  compares with `format!("{:x}", hasher.finalize())`, so any other
  format will fail SHA verification.
- `minimum_hamm0r_version` is checked numerically (`MAJOR.MINOR.PATCH`)
  against the running app's `CARGO_PKG_VERSION` before any download
  starts.

## Bundle layout (zip contents)

Each bundle zip is extracted directly into `~/hamm0r/analyzer/`. The
expected top-level entries:

```
bin/
  analyz0r          (or analyz0r.exe on Windows — must match the OS tag)
runtime/            (optional: dynamic libs the analyzer binary needs)
models/
  <one .gguf>       (any filename; first .gguf is recorded in install.json)
```

Build the analyzor-cli for each target with `cargo build --release -p analyzor-cli`,
then assemble the layout above and zip it.

## Publish recipe (per release)

1. Build bundles for each `(os, arch)` you want to ship.
2. For each zip, compute lowercase hex SHA256 and file size in bytes.
3. Cut a GitHub release: `gh release create analyzer-v0.1.0 ./analyz0r-*.zip`
4. Edit `analyzer/manifest.json` with the new release-asset URLs, SHAs,
   and sizes. Update `generated_at` to the current UTC timestamp.
5. `git push` to `main`. The push is the cutover — no client tries to
   download a bundle until the manifest references its SHA.

To roll back, edit the manifest back to the previous SHAs and push.
Old zips do not need to be deleted from the release.

## Current status

The manifest currently lists **no variants**. Until real bundles are
published, the Settings install flow will show "No variants available"
and the install button will remain disabled. This is intentional — no
client attempts a download until the manifest references a bundle SHA.
