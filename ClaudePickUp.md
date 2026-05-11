# ClaudePickUp — analyz0r-Umbau, nächste Session

> **⚠️ Historisches Session-Handoff (May 2026).** Dieser Text stammt aus
> der aktiven Umbauphase von `docs/analyzorPlan.md`. Die
> Analyzer-Architektur (Subprozess-Bundle, Install-Statemaschine,
> llama-cpp-2 + GGUF, plus Hosted-Judge gegen Azure OpenAI) ist
> inzwischen produktiv. Für den aktuellen Projektstand siehe
> [`current_status.md`](current_status.md),
> [`docs/RefactorPlan.md`](docs/RefactorPlan.md) und — für den analyz0r
> selbst — [`docs/Architecture.md`](docs/Architecture.md) § "The
> analyzer as a separable module". Diese Datei nur noch als historische
> Notiz behandeln; neue Claude-Sessions starten an `current_status.md`.

Diese Datei ist der Einstieg für die nächste Claude-Session. Lies sie zuerst,
dann `docs/analyzorPlan.md` (der Master-Plan), dann diese Datei nochmal.

## Wo wir gerade stehen

Aktiv ist die Umsetzung von `docs/analyzorPlan.md`. Der Plan hat 6 Phasen.
**Phasen 1, 2 und 3 sind komplett**, acht Commits seit `ba7e5565`:

Phase 1:

- `7b51085b` — chore: silence pre-existing clippy errors from rust 1.95 lints
- `c77498da` — refactor(analyzer): extract judge + report orchestration into pipeline module
- `c37a3933` — feat(analyzor-cli): standalone judge + report CLI wrapping pipeline
- `a0c5cfdc` — refactor(core): remove --features analyzer gate; route LLM via analyz0r subprocess

Phase 2:

- `dd3146b2` — feat(storage): add analyzer install metadata module
- `bebbdcba` — feat(analyzer-setup): bundle-shaped manifest schema
- `a27517a2` — feat(analyzer-setup): bundle install with extract + install.json

Phase 3:

- `82f195c7` — feat(analyzer-setup): install state machine

Was Phase 1 jetzt insgesamt liefert:

1. Workspace baut wieder ohne libclang auf Windows. `llama-cpp-2` ist optional
   hinter `analyzer/runtime`-Feature, das nur die `analyz0r`-Bundle-Builds
   einschalten.
2. Komplette Judge- und Report-Orchestrierung als synchrone, Tauri-freie
   Library in [crates/analyzer/src/pipeline.rs](crates/analyzer/src/pipeline.rs).
3. Standalone-Binary [crates/analyzor-cli/](crates/analyzor-cli/) (Bin-Name
   `analyz0r`) mit Subcommands `judge-result`, `judge-run`, `generate-report`.
   NDJSON auf stdout, Human-Logs auf stderr, Exit-Codes 0/2/3.
4. Tauri-Layer in [crates/hamm0r/src/commands/analysis.rs](crates/hamm0r/src/commands/analysis.rs)
   nutzt:
   - `analyzer::pipeline::*` direkt für Heuristik-Pfade (alle Judge-Commands)
   - Subprocess-Aufruf an `analyz0r judge-run` für LLM, mit NDJSON-Parsing
     und Tauri-Progress-Forwarding
   - Auto-Fallback auf Heuristik, wenn Binary oder Modell nicht installiert
5. `--features analyzer` ist weg. Core funktioniert per Default.
6. Binary-Discovery: `$HAMM0R_ANALYZOR_BIN` (Dev-Override) →
   `~/hamm0r/analyzer/bin/analyz0r[.exe]` (Production-Layout, von Phase 2
   bedient).
7. CI: `cargo test --workspace` = 82 passed,
   `cargo clippy --workspace --all-targets -- -D warnings` = clean,
   `cargo check -p analyzer` (ohne libclang) = OK.

Was Phase 2 zusätzlich liefert:

