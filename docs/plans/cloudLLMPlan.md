# cloudLLMPlan.md — Hosted Judge for analyz0r

> **Status (2026-05-07):** planning only. No code changes are implied by this
> document. This plan intentionally changes the current product stance that the
> analyzer never uses cloud inference. If adopted, `ProductVision.md`,
> `Architecture.md`, `Stack.md`, and `Datamodel.md` must be updated in the same
> implementation series.

## Goal

Allow the user to choose exactly one global analyz0r judge in **Settings**:

- **Local Judge** — today's local analyz0r path
- **Hosted Judge** — a cloud-hosted LLM used for all analyzer activities

When **Hosted Judge** is configured, analyz0r uses that hosted model for:

- single-result judging
- full-run analysis
- report-driving verdict generation

The judge choice is global. There is no per-run, per-scenario, or per-request
override in v1.

---

## User decisions already locked in

These are requirements, not open questions:

1. Cloud use is **optional** and analyzer-only.
2. Local remains the default/default-install path.
3. The design should be **provider-agnostic**.
4. The user does not yet know whether the target cloud API is
   `chat/completions` or `responses`.
5. If Hosted Judge is selected, it is used for **all** analyzer work.
6. Auth is **API key** only in v1.
7. Secrets live in the **OS keychain**, not YAML.
8. Settings should ask only for the fields that are actually required.
9. Known Azure data today:
   - endpoint
   - deployment
   - API key
10. No extra warning banner is required for cloud use.
11. The hosted request may include:
   - attack prompt
   - extracted target response
   - run metadata
   - analyz0r judge prompt
12. The judge choice is **global only**.
13. Verdicts and reports must mention **which judge/model** was used.
14. Hosted mode should be fully hosted; do not silently fall back to local.
15. Misconfiguration or API failure should **fail hard** with a clear error.
16. Hosted mode must work **without a local analyzer model install**.
17. Usage/cost controls are required.
18. No per-environment model selection in v1.
19. No proxy/network controls in v1.
20. UI name: **Hosted Judge**.
21. Product/spec docs should be updated to reflect the new stance.
22. The implementation should use a **clean provider abstraction**.
23. No special compliance constraints are known today.
24. HTML report generation remains local.
25. First milestone: configure the global judge in Settings and use it
    everywhere.

---

## Product change

This feature is not a small implementation tweak. It changes an explicit
product belief:

- current vision: analyz0r is local and "never phones home"
- new vision: hamm0r stays local-first in core, but the analyzer may use either
  a local judge or an explicitly configured hosted judge

That means the product promise should become:

- **core stays air-gapped**
- **analyzer defaults to local**
- **hosted judging is an explicit opt-in setting**

The product must still remain useful without any cloud account, and core must
still work with no analyzer installed.

---

## Scope

### In scope

- A global **Judge Mode** setting with:
  - `Local Judge`
  - `Hosted Judge`
- Hosted judge configuration in Settings
- OS-keychain-backed secret storage for the hosted API key
- Provider abstraction in analyzer/runtime orchestration
- Using the selected judge for:
  - `judge_result`
  - `judge_run`
  - `start_analysis`
  - report-generating analysis flows
- Clear attribution of judge provider/model in verdicts and reports
- Hard-fail error behavior when hosted config is broken
- Basic usage/cost controls

### Out of scope for v1

- Per-run judge override
- Per-scenario judge override
- Per-request judge override
- Multiple hosted providers exposed in the UI on day one
- Proxy settings
- Azure AD / Entra auth
- Streaming partial verdicts from the hosted provider
- Background model benchmarking or auto-selection
- Moving report rendering itself to the cloud

---

## UX proposal

### Settings → Analyzer

Replace the current single local-analyzer mental model with a global judge
selector.

Recommended layout:

1. **Judge Mode**
   - `Local Judge`
   - `Hosted Judge`

2. If `Local Judge`:
   - keep today's local analyzer install/status UI
   - keep model/runtime install behavior
   - keep judge prompt template UI

3. If `Hosted Judge`:
   - show hosted provider configuration
   - hide local-install actions that are irrelevant
   - keep the same judge prompt template UI

### Hosted Judge fields

Because the user wants only required fields, the first hosted provider should
surface a minimal set:

- `Provider`
- `Endpoint`
- `Deployment or Model`
- `API Key`
- `API Style` only if required after provider selection
- usage/cost controls

For Azure-first v1, the likely required fields are:

- `Provider: Azure OpenAI`
- `Endpoint`
- `Deployment`
- `API Key`
- `API Version` if the selected API style requires it

### Status display

Hosted Judge status should be explicit:

- `configured`
- `missing required field`
- `invalid secret`
- `connection failed`
- `provider rejected request`

No silent fallback to local. If Hosted Judge is selected and unusable, analysis
actions should fail with a clear user-facing error.

---

## Architecture direction

### Boundary to preserve

This feature must preserve the existing big boundary:

- **core** still does not depend on local model runtime
- **core** still writes/reads run artifacts
- **analyzer path** still performs judging/report generation

What changes is the analyzer execution strategy:

