# Auth Helper Implementation Plan

> **⚠️ Superseded.** This was the original plan for storing
> `auth_acquisition.http_login` inside Target YAML and exposing a
> "Fetch token" button on the Target editor. The HTTP-login pre-flight
> shipped in roughly this shape and then **changed model** in Phase 2 of
> [`RefactorPlan.md`](RefactorPlan.md):
>
> - Targets were retired as a top-level concept; their auth fields
>   were not preserved.
> - Login is now modeled as a **separate Request** whose response
>   declares `response.bind: bearer_token`. Other Requests reference
>   the bound value via `{{login_id.bearer_token}}` in their headers.
>   The runner's DAG resolver fires the login Request before the
>   target Request, automatically. See
>   [`Architecture.md`](Architecture.md) and
>   [`Datamodel.md`](Datamodel.md).
> - A storage migration (`storage::migrations::v2::synthesize_auth_chain_requests`)
>   converts the legacy `http_login` config into the new shape on
>   first startup after upgrade.
> - The bearer-token storage / OS-keychain mechanic still exists, but
>   the resolver checks the env var **first** now — see
>   `storage::secrets::resolve_token` and the rationale in
>   `Architecture.md`.
>
> Read the rest of this file only for historical context.

## Goal

Automate target authentication in `hamm0r` so a user can trigger token
acquisition from the GUI instead of manually logging into the target
system, copying a bearer token, and pasting it into `hamm0r`.

The design must remain:

- local-first
- target-agnostic
- compatible with future target systems
- safe with respect to secrets handling

This plan assumes the first concrete use case is the existing Profiler
login flow.

The current design decision for v1 is:

- HTTP-only login flow
- no Playwright helper in v1
- no extra runtime like Node.js required
- bearer token is extracted from the response of a login HTTP request

There is an existing Playwright login test at:

`C:\workspace\profiler-ng-qa\e2eAgents\tests\login.spec.ts`

---

## Problem statement

Current flow:

1. User logs into the target system manually
2. User extracts bearer token manually
3. User stores token in `hamm0r`
4. User can finally test

Problems:

- slow and repetitive
- error-prone
- fragile when tokens expire
- not scalable across multiple targets
- tightly bound to one target's current login UX if implemented naively

---

## Design principle

Do not build “Profiler login” into `hamm0r` as a one-off special case.

Build a generic local **HTTP auth acquisition** mechanism that lets
`hamm0r` perform a configurable login request and extract a bearer token
from the response.

Only if a future target cannot be handled via normal HTTP request/response
flows should an external browser helper be considered.

This keeps:

- `hamm0r` generic
- target-specific login logic outside the core app
- future migrations manageable

---

## Scope

## In scope

- GUI flow to trigger auth acquisition
- target configuration for helper-based auth acquisition
- local subprocess execution of an auth helper
- JSON contract between `hamm0r` and the helper
- secure storage of acquired auth material using the existing secret model
- first implementation path for bearer-token acquisition

## Out of scope for v1

- automatic background refresh
- cloud-based helper execution
- multi-step helper orchestration
- analyzer integration
- storing plaintext credentials in YAML
- deeply target-specific UI
- non-local secret vault integrations

---

## Architectural approach

## Core idea

Add a new target auth acquisition mode:

- `manual`
- `env_only`
- `http_login`

`http_login` means:

1. `hamm0r` builds a login HTTP request from target-side auth-helper config
2. credentials are resolved from env vars at runtime
3. `hamm0r` sends the login request directly
4. `hamm0r` extracts the bearer token from the HTTP response
5. `hamm0r` stores the resulting token using the existing secret flow

---

## Why HTTP login is the preferred v1 design

### Advantages

- keeps `hamm0r` target-agnostic
- no extra runtime such as Node.js required
- lower operational complexity
- faster feedback loop
- easier installation story
- makes switching target systems much easier
- respects the local-first model
- avoids browser automation unless it is truly needed

### Tradeoffs

- only works for targets whose auth can be reproduced as normal HTTP
  traffic
- request composition and response extraction need to be flexible enough
  for real-world login APIs

---

## Proposed feature model

## Target-level config

Each target should gain auth-helper-related metadata in addition to the
existing auth configuration.

Proposed concepts:

- auth acquisition mode
- login env var name
- password env var name
- login URL
- login method
- login headers
- login body template
- token extraction rule
- optional timeout

Example conceptual shape:

```yaml
auth_helper:
  mode: http_login
  login_env: PROFILER_LOGIN
  password_env: PROFILER_PASSWORD
  login_url: https://example.local/api/auth/login
  login_method: POST
  login_headers:
    Content-Type: application/json
  login_body_template: |
    {
      "email": "{{login}}",
      "password": "{{password}}"
    }
  token_extraction:
    type: jsonpath
    path: $.accessToken
  timeout_seconds: 60
```

Important:

- env var names may be stored
- secret values must not be stored in target YAML

---

## Request and extraction contract

Instead of a subprocess contract, v1 needs a configurable login request
and token extraction contract.

## Request inputs

The login request should be configurable with:

- URL
- method
- headers
- body template
- optional content type

Credentials are injected at runtime from env vars:

- `{{login}}`
- `{{password}}`

Optional future placeholders may include:

- `{{base_url}}`
- `{{timestamp}}`

## Response extraction

Recommended v1 extraction types:

- `jsonpath`
- `regex`

Preferred v1 path:

- `jsonpath` for JSON login responses

Example:

```yaml
token_extraction:
  type: jsonpath
  path: $.accessToken
```

---

## Auth material model

For v1, model this explicitly as bearer-token acquisition.

### v1 supported output

- bearer token

### v2 candidates

- custom auth header
- cookie-based session
- multiple headers

---

## UI/UX plan

## Target editor changes

