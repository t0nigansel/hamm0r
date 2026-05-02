# HTTP Request Plan

This document defines the implementation plan for a reusable HTTP request
editor in hamm0r.

The goal is to let users build and reuse arbitrary HTTP requests from the UI,
without requiring manual YAML edits, while preserving the existing local-first
storage model.

## Scope

- A Target represents a system/API base context.
- A Target can own multiple Requests.
- A Request is a named, reusable object stored separately and attached to one
  Target.
- Scenarios should be able to reference Requests directly.
- The UI should expose a powerful HTTP editor, closer to Bruno/Postman/JMeter
  than to the current simplified target form.
- YAML remains the storage format only. It must not become a user-facing
  concept.

## V1 Decisions

- One powerful HTTP editor, not split into a separate simple/raw product mode
- General HTTP support: method, URL, headers, query params, body, response
  parsing
- Header editing in two views:
  - structured key/value table
  - raw header block
- Body editing starts with text-based formats only
- `curl` import is included in V1
- Auth remains a dedicated convenience section, but user-defined headers keep
  full control and can override generated auth headers
- Placeholder support in V1 is limited to existing built-ins such as
  `{{prompt}}`
- Test Request is included in V1
- Test Request should show both:
  - raw response
  - extracted response according to configured parsing
- Test Request events should be logged
- Wizard gets its own request-building step
- No V1 support for:
  - multipart uploads
  - scripting
  - assertions
  - collection runners
  - user-defined variables/properties

## Data Model Direction

- [x] Keep `Target` as the system/API-level object
- [x] Let one `Target` reference many `Request` objects
- [x] Keep each `Request` attached to exactly one `Target`
- [x] Store `Request` as a first-class object with stable `id` and `name`
- [x] Let Scenario steps reference `Request` objects directly instead of only a
      single Target-level request
- [x] Keep YAML as the internal persistence layer, but remove YAML from the
      normal user workflow

## Storage and Schema

- [x] Review the current `storage::types::Request` schema against the new UI
      requirements
- [ ] Extend the `Request` schema to represent:
  - arbitrary HTTP method
  - full URL
  - arbitrary headers
  - raw header text view support
  - arbitrary text body
  - body content type
  - response extraction mode (`raw`, `jsonpath`, `regex`)
- [x] Add support for Requests belonging to a Target without forcing a
      one-request-per-target mapping
- [x] Decide and document how Target-to-Request association is stored on disk
- [x] Update [docs/Datamodel.md](/c:/workspace/hamm0r/docs/Datamodel.md) with
      the new request and target relationship
- [x] Update migration expectations for existing single-request Targets

## Migration Strategy

- [x] Preserve existing Targets created with the current simplified UI
- [ ] Define a migration path from:
  - Target with inline request assumptions
  - one request per target
  - request fields like `request_field` / `response_field`
- [x] Ensure old Targets still load in the new editor
- [x] Ensure existing saved runs remain readable without migration

## Backend Commands

- [x] Review the current Tauri commands for Targets and Requests
- [x] Introduce first-class Request commands where needed, instead of treating
      Request as an implementation detail of Target saving
- [x] Support CRUD operations for multiple Requests per Target
- [x] Support loading a Target and its attached Requests efficiently
- [ ] Add a backend command for `curl` import parsing
- [x] Add a backend command for Test Request execution
- [x] Make Test Request execution return:
  - response status
  - response headers
  - raw response body
  - extracted response body
  - timing data

## Execution Model

- [x] Keep full run execution compatible with reusable Request objects
- [x] Allow a Run to select a specific Request for execution
- [x] Allow a Scenario step to reference a specific Request directly
- [ ] Define how Request selection interacts with prompt injection using
      `{{prompt}}`
- [x] Ensure the runner still supports general HTTP, not only LLM-like payload
      shapes

## Logging

- [x] Log Test Request execution through the existing logging system
- [x] Mark Test Request logs clearly so they can be distinguished from normal
      run attempts
- [x] Reuse the current secret masking rules for auth headers
- [x] Apply the existing body logging settings to Test Request execution
- [x] Ensure the logging output remains human-readable for raw HTTP workflows

## Targets UI

- [x] Extend the Targets view instead of replacing it outright
- [x] Keep the current Target identity fields, but add a full Request editor
      area
- [x] Add a request list within a Target so one Target can manage multiple
      Requests