- local mode: current local analyz0r path
- hosted mode: analyzer pipeline calls a hosted inference adapter instead of a
  local runtime/model

### Recommended abstraction

Introduce a single conceptual interface inside analyzer-facing code:

```text
JudgeBackend
  - judge_one(input) -> verdict
  - judge_many(inputs) -> stream/progress + verdicts
  - describe() -> judge identity metadata
```

Implementations:

- `LocalJudgeBackend`
- `HostedJudgeBackend`

Hosted provider-specific behavior then lives behind a second layer:

```text
HostedProviderAdapter
  - build_request(...)
  - execute(...)
  - parse_response(...)
  - provider_identity(...)
```

First implementation:

- `AzureOpenAiAdapter`

The UI can remain provider-agnostic even if only Azure is implemented first.

### Why this shape

- avoids baking Azure assumptions into verdict/report code
- preserves one global "selected judge" mental model
- keeps the prompt-building and verdict parsing logic shared
- gives a clean path to other hosted providers later

---

## Runtime model

### Local Judge mode

Use today's local analyzer/runtime path.

Requirements unchanged:

- local install metadata remains the source of truth
- local model install remains optional
- no hosted call is attempted

### Hosted Judge mode

Do **not** require a local GGUF/model install.

Two acceptable implementation shapes:

1. **Reuse the existing `analyz0r` subprocess**
   - it reads hosted config
   - it uses HTTP instead of local runtime
   - it still writes verdict JSONL and reports

2. **Route hosted analysis directly from core to analyzer library code**
   - only if this does not blur the core/analyzer boundary too much

Recommendation:

- keep one analyzer execution surface and let it select local vs hosted
  internally

That keeps the UI/backend command layer simpler and preserves one analysis
pipeline.

---

## Settings and config model

### `config.yaml`

Add only non-secret hosted judge configuration here.

Recommended shape:

```yaml
version: 1
analyzer:
  enabled: true
  judge_mode: local            # local | hosted
  model_variant: auto
  judge_prompt_template: |
    ...
  hosted_judge:
    provider: azure_openai
    endpoint: https://example.openai.azure.com
    deployment: gpt-5.2-chat
    api_style: auto            # auto | chat_completions | responses
    api_version: 2024-xx-xx    # optional until required by provider
    secret_ref: hosted-judge-default
    max_input_chars: 24000
    max_output_tokens: 1200
    request_timeout_seconds: 60
    max_retries: 1
```

Notes:

- `secret_ref` is a keychain reference name, not the secret value
- `judge_mode` is the global switch
- `hosted_judge` is ignored when `judge_mode == local`
- only required fields should be shown in UI; optional fields may have safe
  defaults

### Secret storage

Use the same OS-keychain strategy as target auth:

- service: `hamm0r`
- account/ref: a stable hosted-judge secret reference

The plaintext API key should:

- cross the JS→Rust bridge only on save
- never be written to YAML
- never be returned to the UI after save
- never be logged

---

## Provider abstraction details

### Provider enum

Recommended first shape:

```text
HostedProvider
  - AzureOpenAi
```

Even with one real implementation, code should be written so adding another
provider does not require rewriting Settings, verdict attribution, or the
pipeline contract.

### API style

Because the user does not yet know whether the company endpoint expects
`chat/completions` or `responses`, v1 should model API style explicitly:

- `auto`
- `chat_completions`
- `responses`

Recommendation:

- UI default: `auto`
- Azure adapter may initially implement one style, but the config model should
  not assume that forever

---

## Analyzer input and output

### What is sent to the hosted judge

Per user decision, the hosted judge may receive:

- analyz0r judge prompt/template output
- attack prompt
- extracted target response
- attempt/run metadata used for judging

The hosted provider call should not send:

- raw secrets
- redacted auth headers restored to plaintext
- unrelated engagement files

### What is recorded locally

Verdicts and reports should record judge identity clearly.

Recommended verdict header extension:

```json
{
  "type": "header",
  "run_id": "run-001",
  "model": "azure_openai:gpt-5.2-chat",
  "judge_mode": "hosted",
  "provider": "azure_openai",
  "deployment": "gpt-5.2-chat",
  "analyzer_version": "0.2.0",
  "started_at": "2026-05-07T10:00:00Z"
}
```

Report output should also show:

- Judge mode: local or hosted
- Provider/model/deployment

That gives the user traceability when comparing runs.

---

## Error handling

Hosted Judge mode should fail hard and clearly.

### Fail examples

- missing endpoint
- missing deployment/model
- missing API key in keychain
- unsupported provider config
- timeout talking to provider
- 401/403 from provider
- invalid response schema from provider

### UX expectation

Return actionable errors such as:

- `Hosted Judge is selected, but no API key is stored.`
- `Azure OpenAI endpoint rejected the request with 401 Unauthorized.`
- `Hosted Judge is configured for responses API, but this provider adapter does not support it yet.`

Do not:

- silently switch to local
- silently downgrade to heuristic-only judging

---

## Usage and cost controls

The user explicitly wants controls. Keep them small and global.

Recommended v1 controls:

