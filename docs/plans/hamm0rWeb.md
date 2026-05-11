# hamm0rWeb Roadmap

## Goal

Create a separate project called `hamm0rWeb` as a client-server web
application that can run in Azure and be embedded into another web
application via `iframe`.

The existing local-first desktop application, `hamm0r`, remains a
separate project with its own architecture, release flow, and product
positioning.

This is intentionally not a migration of the current Tauri app into the
browser. It is a sibling product that may reuse selected core logic, but
must not force the local-first desktop app to become cloud-native.

---

## Product split

### `hamm0r`

- Local-first desktop tool
- Tauri-based UI
- Files on local disk as source of truth
- No cloud dependency in the default workflow
- Optional local analyzer

### `hamm0rWeb`

- Browser-based client-server application
- Hosted backend, for example on Azure
- Multi-user and tenant-aware
- Browser-safe frontend
- Embeddable in another application via `iframe`

### Shared principle

- Reuse domain logic where it is genuinely portable
- Do not force the desktop product to adopt server assumptions
- Do not let cloud requirements weaken the desktop invariants

---

## Non-goals

- Do not retrofit the current Tauri frontend to run directly in the browser
- Do not replace the existing desktop app with the web app
- Do not introduce cloud dependencies into the current local-first core flow
- Do not try to make one codebase serve both products at every layer from day one

---

## High-level architecture

### `hamm0rWeb` frontend

- Standalone web client
- Talks to backend over HTTP
- Uses WebSocket or SSE for run progress
- Supports embed mode in `iframe`
- No dependency on `window.__TAURI__`

### `hamm0rWeb` backend

- Authenticated API server
- Job orchestration for runs
- Persistent storage for engagements, runs, responses, and exports
- Secure secret handling for target credentials
- Optional future analyzer service

### Shared reusable logic

Potential reuse candidates from the current repo:

- Runner execution logic
- Request building and substitution logic
- Prompt parsing and validation
- OWASP tagging and reporting helpers

Likely not reusable as-is:

- Tauri command layer
- Tauri event model
- Local filesystem assumptions
- OS keychain integration
- Desktop export/open-file UX

---

## Workstreams

## 1. Product boundary

Define the boundary between the two projects before implementation begins.

Deliverables:

- Written decision that `hamm0rWeb` is a separate product
- Naming and repo strategy
- Ownership boundary between shared libraries and product-specific code
- Clear statement of which invariants remain desktop-only

Key decisions:

- Separate repo vs monorepo sibling project
- Shared crates package strategy
- Versioning strategy between `hamm0r` and `hamm0rWeb`

Recommendation:

- Keep `hamm0r` unchanged as the desktop repo
- Start `hamm0rWeb` as a separate project
- Extract shared logic later and only where justified by real reuse

---

## 2. API contract

Define a browser/server contract before building UI or backend details.

Core API domains:

- Authentication and session
- Prompt library
- Targets
- Requests
- Scenarios
- Engagements
- Runs
- Results
- Exports

Progress/event transport:

- WebSocket or SSE for run progress
- Polling fallback if needed

Deliverables:

- API spec document
- Run lifecycle state machine
- Error contract
- Export contract

Important difference from desktop:

- Tauri `invoke` commands become HTTP endpoints
- Tauri `emit` events become server push or polling

---

## 3. Storage model

Design storage for hosted multi-user usage.

Current desktop model:

- Filesystem is source of truth
- `~/hamm0r/engagements/...`

Web requirements:

- Tenant separation
- User separation
- Durable hosted storage
- Searchable run metadata
- Artifact download and retention policy

Proposed direction:

- Relational database for metadata and indexing
- Blob/object storage for response bodies and report artifacts
- Explicit artifact references instead of local relative paths

Deliverables:

- Data model for users, orgs, engagements, runs, results
- Artifact storage strategy
- Retention and cleanup policy
- Migration and backup strategy

---

## 4. Secret handling

This must be redesigned early because the desktop model depends on OS
keychain and env vars.

Web requirements:

- Per-user or per-tenant secret ownership
- Encryption at rest
- Secure injection into run execution
- Auditability
- Least-privilege access model

Likely direction:

- Azure Key Vault or equivalent managed secret store
- Backend-owned secret references
- Never expose stored secrets back to the browser

Deliverables:

- Secret lifecycle design
- Authz rules for secret access
- Rotation and revocation plan

---

## 5. Execution model

The current runner is invoked in-process from the desktop shell. In
`hamm0rWeb`, execution must become a backend job model.

Needs:

- Queue or job system
- Retry policy
- Concurrency limits
- Cancellation
- Timeouts
- Per-tenant fairness and resource controls

Deliverables:

- Run worker design
- Job status model
- Cancellation and failure semantics
- Scaling model for Azure

---

## 6. Frontend rebuild

The existing UI can inspire the web UI, but should not be treated as a
drop-in browser build.

Needed changes:

