# hamm0r Requirements

Convention: each requirement uses `## REQ-NNN: Title`.
Agents derive test coverage from these entries.

---

## REQ-001: Navigation — sidebar views
The sidebar shall contain navigation items for all main views:
home, runs, targets, library, scenarios, and workbench.
Clicking a sidebar item switches the active view and updates the breadcrumb.

## REQ-002: Navigation — active state
The active sidebar item shall be visually marked (CSS class `active`).
Only one sidebar item is active at a time.

## REQ-003: Breadcrumb — view name
The topbar breadcrumb shall reflect the name of the currently active view.

## REQ-004: Breadcrumb — engagement name
When no engagement is open, the breadcrumb shall show "no engagement open".
When an engagement is open, it shall show the engagement's name.

## REQ-005: Engagement pill
The topbar pill (`#db-label`) shall show "no engagement" when no engagement is open.

## REQ-006: Home — recent engagements
The home view shall list recent engagements by name and creation date.
When no engagements exist, it shall show an empty-state message.

## REQ-007: Runs view — engagement cards
The runs view shall show one card per engagement.
When no engagements exist, it shall show an empty-state message.

## REQ-008: Topbar controls
The topbar shall always show a "New engagement" button and an "Open engagement" button.

## REQ-009: Library — prompt list
The library view shall display all prompts returned by the backend,
grouped by OWASP category.
When no prompts exist, it shall show a no-results message.

## REQ-010: Library — filter chips
The library view shall show OWASP filter chips.
An "ALL" chip selects all categories. Individual chips filter by OWASP ref.

## REQ-011: Library — search
The library view shall include a search box to filter prompts by text.

## REQ-012: Library — prompt count
The library view shall display the total number of prompts currently shown.

## REQ-013: Library — add prompt
The library view shall include an "Add prompt" button.
