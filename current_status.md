# Current Status

Last updated: 2026-05-11

This file is a one-page snapshot of where the codebase sits. The full
narrative lives in [`docs/RefactorPlan.md`](docs/RefactorPlan.md).

## Where we are

The Phase 2 refactor (collapse 5 concepts down to 4 primitives) is
**effectively done**. The sidebar is `Home · Requests · Prompts ·
Scenarios · Engagements · Settings`. Targets, the Workbench view, and
the Engagement Wizard are gone from the UI and (mostly) from the
backend.

Matrix Scenarios — N Requests × a library subset, fired as a Cartesian
product, with auth-chain prerequisites resolved from
`Request.response.bind` — are the only Scenario mode. The legacy
step-based execution path has been retired in the runner, storage
schema, command layer, and UI.

## What works end-to-end today

- **Requests** view: full CRUD, structured + raw body editor, bearer
  token storage in the OS keychain (env-var-first precedence — see
  [`docs/Architecture.md`](docs/Architecture.md)), per-row Test
  Request button.
- **Prompts / Library** view: full CRUD. Name + auto-derived id +
  category + severity + OWASP ref + free-form tags. Bundled starter
  library seeded on first launch.
- **Scenarios** view: matrix editor (Request multi-select, OWASP chips,
  category chips, shared-session toggle, live prompt counter).
- **Engagements** view: list, open, per-row delete (refuses while a run
  is active). Run table with rerun, stop, delete, analyze, export-MD /
  export-PDF buttons. Engagement detail header shows the source
  Scenario name (resolved from `RunHeader.scenario_id`).
- **Home** view: recent engagements, "Run a Scenario" modal picker,
  "Open Scenarios" CTA.
- **Settings**: General · Logging · Analyz0r (with Prompt / Local Judge /
  Hosted Judge sub-tabs). Hosted-Judge mode stores the API key in the
  OS keychain.
- **Analyzer**: opt-in install flow that fetches a per-OS bundle, both
  local-judge (llama-cpp-2 + GGUF) and hosted-judge (Azure OpenAI)
  paths work end-to-end.

## Test status

`cargo test --workspace`: **162 passing**, zero warnings, zero failures.

UI is not under automated test yet — the Playwright spec for the
auth-chain matrix flow is the only meaningful piece still tracked in
the work-list.

## What's still open

See the work-list at the bottom of
[`docs/RefactorPlan.md`](docs/RefactorPlan.md). One meaningful item:

- **End-to-end Playwright spec** for the auth-chain matrix flow. Closes
  the Phase 2 Definition of Done.

Smaller polish items live in [`docs/ToDo.md`](docs/ToDo.md).
