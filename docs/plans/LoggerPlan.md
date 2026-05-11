# Logger Plan

This document is the implementation plan for application logging in
hamm0r. It is the agreed basis for the work and is meant to be
actionable step by step.

## Agreed scope

- Primary goal: debugging / error diagnosis
- Audience: developers
- Log destination: files on disk
- Log root: `~/hamm0r/logs/`
- Components:
  - core logs: `~/hamm0r/logs/hamm0r/`
  - analyzer logs: `~/hamm0r/logs/analyz0r/`
- Active log files:
  - `hamm0r.log`
  - `analyz0r.log`
- Retention: max 5 files per component
- Rotation:
  - on every app start
  - and whenever a log file exceeds 100 MB
- Release default:
  - logging enabled
  - level `info`
  - lifecycle + error logging
- Log levels:
  - `error`
  - `info`
  - `debug`
- Body logging:
  - controlled by app setting
  - full bodies allowed in debug mode
  - if body size exceeds 500 KB, log a message like:
    `Won't log the payload since the size is 632 KB`
- Header logging:
  - log all headers
  - mask secrets in known secret-bearing headers
- Correlation fields:
  - `app_session_id`
  - `run_id` where available
- Frontend logs: not included
- UI error surfacing: only for user-relevant errors
- Logging settings are not live-switched; changes apply after restart

## Working assumption

- [x] Rotation naming scheme is confirmed in implementation as:
  - active file stays `hamm0r.log` / `analyz0r.log`
  - rotated files use `.1`, `.2`, `.3`, `.4`
  - max 5 files total per component including the active file

## Implementation plan

### 1. Logging concept and event catalog

- [x] Define the logging components used throughout the app:
  - `app`
  - `settings`
  - `storage`
  - `tauri`
  - `runner`
  - `analysis`
  - `analyz0r`
- [x] Define the canonical log line format:
  - `[timestamp] [level] [component] [app_session_id=...] [run_id=...] message`
- [x] Define which lifecycle events must always be logged:
  - app start
  - app shutdown
  - settings load
  - UI ready
  - target load
  - run start
  - run finish
  - run failure
  - analysis start
  - analysis finish
  - analysis failure
  - analyzer activation
  - analyzer download/install
  - relevant Tauri command execution
  - relevant file write operations in `debug`

### 2. Settings model

- [x] Extend persisted app settings with `logging_enabled`
- [x] Extend persisted app settings with `logging_level`
- [x] Extend persisted app settings with `body_logging_enabled`
- [x] Define default values:
  - logging enabled
  - level `info`
  - body logging disabled
- [x] Ensure logging settings are loaded at app startup
- [x] Ensure changed logging settings require app restart to take effect

### 3. Core logger infrastructure

- [x] Add file logger initialization for `~/hamm0r/logs/hamm0r/`
- [x] Create or open active file `hamm0r.log`
- [x] Rotate core logs on every app start
- [x] Rotate core logs when active file exceeds 100 MB
- [x] Enforce retention of max 5 core log files
- [x] Keep logger output human-readable, not JSON
- [x] Support multiline log entries for request/response bodies
- [x] Keep runtime overhead low enough for normal runs

### 4. Analyzer logger infrastructure

- [x] Add separate file logger initialization for `~/hamm0r/logs/analyz0r/`
- [x] Create or open active file `analyz0r.log`
- [x] Rotate analyzer logs on every app start
- [x] Rotate analyzer logs when active file exceeds 100 MB
- [x] Enforce retention of max 5 analyzer log files
- [x] Keep analyzer logs fully separate from core logs

### 5. Redaction rules

- [x] Define the list of known secret-bearing headers to mask
- [x] Mask `Authorization` header values
- [x] Mask auth-related custom header values from request/auth configuration
- [x] Ensure masked headers still preserve enough structure for debugging
- [x] Keep header logging enabled for both requests and responses
- [x] Restrict redaction scope to known secret-bearing headers for the first implementation

### 6. Request/response logging in the runner

- [x] Log request lifecycle for every attempt
- [x] Log request metadata:
  - method
  - URL
  - headers
  - body size
  - `run_id`
- [x] Log response metadata:
  - status code
  - headers
  - body size
  - duration
  - `run_id`
- [x] Log request bodies when body logging is enabled
- [x] Log response bodies when body logging is enabled
- [x] Replace body output above 500 KB with a size notice instead of the full body
- [x] Keep run artifacts on disk as the primary source of truth; app logs only extend them

### 7. App and Tauri lifecycle logging

- [x] Log app start
- [x] Log app shutdown
- [x] Log settings load success/failure
- [x] Log UI ready
- [x] Log Tauri command invocation
- [x] Log Tauri command success
- [x] Log Tauri command failure
- [x] Log analyzer activation flow
- [x] Log analyzer download/install flow
- [x] Log analysis lifecycle in the desktop shell
- [x] Log relevant storage actions in `debug`

### 8. User-relevant error handling

- [x] Implement a hard-coded first-pass list of user-relevant errors
- [x] Mark run startup failures as user-relevant
- [x] Mark missing request/target/scenario errors as user-relevant
- [x] Mark network execution failures as user-relevant
- [x] Mark request timeout failures as user-relevant
- [x] Mark run artifact write failures as user-relevant
- [x] Mark analyzer activation failures as user-relevant
- [x] Mark analyzer download/model load failures as user-relevant
- [x] Mark analysis execution failures as user-relevant
- [x] Mark required artifact read failures as user-relevant
- [x] Log all user-relevant errors at `error`
- [x] Surface user-relevant errors to the UI in addition to logging them

### 9. Correlation fields

- [x] Generate `app_session_id` on app startup
- [x] Attach `app_session_id` to all log entries
- [x] Attach `run_id` to all log entries where a run context exists
- [x] Ensure correlation fields are present in both core and analyzer logs

### 10. Tests and verification

- [x] Add tests for startup rotation
- [x] Add tests for size-based rotation at 100 MB
- [x] Add tests for max-5 retention per component
- [x] Add tests for secret masking in headers
- [x] Add tests for body logging enabled/disabled
- [x] Add tests for the 500 KB body cutoff message
- [x] Add tests for core/analyzer log separation
- [x] Add tests for user-relevant error surfacing
- [x] Verify release-default behavior:
  - logging enabled
  - level `info`
  - lifecycle + error logs
  - body logging disabled unless explicitly enabled

### 11. Settings UI

- [x] Add a Settings section for logging
- [x] Add toggle for logging enabled/disabled
- [x] Add selector for logging level
- [x] Add toggle for body logging enabled/disabled
- [x] Add UI copy that logging changes apply after app restart

## Recommended implementation order

- [x] Step 1: settings model + logger foundation
- [x] Step 2: core file logging + rotation
- [x] Step 3: runner request/response logging
- [x] Step 4: user-relevant error surfacing
- [x] Step 5: analyzer logging
- [x] Step 6: settings UI
- [x] Step 7: tests and refinement

## Risks to watch

- [ ] Logging full bodies can produce very large logs quickly
- [ ] Header-only masking does not protect secrets embedded in bodies
- [ ] Excessive `debug` logging in the runner can hurt performance
- [ ] Multiline body logs need careful formatting to remain readable