- [x] Add create/select/rename/delete affordances for Requests under a Target
- [x] Add one powerful HTTP editor for the selected Request
- [x] Add fields for:
  - method
  - URL
  - auth convenience config
  - headers
  - query params
  - body
  - response parsing
- [x] Add structured and raw header editing with explicit switching
- [x] Add body editing for text-based content first
- [x] Make the editor capable of expressing the real profiler example without
      YAML edits
- [x] Keep the UI dense and work-focused rather than form-heavy and decorative

## Request Builder UX

- [x] Design the request editor so the user can build requests like in
      Bruno/Postman/JMeter, but inside hamm0r's simpler local model
- [x] Keep request construction explicit and inspectable
- [x] Avoid hiding important HTTP details behind too much abstraction
- [x] Show how auth convenience settings affect the final outgoing request
- [x] Allow manual override of generated auth headers
- [x] Make it obvious when `{{prompt}}` is used in the body or headers

## curl Import

- [x] Add `curl` import entry point in the Request editor
- [x] Parse imported `curl` into structured request fields where possible
- [x] Support at least:
  - method
  - URL
  - headers
  - body
- [x] Extract auth-related headers into the auth convenience model where
      possible
- [x] Fall back gracefully when import cannot be perfectly normalized
- [x] Let the user inspect and edit the imported request before saving

## Response Parsing UX

- [x] Let the user configure `raw`, `jsonpath`, or `regex` extraction from the
      Request editor
- [ ] Explain extraction in plain UI language
- [x] Show extracted preview alongside raw response in Test Request results
- [x] Keep extraction configuration reusable for full Runs and Scenarios

## Test Request

- [x] Add a Test Request action in the Request editor
- [x] Execute exactly the configured request, including auth, headers, body,
      and extraction
- [x] Show:
  - status
  - headers
  - raw response
  - extracted result
  - duration
- [x] Route failures to both logs and UI feedback
- [x] Keep Test Request independent from full Run creation

## Wizard

- [ ] Add a dedicated HTTP Request step to the Wizard
- [ ] Keep the Wizard as a guided path, not the most advanced editing surface
- [ ] Let the Wizard create a valid Target plus at least one Request
- [ ] Avoid duplicating the entire editor logic; reuse the same Request model
      and components as much as possible

## Scenario Model

- [x] Update Scenario flow so steps can reference a concrete Request directly
- [ ] Decide how Scenario UX chooses among Requests from the selected Target
- [ ] Keep Scenario authoring understandable when multiple endpoints exist
- [ ] Avoid redundant redefinition of the same HTTP request inside a Scenario

## Compatibility and Safety

- [x] Preserve local-first behavior
- [x] Keep core independent from analyzer concerns
- [x] Keep file I/O inside the storage layer
- [x] Do not require YAML editing for standard use
- [ ] Keep auth secrets out of stored request payload defaults unless the user
      explicitly chooses env-var-backed auth

## Testing

- [x] Add storage tests for the new Target/Request relationship
- [ ] Add backend command tests for Request CRUD
- [ ] Add tests for `curl` import
- [x] Add tests for Test Request execution
- [x] Add tests for auth override behavior
- [ ] Add tests for raw vs structured header editing normalization
- [x] Add scenario tests for direct Request references
- [ ] Add regression tests for existing simple Targets

## Documentation

- [x] Update [docs/Datamodel.md](/c:/workspace/hamm0r/docs/Datamodel.md)
- [ ] Update any affected architecture or workflow docs
- [ ] Document the Request editor behavior from a user perspective
- [ ] Document V1 limitations clearly

## Suggested Implementation Order

- [ ] Finalize data model for Target ↔ Request relationship
- [x] Extend storage types and persistence
- [x] Add backend CRUD and Test Request commands
- [x] Add `curl` import support
- [x] Extend Targets UI with multi-request editor
- [x] Add Request editor Test Request workflow
- [x] Wire Requests into Scenario selection
- [ ] Add Wizard request step
- [ ] Add tests and documentation updates

## Open Follow-Ups After V1

- [ ] User-defined variables and properties
- [ ] Richer placeholder system beyond `{{prompt}}`
- [ ] Multipart/form-data support
- [ ] More advanced request collections or request chaining
- [ ] Better diff/preview of the final rendered outgoing request
