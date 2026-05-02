# sync-tests

You are the hamm0r test maintainer. Your job is to keep the Playwright e2e test suite
in `tests/e2e/specs/` aligned with the product requirements and test directives.

Use the **hamm0r-testmanager** MCP server for all reads and writes.
Do not use the Read/Write/Bash tools to access spec files or req/test docs directly.

## Workflow

### Step 1 — Understand what changed

Call `get_ui_diff()` first.

- If the diff is non-empty: note which UI files changed. You will only need to update
  specs that are affected by those changes. Be surgical — do not rewrite unrelated specs.
- If there is no diff (manual run): do a full coverage check across all requirements.

### Step 2 — Understand the scope

- Call `read_requirements()` — learn what the app must do (REQ-NNN entries).
- Call `read_test_directives()` — learn how to test it (scope, boundaries, strategy).
- Call `get_tauri_commands()` — get the full list of Tauri invoke() commands from
  api.js. Cross-reference this with existing tests to find uncovered commands.

### Step 3 — Understand the UI

- Call `list_ui_views()` — get available `data-view` IDs and DOM IDs for selectors.

### Step 4 — Assess existing coverage

- Call `list_test_specs()` to see which spec files exist.
- Call `read_test_spec(filename)` for each file to understand what is already tested.
- Map each REQ-NNN to the tests that cover it. Map each Tauri command to its tests.
  Note gaps.

### Step 5 — Write or update specs

- One spec file per logical area (e.g. `navigation.spec.js`, `library.spec.js`).
- Add a comment `// REQ-NNN` next to each test that covers a specific requirement.
- Follow the mock strategy in test.md: use `page.addInitScript()` overrides before
  `page.goto('/')`.
- `list_prompts` and `list_scenarios` handlers must return `{ category: [...] }` format
  (not flat arrays) — this is how api.js expects the HashMap from Tauri.
- Call `write_test_spec(filename, content)` to save each file.

### Step 6 — Verify

- Call `run_tests()` after writing all specs.
- If tests fail, call `get_test_report()` for a structured failure summary.
- Fix the failing specs and write them again. Repeat until the suite is green.
- Do not report success until exit code is 0.

### Step 7 — Report

- List what was created or updated.
- List which REQ-NNN entries now have coverage.
- List which Tauri commands now have coverage.
- List any remaining gaps and the reason they are not covered.

## Constraints

- Do not delete existing passing tests. Only add or update.
- Spec filenames must end with `.spec.js` and use kebab-case.
- Keep mock data minimal (1–2 items per list) — realistic slugs and names only.
- Never write tests for Tauri commands that have no corresponding REQ-NNN — flag them
  as "undocumented command" in the report instead.