1. Neues Storage-Modul [crates/storage/src/analyzer_install.rs](crates/storage/src/analyzer_install.rs)
   für `~/hamm0r/analyzer/install.json` (Read/Write/Remove, Schema-Version 1,
   Schema-Reject bei unbekannter Version).
2. Manifest-Schema umgebaut: `model + runtime` weg, ein `bundle` (Artifact)
   plus `os` / `arch` / `model_id`. SHA-Verifikation läuft jetzt
   unconditionally — der TODO-Bypass ist weg, Bundled-Fallback hat
   `sha256 = "PLACEHOLDER"` (test-pinned), schlägt also bewusst beim Install fehl.
3. Bundle-Install-Pipeline in
   [crates/hamm0r/src/commands/analyzer_setup.rs](crates/hamm0r/src/commands/analyzer_setup.rs):
   Download → SHA-Verify → ZIP-Extract (in `spawn_blocking`, mit Zip-Slip-Schutz
   via `enclosed_name()`) → wipe altes Layout → atomarer `rename` ins Ziel →
   `install.json` als letztes (= "is installed"-Signal) → Cleanup `.staging/`.
4. `uninstall_analyzer` putzt `install.json` zuerst, dann `bin/`, `runtime/`,
   `models/`, `.staging/`. Engagement-Folder unangetastet.
5. `get_analyzer_status` liest `install.json` (statt `first_gguf()`); zwei neue
   Felder `variant_id` und `bundle_version` für die UI.
6. Neue Dep: `zip = "2"` (default-features off, nur `deflate`).
7. CI: `cargo test --workspace` = 98 passed, `cargo check -p analyzer` (ohne
   libclang) = OK.

Was Phase 3 zusätzlich liefert:

1. Privater Enum `InstallState` mit `not_installed | downloading | installed |
   broken_install | incompatible_version`. UI kriegt es als String im DTO
   (`install_state_string_values_are_stable`-Test pinned die Werte).
2. `install_state_on_disk()` — kombiniert install.json-Read + Layout-Check
   (Entrypoint da? Modell da?) und unterscheidet sauber zwischen
   broken_install und incompatible_version (zwei-Pass-Parse: erst Schema-
   Version proben, dann typed deserialize).
3. Neue Tauri-State `AnalyzerInstallTracker(Arc<Mutex<Option<String>>>)`
   in [crates/hamm0r/src/commands.rs](crates/hamm0r/src/commands.rs),
   registriert in [crates/hamm0r/src/main.rs](crates/hamm0r/src/main.rs).
   Set vor spawn, Clear nach Task — `downloading` überschreibt die On-Disk-
   Antwort (sonst flapped die UI während dem Install).
4. `download_and_install_analyzer` blockt einen zweiten parallelen Install mit
   klarer Fehlermeldung statt im Staging-Dir zu racen.
5. `AnalyzerStatus` neue Felder: `state`, `downloading_variant_id`, plus
   `installed: bool` für Back-Compat.
6. CI: `cargo test --workspace` = 105 passed.

## Nächster Schritt — Phase 4: UI-Verbindung an die State-Maschine

Phase 4 ist im Plan als "Core-zu-Analyzer-Ausführung" gelabelt. Vieles davon
ist schon in Phase 1, Schritt 3 (`a0c5cfdc`) erledigt — Subprocess-Wiring,
Auto-Fallback, etc. Was übrig bleibt ist die **Frontend-Anbindung** an die
in Phase 3 gebauten States.

**Konkret:**

1. [ui/js/api.js](ui/js/api.js) / [ui/js/app.js](ui/js/app.js) prüfen: 
   - `getAnalyzerStatus()` muss jetzt das neue Status-DTO konsumieren
     (`state`, `downloading_variant_id`, `bundle_version`, `variant_id` neu).
   - Verifizieren, dass das alte `installed`-Feld noch genauso behandelt wird
     (Back-Compat steht).