- Remove Tauri-only assumptions
- Replace desktop file flows with browser download/open flows
- Design auth-aware navigation
- Support embed mode inside an `iframe`
- Handle resize, routing, and host integration cleanly

Deliverables:

- Web client architecture
- Embed mode UX rules
- Component map from current desktop UX to web UX

---

## 7. iFrame embedding

Embedding must be designed deliberately, not added at the end.

Requirements:

- `frame-ancestors` CSP configured for allowed host applications
- No conflicting `X-Frame-Options`
- Optional `postMessage` contract to communicate with host app
- Stable sizing and layout behavior inside constrained containers
- Auth/session strategy for embedded usage

Questions to answer:

- Does the host app pass identity context?
- Does the iframe app maintain its own login?
- Should the host app be able to start runs or open engagements by message?

Deliverables:

- Embed mode spec
- Host-to-iframe event contract
- CSP and security checklist

---

## 8. Reporting and exports

Desktop export flows rely on local file access and OS-open behavior. Web
exports must be server-generated or browser-generated in a controlled way.

Needs:

- Downloadable Markdown and PDF artifacts
- Shareable URLs or authenticated downloads
- Server-side export generation where appropriate
- Clear distinction between preview and persisted artifact

Deliverables:

- Export pipeline design
- Artifact naming convention
- Download endpoint rules

---

## 9. Analyzer strategy

This is a fork in the road and should be deferred until core hosted runs
work well.

Options:

1. No analyzer in v1 of `hamm0rWeb`
2. Hosted analyzer service
3. Bring-your-own model endpoint

Recommendation:

- Exclude analyzer from `hamm0rWeb` v1
- Ship raw evidence, exports, and reports first
- Revisit analyzer after run execution, storage, and security are stable

Reason:

- Analyzer introduces compute, cost, privacy, and model-governance complexity

---

## 10. Security and compliance

Because this is a hosted system, this workstream is first-class.

Needs:

- Authentication
- Authorization
- Tenant isolation
- Audit logs
- Input validation
- Abuse protections
- Secure artifact access
- Network egress policy for target execution

Azure-specific topics:

- Managed identity
- Key Vault
- Blob storage access control
- App Gateway / Front Door / WAF
- Private networking if required

Deliverables:

- Threat model
- Security controls checklist
- Compliance assumptions and gaps

---

## 11. Delivery phases

### Phase 0: Decision and scope

- Confirm separate-project strategy
- Confirm v1 scope for `hamm0rWeb`
- Decide repo and package structure

### Phase 1: API and data model

- Define HTTP API
- Define hosted data model
- Define artifact storage model

### Phase 2: Execution backend

- Implement run orchestration backend
- Persist engagements, runs, and results
- Add progress streaming

### Phase 3: Web client

- Build browser-safe frontend
- Recreate core workflows: target, scenario, run, results
- Remove all Tauri dependencies

### Phase 4: Embed mode

- Add iframe-safe rendering
- Add host integration contract
- Validate auth/session story

### Phase 5: Export and reporting

- Add Markdown/PDF export flows
- Add artifact previews and downloads

### Phase 6: Hardening

- Security review
- Load and resilience testing
- Tenant isolation validation
- Azure deployment hardening

### Phase 7: Optional analyzer

- Reassess whether hosted analyzer belongs in the web product

---

## Suggested repo strategy

Option A: Separate repos

- `hamm0r` for desktop
- `hamm0rWeb` for hosted app
- Extract shared crates later into a third shared package if needed

Pros:

- Clean product boundary
- No accidental architecture leakage
- Easier independent release cadence

Cons:

- Shared code extraction comes later

Option B: Monorepo with sibling apps

- `apps/hamm0r-desktop`
- `apps/hamm0r-web`
- `crates/shared-*`

Pros:

- Easier shared refactors
- Single place for shared docs and CI

Cons:

- More discipline required to preserve boundaries

Recommendation:

- Start with separate projects unless there is already strong operational
  value in keeping a monorepo

---

## First implementation target

The first useful `hamm0rWeb` milestone should be:

- login
- create target
- create engagement
- start run
- watch progress
- inspect results
- export Markdown

This is enough to validate the hosted architecture without dragging in
the analyzer or every desktop feature.

---

## Success criteria

`hamm0rWeb` is on track when:

- It runs fully in a browser without Tauri dependencies
- It can be embedded in another app via `iframe`
- A user can launch a run and inspect results remotely
- Exports work in hosted mode
- The desktop `hamm0r` product remains independent and local-first
- No cloud-only assumptions leak back into the desktop architecture

---

## Open questions

- Should `hamm0rWeb` support the full desktop scenario model in v1?
- Is embed mode the primary mode or only one integration surface?
- What auth model does the host application require?
- Is tenant isolation per customer, per workspace, or per user?
- Do target credentials belong to individual users or shared workspaces?
- Is analyzer functionality needed in the first hosted release?