- `max_input_chars`
- `max_output_tokens`
- `request_timeout_seconds`
- `max_retries`
- optional `max_attempts_per_batch` if batching becomes relevant

Recommended v1 non-goals:

- exact currency budgeting
- provider-side spend dashboards
- per-run cost ceiling estimation if tokenization is too provider-specific

If feasible, the UI may later show lightweight diagnostics such as:

- last request latency
- total judge requests in the run
- model/deployment used

---

## Files and modules likely to change

### Docs

- `docs/ProductVision.md`
- `docs/Architecture.md`
- `docs/Stack.md`
- `docs/Datamodel.md`

### Storage / types

- hosted judge config types
- keychain secret helpers for hosted API key
- settings DTOs

### Analyzer / CLI / orchestration

- analyzer backend selection
- hosted provider adapter layer
- Azure adapter
- subprocess/config handoff as needed

### UI

- Settings analyzer section
- hosted judge secret save/status UX
- analyzer status labels mentioning local vs hosted judge

---

## Required spec changes

### `ProductVision.md`

Must change these ideas:

- "no cloud account" can no longer be an absolute statement for the analyzer
- analyz0r can no longer be described as always local and never phoning home

Recommended replacement stance:

- hamm0r core remains local-first and fully useful without cloud
- analyz0r defaults to local
- hosted judging is an explicit opt-in for users who want it

### `Architecture.md`

Must change:

- analyzer "must not make cloud calls"

Recommended replacement:

- runner must not make cloud calls
- core default workflow remains cloud-free
- analyzer may use a hosted provider only when `Hosted Judge` is explicitly
  selected in Settings

### `Stack.md`

Must change:

- "Do not use any cloud inference API"

Recommended replacement:

- do not use cloud inference in core or runner
- analyzer may use a hosted inference adapter in Hosted Judge mode

### `Datamodel.md`

Must add:

- `judge_mode`
- hosted judge config block
- hosted secret reference semantics
- verdict/report identity fields if expanded

---

## Recommended delivery order

### Phase 1 — spec realignment

- update `ProductVision.md`
- update `Architecture.md`
- update `Stack.md`
- update `Datamodel.md`

This feature should not be implemented while the docs still say the opposite.

### Phase 2 — config + secret plumbing

- add `judge_mode`
- add hosted judge config schema
- add keychain secret save/read path
- add Settings UI

### Phase 3 — analyzer backend abstraction

- introduce `JudgeBackend`
- keep local path working
- add hosted backend selection

### Phase 4 — Azure provider implementation

- implement `AzureOpenAiAdapter`
- support the chosen API style first
- return normalized analyzer outputs

### Phase 5 — end-to-end analyzer flows

- single result judge
- full run analysis
- report generation flow using hosted verdicts

### Phase 6 — hardening

- failure messages
- hosted status display
- usage/cost controls
- documentation polish

---

## Testing strategy

### Unit tests

- hosted config round-trip
- secret reference handling
- provider request builder
- provider response parser
- judge backend selection
- hosted error mapping

### Integration tests

- local mode still works unchanged
- hosted mode with missing key fails clearly
- hosted mode with fake provider returns verdicts correctly
- report generation uses hosted verdict metadata
- no local model install required in hosted mode

### Manual checks

1. Fresh install, no local analyzer model:
   - select `Hosted Judge`
   - configure endpoint/deployment/key
   - analyze one result successfully
2. Full run analysis in hosted mode:
   - verdict file written
   - report generated
   - report shows hosted judge identity
3. Broken config:
   - clear failure
   - no silent local fallback
4. Switch back to local:
   - hosted config preserved
   - local flow works again

---

## Risks

### 1. This weakens the old "never phones home" story

That is acceptable only because the user wants it and because the opt-in
boundary stays explicit.

### 2. Provider abstraction can become fake abstraction

If Azure-specific fields leak everywhere, the abstraction will be cosmetic.
Keep Azure details inside the adapter and config mapping layers.

### 3. Hosted APIs change faster than local runtime code

The adapter must treat API shape as a compatibility surface with clear
errors and tests.

### 4. Cost surprises

Even basic limits are better than none. V1 should ship with conservative
defaults.

### 5. Boundary blur between core and analyzer

Avoid scattering hosted inference calls across the Tauri command layer.
Keep one analyzer path and one judge abstraction.

---

## Success criteria

This feature is done when:

1. The user can open Settings and choose `Local Judge` or `Hosted Judge`.
2. Hosted Judge can be configured with the minimum required fields.
3. The hosted API key is stored in the OS keychain, not YAML.
4. With Hosted Judge selected, analysis works without any local model install.
5. The same global judge choice is used for all analyzer activities.
6. Verdicts and reports clearly record which judge/model/provider was used.
7. Hosted misconfiguration fails hard with a clear error.
8. Local mode still works and remains the default experience.

---

## Recommendation

Build this as a **single global judge system** with a clean backend/provider
split:

- one user-facing selector in Settings
- one analyzer pipeline
- two backend implementations: local and hosted
- one Azure implementation first

That gives the shortest path to your first milestone without painting the code
into an Azure-only corner.
