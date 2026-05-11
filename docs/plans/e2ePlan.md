# e2e Test System — Implementation Plan

Goal: a self-maintaining Playwright test suite where the test manager only edits
`req.md` and `test.md`. Everything else — test generation, repair, and execution —
happens automatically.

---

## Architecture recap

```text
req.md / test.md  →  MCP server  →  Claude agent  →  tests/e2e/specs/
                                                           ↑
                             git hook / CI ────────────────┘ (auto-trigger)
```

---

## Checklist

### Phase 1 — Smarter agent (no new infrastructure needed)

- [x] **Add `get_ui_diff` tool to MCP server**
  Returns `git diff HEAD~1 -- ui/` so the agent knows exactly what changed in the
  UI layer and can update only the affected specs instead of re-reading everything.
  File: `scripts/mcp-testmanager/server.js`

- [x] **Add `get_tauri_commands` tool to MCP server**
  Parses `ui/js/api.js` and returns every `invoke()` call as a structured list.
  The agent can self-discover which commands exist and verify that each one has
  test coverage — without relying on `req.md` being kept up to date.
  File: `scripts/mcp-testmanager/server.js`

- [x] **Update `/sync-tests` command prompt**
  Teach the agent to call `get_ui_diff()` first. If there is a diff, target only
  the affected specs. If there is no diff (manual run), fall back to full coverage
  check. Also tell it to use `get_tauri_commands()` to cross-check coverage.
  File: `.claude/commands/sync-tests.md`

---

### Phase 2 — CI safety net (runs tests on every PR)

- [x] **Add GitHub Actions workflow**
  Runs `npm ci && npx playwright install chromium && npm test` in `tests/e2e/`
  on every push and pull request. Uploads the Playwright HTML report as an artifact.
  File: `.github/workflows/e2e.yml`

- [x] **Add `get_test_report` tool to MCP server**
  Reads the last Playwright JSON report (`playwright-report/results.json`) and
  returns a structured failure summary. The repair agent uses this instead of
  re-running the full suite to understand what broke.
  File: `scripts/mcp-testmanager/server.js`
  Prereq: configure `playwright.config.js` to also emit a JSON reporter. ✓

---

### Phase 3 — Auto-trigger on UI changes (git hook)

- [x] **Add post-commit hook**
  Detects changes to `ui/`, `req.md`, or `test.md` in the latest commit via
  `git diff-tree --no-commit-id -r --name-only HEAD`. If any match, spawns the
  sync agent non-interactively.
  File: `.githooks/post-commit`

- [x] **Document hook setup in CLAUDE.md**
  One-line setup: `git config core.hooksPath .githooks`
  Without this, the hook file exists but git doesn't run it.

---

### Phase 4 — Self-healing (repair loop on CI failure)

- [x] **Add repair step to CI workflow**
  ~~Separate `repair` job that runs only when `e2e` fails on main.~~
  **Not implemented in CI** — storing `ANTHROPIC_API_KEY` as a GitHub Actions secret
  is a security risk. Auto-repair is available locally via `/repair-tests` only.

- [x] **Add `repair-tests` slash command**
  A focused variant of `/sync-tests` that only fixes broken tests — it does not
  add new coverage. Reads the failure report, reads the affected spec files, reads
  the current UI source, and writes targeted fixes.
  File: `.claude/commands/repair-tests.md`

---

## Dependency order

```text
Phase 1  →  Phase 2  →  Phase 3  →  Phase 4
(smarter)   (CI net)    (auto-     (self-
                         trigger)   healing)
```

Phase 1 can be done without any infrastructure. Each phase adds one layer of
automation on top of the previous one. Stop at whichever phase gives enough
autonomy for the current workflow.
