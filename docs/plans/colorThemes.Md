# Color Themes Plan

## Goal

Add a second application theme named **Spirit Testing**, inspired by the
provided screenshot: a bright, enterprise-style interface with an orange
top bar, white canvas, pale grey cards/panels, dark readable body text,
and orange/brown action accents.

Add a third application theme named **TestSolutions**, inspired by the
provided TestSolutions screenshot: a bright interface with mint header
surfaces, white canvas, black primary actions/text, grey secondary text,
and small green/blue accents.

This plan covers the steps to implement the theme picker that already
exists in Settings.

## Product Fit

This aligns with `docs/productVision.md`: theming is local UI preference
work, does not add cloud calls, does not affect the attack workflow, and
does not require users to edit YAML in the standard journey.

The change must remain core-only. It must not depend on `crates/analyzer/`
or any model/runtime path.

## Current State

- `ui/index.html` already has a Settings -> Appearance -> Theme `<select>`,
  but it is disabled and the inline note says alternate themes are not
  implemented yet.
- The select already includes:
  - `default` -> `hamm0r (default)`
  - `spirit` -> `Spirit Testing`
  - `testsolutions` -> `TestSolutions`
- `crates/storage/src/types.rs` already has `AppConfig.ui.theme`, but the
  enum currently supports only `system`, `light`, and `dark`.
- `crates/hamm0r/src/commands/app_settings.rs` does not expose or save UI
  theme settings in `AppSettingsDto`.
- `ui/js/app.js` loads and saves analyzer/logging settings, but it does
  not read from or write to `#settings-theme`.
- `ui/style.css` is already token-driven via CSS variables under `:root`,
  which is the right foundation for theme variants.

## Decisions Needed Before Implementation

Answered product decisions:

1. **Spirit Testing changes colors only.** Do not change layout structure to
   mimic the screenshot.
2. **Keep hamm0r branding.** Do not swap the top-bar logo or product name.
3. **Keep `TestSolutions` for later, but hide it until it is ready.**
4. **TestSolutions changes colors only.** Do not change layout structure or
   branding to mimic the screenshot.

## Theme Model

Use explicit app theme IDs instead of mapping this to operating-system
light/dark:

- `default` -> current hamm0r theme
- `spirit_testing` -> new Spirit Testing theme
- `testsolutions` -> new TestSolutions theme

Implementation detail:

- Either extend `storage::types::Theme` with `Default` and
  `SpiritTesting`, keeping serde compatibility for old `system/light/dark`
  configs, or introduce a new UI-specific enum whose labels match actual
  app themes.
- Preserve backward compatibility for existing config files:
  - `system`, `dark`, and absent values should resolve to the current
    hamm0r default theme.
  - `light` can resolve to `spirit_testing` only if that is an explicit
    product decision; otherwise keep it as a legacy alias for default until
    a real light theme exists.

## Implementation Steps

### 1. Storage Contract

Files:

- `crates/storage/src/types.rs`
- `crates/storage/src/settings.rs`
- `docs/Datamodel.md`

Steps:

1. Add theme values for Spirit Testing and TestSolutions, serialized as
   `spirit_testing` and `testsolutions`.
2. Keep old `config.yaml` values readable.
3. Update default config creation tests to assert the default theme.
4. Add round-trip tests for the new theme value.
5. Update `docs/Datamodel.md` so `ui.theme` lists the supported values.

### 2. Tauri Settings DTO

File:

- `crates/hamm0r/src/commands/app_settings.rs`

Steps:

1. Add a `UiSettingsDto` with a `theme: String` field.
2. Add `ui: UiSettingsDto` to `AppSettingsDto`.
3. Populate `ui.theme` in `get_app_settings`.
4. Parse and save `ui.theme` in `save_app_settings`.
5. Add a test that a save payload with `ui.theme` deserializes.
6. Reject unknown theme values with a clear error.

### 3. Frontend Settings Wiring

Files:

- `ui/index.html`
- `ui/js/app.js`

Steps:

1. Enable `#settings-theme`.
2. Keep implemented theme options active.
3. On `loadAppSettings()`, set the select to the saved theme.
4. On settings save, include:

   ```js
   ui: {
     theme: $('#settings-theme').value
   }
   ```

5. Apply the theme immediately when settings load and after save.
6. Consider applying theme on select change for live preview, while still
   using the Save button as the persistence point.
7. Replace the "Alternate themes are not implemented yet" note with a
   persistence note.

### 4. Theme Application

Files:

- `ui/js/app.js`
- `ui/style.css`

Steps:

