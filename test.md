# Test Directives

Guidance for the test maintainer agent on HOW to test hamm0r.
These directives apply on top of the requirements in `req.md`.

---

## Scope

Every view listed in REQ-001 needs at minimum:
- An **empty-state test** (backend returns nothing)
- A **populated-state test** (backend returns mock data)
- A **primary interaction test** (click the main action, verify outcome)

---

## Navigation

- Cover every sidebar item with a click-and-verify test.
- Verify that navigating away deactivates the previous sidebar item.
- After each navigation, assert both the breadcrumb text AND the view's `active` CSS class.

---

## Text Field Boundaries

Test every visible text input with:
- Empty value (blank submit should be rejected or show validation)
- Single character
- Typical value (medium-length, ASCII)
- Max expected length (fill to the field's `maxlength` attribute if present)
- Special characters: `< > & " ' / \`

Engagement name field specifically: max 64 characters, empty should be rejected.

---

## Mock Data Strategy

- Use `page.addInitScript()` overrides before `page.goto('/')` — never patch after navigation.
- Keep mock data minimal: 1–2 items is enough to verify list rendering.
- Use realistic slugs and names (e.g. `alpha-scan`, `Alpha Scan`) — avoids false positives from case-insensitive matching.
- `list_prompts` and `list_scenarios` return `HashMap<category, entry[]>` — mock must use object format `{ category: [...] }`, not a flat array.

---

## Error States

- If a Tauri handler returns a rejected promise, the UI should show an error indicator, not a blank screen.
- At minimum, cover: `list_engagements` failing on the home view.

---

## Regression Guards

- Do not delete existing passing tests — only add or update.
- After writing new specs, always call `run_tests()` to confirm the suite is green.
- If a test targets a specific DOM ID, add a comment with the corresponding REQ-NNN so coverage is traceable.
