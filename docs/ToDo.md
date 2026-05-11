# TODO

Polish-level items the user asked for. Bigger refactor / Phase-2
remaining work lives in [`RefactorPlan.md`](RefactorPlan.md) and the
single Phase 2 closeout (e2e Playwright spec) is tracked there.

## Current open items

- [ ] **App icon** — current icon quality is not good enough.
- [ ] **Alternative color themes** — Spirit Testing and TestSolutions
  brand variants alongside the default theme.

## Done (kept for reference, prune when convenient)

- [x] Fix the Settings button (couldn't open/respond) — root cause was
  the `openRequestAuthTokenModal` ReferenceError aborting DCL setup
  before the Settings click handler registered. Fixed alongside the
  bearer-token UI restore.
- [x] Add an easy way to create and add prompts — full CRUD shipped in
  the Library view, including name-with-auto-slug, OWASP picker, and
  free-form tags.
- [x] Fix the Workbench response view / Workbench sending — the
  Workbench view itself was retired in Phase 2F of the refactor.
  Request firing now lives behind the Requests view's "Test request"
  button.
- [x] Wizard scenario-selection bug — the Engagement Wizard was retired
  in Phase 2 of the refactor; "Run a Scenario" replaced it.