Add an auth acquisition section in the target editor.

Suggested controls:

- auth type
- auth source
- token env var name
- login request configuration when `http_login` is selected
- button: `Fetch token`
- status line:
  - token present
  - fetched recently
  - optional expiry
  - last fetch failed

## User flow

1. User configures target auth as bearer
2. User chooses auth source `HTTP login`
3. User enters login config and env var names
4. User clicks `Fetch token`
5. `hamm0r` performs the login request directly
6. If successful, token is stored in keychain
7. UI updates token status
8. User can test target immediately

---

## Secret handling plan

This must follow the existing secret design in `hamm0r`.

### Rules

- username/password values stay outside YAML
- login/password values stay outside YAML
- `hamm0r` reads credentials from env vars at runtime
- acquired bearer token is stored through the existing keychain-backed mechanism
- token value is never shown back in plaintext
- token value is never logged
- failed request diagnostics must not leak credentials or token values

### Recommended storage behavior

After successful acquisition:

- call the equivalent of the current `set_bearer_token`
- store token under the configured token env-var-shadow name
- preserve the current runner behavior so no execution changes are needed

This is a key benefit of the design:

- run execution does not need to know where the token came from

---

## Profiler first implementation path

## Step 1: Capture real login request shape

Use the existing Playwright test only as a reference to verify the user
journey and expected success state.

For the actual feature, identify and document:

- login endpoint URL
- method
- required headers
- request body format
- response JSON shape
- exact field containing the bearer token

Given the current clarification, the token is expected to come from the
response of a network request.

Confirmed Profiler login request for v1:

- URL:
  `https://profiler-test.spirit-indianer.com/api/auth/users/login`
- Method:
  `POST`
- Required request header:
  - `Content-Type: application/json`
- Request body:

```json
{
  "email": "{{login}}",
  "password": "{{password}}"
}
```

- Token extraction:
  - JSONPath: `$.jwToken`

Important note:

- Browser-originated headers such as `sec-ch-ua`, `sec-ch-ua-platform`,
  `sec-ch-ua-mobile`, `Referer`, and the browser `User-Agent` should not
  be assumed to be required by default
- v1 should start with the minimal request shape and only add extra headers
  if the endpoint proves to require them

---

## Backend implementation plan

## New command surface

Add command(s) in the Tauri layer such as:

- `acquire_target_auth`

Responsibilities:

- validate target config
- resolve login/password from env vars
- build login request
- send login request with `reqwest`
- enforce timeout
- parse response body
- extract bearer token
- persist acquired token in keychain
- return safe status to UI

## What the backend must not do

- hardcode Profiler login semantics
- store plaintext username/password
- log request bodies or token values

---

## Data model changes

Target metadata likely needs additive fields for auth acquisition config.

Potential additions:

- `auth_source`
- `auth_login_url`
- `auth_login_method`
- `auth_login_headers`
- `auth_login_body_template`
- `auth_login_timeout_seconds`
- `auth_login_env`
- `auth_password_env`
- `auth_token_extraction_type`
- `auth_token_extraction_path`

These are target metadata concerns, not run artifact concerns.

They should not change:

- run JSONL schema
- verdict schema
- engagement artifact contract

That is important for preserving current invariants.

---

## Error handling

Expected failure modes:

- login endpoint unreachable
- timeout
- invalid response format
- missing env vars for username/password
- token extraction rule no longer matches
- credentials rejected

UI should show:

- short user-facing error
- safe diagnostic hint
- no secret leakage

Example:

- `Token fetch failed: login timed out after 60s`
- `Token fetch failed: PROFILER_LOGIN env var not set`
- `Token fetch failed: token field not found in response`

---

## Testing strategy

## Unit tests

- target config validation
- login request builder
- response extraction
- timeout handling
- safe storage of returned token
- error mapping

## Integration tests

- mock login success
- mock bad credentials
- mock invalid JSON response
- mock timeout
- mock extraction failure
- successful token storage visible via existing token status API

## Manual validation

- configure target with external helper
- configure target with HTTP login
- click `Fetch token`
- verify token status becomes available
- run target test successfully

---

## Rollout plan

### Phase 1

- bearer-token-only HTTP login
- one-button `Fetch token`
- existing keychain storage reuse
- Profiler login as first concrete implementation

### Phase 2

- token metadata in UI, for example fetched at / expires at
- better diagnostics

### Phase 3

- optional fallback helper for targets that cannot be handled with pure HTTP
- cookie/header bundles if needed in the future

---

## Recommended implementation order

1. Define helper JSON contract
1. Define target metadata additions for HTTP login
2. Document the Profiler login request and token response shape
3. Add backend command to perform login and store token
4. Add target editor UI for `Fetch token`
5. Wire status display into existing auth status area
6. Add tests and failure-state UX

---

## Risks

### 1. Target login API changes

Mitigation:

- keep the login request shape configurable
- avoid hardcoding one target too deeply

### 2. Token extraction field changes

Mitigation:

- document exact response shape per target
- make extraction rules configurable

### 3. Credentials leak through logs

Mitigation:

- reserve stdout for final JSON only
- keep helper logs on stderr
- sanitize logs

### 4. Future targets do not use simple bearer-token HTTP login

Mitigation:

- add a fallback design later for non-HTTP login targets

---

## Open questions

These should be clarified before implementation starts:

1. Soll v1 wirklich nur Bearer-Token unterstützen, oder möchtest du den Vertrag direkt so anlegen, dass auch Cookies und Header-Bundles unterstützt werden?

2. Soll die Request-Konfiguration komplett frei im Target pflegbar sein, oder möchtest du später zusätzlich Presets pro Zielsystem?

3. Reicht dir für v1 ausschließlich `Fetch token`, ohne automatisches Refresh?
