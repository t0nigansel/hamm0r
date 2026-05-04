# Current Status

Last updated: 2026-05-02

## Context

We are debugging the **Targets** page in the Tauri app.

User-visible symptoms:
- `Save Target + Request` appears to do nothing
- `+ Header` appears to do nothing
- Repro case used repeatedly: open the old target/profile `Profiler`, add one
  character to the body content, click save, reload target, change is gone

The user starts the app successfully from Git Bash with:

```bash
/c/Users/Gansel/.cargo/bin/cargo.exe tauri dev
```

This does launch the current app window.

## Important Findings

### 1. This does **not** currently look like a backend/storage bug

We added diagnostic logging for the target editor save flow.

Expected log lines would be emitted under component `target-editor` when:
- `+ Header` is clicked
- save starts
- validation passes
- `save_target_meta` is called
- `save_request` is called

Actual result from the user's log:

```text
[2026-05-02T11:10:57Z] [info] [app] [app_session_id=app-27132-1777720257971] Application startup
[2026-05-02T11:10:57Z] [info] [tauri] [app_session_id=app-27132-1777720257971] Tauri setup completed
[2026-05-02T11:10:57Z] [info] [app] [app_session_id=app-27132-1777720257971] UI ready
```

There are **no** `target-editor` log lines at all.

That means the click/save flow is not reaching the instrumented JS path.

### 2. Because of that, several earlier hypotheses are now unlikely

These are likely **not** the primary cause:
- YAML write failure
- `save_request` backend bug
- `save_target_meta` backend bug
- storage-layer persistence bug
- JSON body parse failure inside the save path

If the save path had started, we would have seen at least one
`target-editor` log line.

### 3. The problem is now believed to be earlier than save logic

Most plausible remaining causes:

1. The running window is still not executing the current `ui/js/app.js`
   despite restart / asset version bump
2. Clicks are not reaching the target editor controls on the Targets page
   (layout / hit-testing / overlay / interaction issue)

At this point, the second option is the stronger suspect.

## Relevant Files

### Frontend

- `ui/index.html`
- `ui/js/app.js`
- `ui/js/api.js`
- `ui/style.css`

### Backend logging support added for debugging

- `crates/hamm0r/src/commands.rs`
- `crates/hamm0r/src/main.rs`

## Changes Already Made In This Session

### Target editor / save-path attempts

- Added explicit request draft reconstruction with `ensureCurrentRequestDraft()`
- Added clearer validation before save
- Added asset version bump in `ui/index.html`
- Added `novalidate` to `#target-form`
- Added delegated click/submit handling attempts
- Added target-editor diagnostic logging via a new Tauri command

### Logging command added

New command:
- `log_ui_debug`

Purpose:
- log frontend save/click flow safely without body contents

### Current diagnostic result

The diagnostic command works in principle, but **no target-editor events were
logged from the user's click attempts**.

## Files Modified During Investigation

These files were touched while debugging this issue:

- `ui/index.html`
- `ui/js/app.js`
- `ui/js/api.js`
- `crates/hamm0r/src/commands.rs`
- `crates/hamm0r/src/main.rs`

There are also many other repo changes from prior work in this branch/worktree.
Do **not** assume a clean working tree.

## State Of The User Data Used For Repro

Target file:
- `C:\Users\Gansel\hamm0r\targets\profiler-molg8vnq.yaml`

Request file:
- `C:\Users\Gansel\hamm0r\requests\profiler-molg8vnq.yaml`

Current target YAML is minimal and old-style:

```yaml
version: 1
id: profiler-molg8vnq
name: Profiler
request_id: profiler-molg8vnq
```

This may matter because the bug is repeatedly reproduced with an **older**
target/profile rather than a freshly created one.

## What The Next Session Should Do First

1. Read this file.
2. Do **not** start by changing backend persistence.
3. Focus on the **Targets view interaction layer**:
   - whether clicks reach the buttons
   - whether the current `app.js` is truly loaded
   - whether something in layout/CSS blocks pointer interaction
4. Verify at runtime whether the target editor controls are hit-testable.

## Suggested Next Technical Step

The next session should perform a **focused runtime/UI interaction diagnosis**,
not another broad save-path refactor.

Best next step:
- instrument the Targets page with unmistakable visible diagnostics
  (for example temporary UI text mutation on button click, or a minimal
  top-level click tracer for the target editor area)
- or inspect the interaction/hit-test behavior directly if runtime tooling is
  available

The key question to answer first is:

> Are clicks on `#btn-add-header-row` and the target form area actually
> reaching the frontend code in the running app window?