2. UI-Logik je nach State in [ui/js/app.js](ui/js/app.js):
   - `not_installed` → "Download & Install"-Button enabled, Judge-Buttons
     disabled.
   - `downloading` → Progress-Bar, Buttons disabled.
   - `installed` → Judge-Buttons enabled, "Uninstall"-Button sichtbar.
   - `broken_install` → "Repair"-Button (ruft dieselbe
     `download_and_install_analyzer`-Action) plus Hinweis-Text.
   - `incompatible_version` → "App-Update nötig"-Hinweis, optional
     "Uninstall"-Button um den incompatiblen Stand wegzuputzen.
3. Polling: aktuell wird Status vermutlich beim Modal-Open einmal geholt.
   Während `downloading` muss die UI alle ~1s pollen (oder besser: die
   `analyzer-download-progress`-Events verwenden, die schon emittiert
   werden), sonst sieht der User keinen Fortschritt.
4. Logger-Events: bei jedem State-Wechsel (außer downloading-progress) ein
   `console.log` / `logger.info`-Eintrag, damit Bug-Reports den Verlauf
   zeigen.
5. Tests: das e2e-Test-System
   ([scripts/mcp-testmanager](scripts/mcp-testmanager) +
   [tests/e2e/](tests/e2e/)) sollte Settings-Modal-Smoke abdecken — passt
   gut, sobald Phase 5 die Copy-Texte stabilisiert hat.

**Was NICHT in Phase 4:**

- Finale Settings-Modal-Texte und Visual-Design — das ist Phase 5
- Per-OS-Bundle-Build — das ist Phase 6

**Achtung beim Editieren von ui/**: Der User arbeitet aktuell parallel an
Auth-Acquisition / engagements / hamm0rWeb-Themen in `ui/index.html`,
`ui/js/api.js`, `ui/js/app.js`, `ui/style.css` (siehe Working-Tree-Status
unten). Vor dem Editieren erst diff anschauen und nur den Analyzer-Settings-
Bereich anfassen, sonst entsteht Merge-Stress.

## Danach (Reihenfolge laut Plan)

1. **Phase 5:** Settings-Modal-Copy + "Repair"-Button polish + klare
   Fehlermeldungen.
2. **Phase 6:** Per-OS-Bundles bauen (CI-Pipeline mit `analyzor-cli
   --features runtime`), Manifest publishen, echte SHA256-Werte setzen,
   `bundled_manifest()` retiren oder mit echten Hashes füllen.

## Wichtige Konventionen aus CLAUDE.md (nicht vergessen)

- Kleine Diffs, ein Commit = ein Gedanke.
- Tests sind Teil derselben Änderung.
- File-I/O läuft über `crates/storage/` (Invariante).
- Keine neuen Deps ohne Begründung. `clap` ist für die CLI gerechtfertigt
  (Standard-Rust-CLI-Parser, im Plan implizit angenommen).
- Vor jeder Änderung an Run-/Verdict-JSONL-Schema oder am Core/Analyzer-Boundary
  → erst fragen.
- Commit-Message-Stil siehe `git log` (chore/feat/refactor + ausführlicher Body).

## Offene Working-Tree-Änderungen (NICHT von dieser Arbeit)

Diese liegen ungetracked oder modifiziert im Tree und sollen in dieser Reihe
**nicht** mitgenommen werden — sie gehören zu anderen Tasks:

- `.claude/mcp.json`, `.claude/settings.local.json` — User-Setup
- `ui/index.html`, `ui/js/app.js`, `ui/style.css` — andere UI-Arbeit
- `docs/analyzorPlan.md` — der Plan selbst, noch nicht committed (kann separat
  als Doc-Commit geliefert werden, oder zusammen mit Phase 6 wenn alles steht)
- `prompts/mogelPrompts.yaml`, `tests/e2e/specs/` — andere Tasks

## Pingelig: Verifikation nach jedem Schritt

Vor jedem Commit:

```pwsh
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Auf Windows ohne libclang muss `cargo check -p analyzer` (ohne Features)
weiterhin OK sein. Das ist die zentrale Win-Build-Invariante aus Phase 1.