1. Add a small `applyTheme(theme)` function that sets an attribute on the
   document root, for example:

   ```js
   document.documentElement.dataset.theme = theme || 'default';
   ```

2. Call it as early as possible after settings load.
3. Add CSS token overrides:

   ```css
   html[data-theme="spirit_testing"] {
     color-scheme: light;
     --bg-app: #ffffff;
     --bg-sidebar: #f7f7f7;
     --bg-surface: #f3f7f8;
     --bg-surface-alt: #ffffff;
     --header-bg: #ff4b00;
     --header-border: #ff4b00;
     --color-accent: #b83a00;
     --color-accent-hover: #8f2d00;
     --color-text-primary: #111111;
     --color-text-secondary: #2b2b2b;
     --color-text-muted: #5d6468;
     --color-border: #d9e0e3;
   }
   ```

   ```css
   html[data-theme="testsolutions"] {
     color-scheme: light;
     --bg-app: #ffffff;
     --bg-sidebar: #ffffff;
     --bg-surface: #f7fbfa;
     --bg-surface-alt: #ffffff;
     --header-bg: #bdf2e1;
     --header-border: #bdf2e1;
     --color-accent: #00d59f;
     --color-accent-hover: #00b98a;
     --color-text-primary: #050505;
     --color-text-secondary: #333333;
     --color-text-muted: #555c5a;
     --color-border: #dce8e4;
   }
   ```

4. Audit hard-coded dark colors in `ui/style.css` and replace the ones that
   visibly break the light theme with semantic tokens.
5. Keep the default hamm0r theme visually unchanged.

### 5. Component Polish For Spirit Testing

File:

- `ui/style.css`

Likely overrides:

- Header: bright orange background, white icon/text treatment.
- Main canvas: white or near-white.
- Panels/cards: pale grey surfaces with subtle border and softer shadow.
- Primary buttons: orange/brown fill.
- Ghost buttons: light background with orange hover/focus.
- Inputs/selects/textareas: white backgrounds, dark text, grey borders.
- Tables and lists: high contrast text, pale hover rows.
- Toasts/modals: ensure readable foreground/background combinations.
- Status colors: keep semantic green/red/yellow legible on light surfaces.

Avoid turning this into a different application layout unless question 1 is
answered that way.

### 5a. Component Polish For TestSolutions

File:

- `ui/style.css`

Likely overrides:

- Header: mint background, black text/icon treatment.
- Main canvas: white.
- Panels/cards: very light mint/white surfaces with subtle borders.
- Primary buttons: black fill with white text.
- Ghost buttons: white background with mint hover/focus.
- Inputs/selects/textareas: white backgrounds, dark text, grey borders.
- Tables and lists: high contrast text, pale mint hover rows.
- Toasts/modals: ensure readable foreground/background combinations.
- Status colors: keep semantic green/red/yellow legible on light surfaces.

Avoid turning this into a different application layout or rebrand.

### 6. Visual Verification

Run the app with:

```powershell
cd crates/hamm0r
cargo tauri dev
```

Manual checks:

1. Open Settings -> Appearance.
2. Select Spirit Testing and save.
3. Confirm the app switches without restart.
4. Restart the app and confirm the theme persists from `~/hamm0r/config.yaml`.
5. Visit Home, Requests, Libraries, Scenarios, Runs, Settings, and modals.
6. Confirm text remains readable and controls do not overlap.
7. Select TestSolutions and repeat the same readability checks.
8. Switch back to the default theme and confirm the original dark design is
   still intact.

Automated checks:

```powershell
cargo fmt --all
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## Files Expected To Change

- `crates/storage/src/types.rs`
- `crates/storage/src/settings.rs`
- `crates/hamm0r/src/commands/app_settings.rs`
- `docs/Datamodel.md`
- `ui/index.html`
- `ui/js/app.js`
- `ui/style.css`

## Non-Goals

- No analyzer changes.
- No new dependencies.
- No runtime network calls.
- No YAML editing UI.
- No database or additional preference store.
- No broad frontend framework or build pipeline change.
- No rebrand unless explicitly approved.

## Risks

- Many CSS rules still contain hard-coded dark colors. The theme may need a
  focused cleanup pass so the light theme is not a patchwork.
- Existing config values use `system/light/dark`, while the UI select uses
  `default/spirit/testsolutions`. The implementation must normalize these
  deliberately to avoid breaking existing users.
- Applying the theme only after async settings load may cause a brief default
  theme flash on startup. A later improvement could store the last applied
  theme in local storage for earlier paint, but `config.yaml` should remain
  the source of truth.
