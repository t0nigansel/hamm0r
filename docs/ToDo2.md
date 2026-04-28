# Hamm0r — UX Restructuring ToDo

## Kontext

Aktuell sind die Konzepte **Workbench**, **Scenario**, **Library**, **Engagement** und **Target** in der UI gleichberechtigt nebeneinander, was zu Verwirrung führt. Es gibt keinen geführten Einstiegs-Flow für neue User. Die Workbench ist Default-Landing-Page, ist aber konzeptionell der Power-User-Modus.

## Konzeptmodell (verbindlich)

| Konzept | Bedeutung | Wiederverwendbar? |
|---|---|---|
| **Target** | *Was* angegriffen wird: Endpoint, Auth, Protokoll | Ja |
| **Library** | *Womit*: Prompt-Sammlungen (OWASP A01–A10, custom) | Ja |
| **Scenario** | *Wie*: Strategie-Template (Single-Shot, Multi-Turn, Mutation-Chain, Judge-Config) | Ja |
| **Engagement** | Eine konkrete Durchführung: `Target × Scenario × Library + Results + Timestamps` | Nein (Instanz) |
| **Workbench** | Free-Play-Modus für Power-User, einzelne Prompts gegen Target ohne Reporting-Overhead | — |

## Ziele

1. Neuer User kommt in **< 60 Sekunden** zum ersten Test-Ergebnis.
2. Konzepte sind sauber getrennt: Asset-Management (Sidebar) vs. Workflow (Wizard).
3. Bestehender Workbench-Modus bleibt für Power-User unverändert erreichbar.

---

## Tasks

### 1. Sidebar neu ordnen

Reihenfolge **von oben nach unten** im linken Menü:

1. Engagements
2. Targets
3. Libraries
4. Scenarios
5. Workbench
6. Settings

**Rationale:** Workflow-Einstiege oben, Konfigurations-Assets in der Mitte, Power-User-Tool unten.

### 2. Neue Landing-Page nach Login

Ersetzt den aktuellen leeren Workbench-State als Default.

- Zwei große Cards nebeneinander:
  - **"Start Engagement"** → öffnet Engagement-Wizard (siehe Task 3)
  - **"Open Workbench"** → öffnet den bestehenden Workbench-Modus
- Darunter: Liste der letzten 5 Engagements mit Status (Running, Done, Failed) und Quick-Resume.

### 3. Engagement-Wizard implementieren

Modaler oder full-page Wizard mit 4 Schritten. Wird durch `[+ New Engagement]` getriggert.

#### Schritt 1: Target

- Toggle: **Use Existing** | **Create New**
- Existing: Dropdown aus gespeicherten Targets.
- New: Inline-Form für `name`, `base_url`, `protocol` (openai_compat, anthropic, custom), `auth` (none, bearer, api_key, custom_header), `session_handling`.
- "Test connection" Button vor Weiter.

#### Schritt 2: Scenario

- Karten-Auswahl mit vordefinierten Templates:
  - `Quick Scan` — kleine OWASP-Stichprobe, single-shot, ~20 Prompts
  - `OWASP LLM Top 10 Full` — komplette Coverage A01–A10
  - `Prompt Injection Deep Dive` — A01-fokussiert mit Mutationen
  - `Jailbreak Battery` — kuratierte Jailbreak-Prompts
  - `Custom` — User wählt alles selbst
- Jede Karte zeigt: Anzahl Prompts (geschätzt), Laufzeit (geschätzt), OWASP-Coverage-Badges.

#### Schritt 3: Library

- Vorausgewählt basierend auf Scenario, editierbar.
- Multi-Select über A01–A10 + Custom-Libraries.
- Live-Counter: "X prompts selected, Y judged".

#### Schritt 4: Review & Fire

- Zusammenfassung: Target | Scenario | Library | geschätzte Laufzeit | geschätzte Kosten (falls API-Pricing bekannt).
- Prominenter `▶ Fire` Button.
- Optional: "Save as Template" Checkbox, um diese Kombi als neues Scenario abzulegen.

#### Wizard-State

- Wizard-State bleibt persistent, falls User abbricht (Resume möglich).
- Bei "Fire" wird ein Engagement-Record erzeugt und der User landet auf der Live-Results-View des neuen Engagements.

### 4. Workbench anpassen

- Workbench bleibt funktional erhalten.
- Empty-State der Workbench: statt direkter Konfiguration zuerst **"Pick Target"** als CTA.
- Header oben zeigt aktuelles Target als Chip (klickbar zum Wechseln), nicht als Dropdown direkt im Header-Flow.
- Hinweis-Banner im Empty-State: *"Looking for guided testing? Start an Engagement →"* als Link zum Wizard.

### 5. Engagement-Detail-View

Eigene Route `/engagements/:id` mit:

- Header: Target, Scenario, Status, Start/End-Time
- Tabs: `Results` (Diff/Signals/Raw/Judge wie aktuell), `Timeline` (Prompt-by-Prompt), `Report` (OWASP-Coverage-Heatmap, Export)
- Action: `Re-run`, `Export Report (MD/PDF)`, `Archive`

### 6. Migration / Backwards Compatibility

- Bestehende Workbench-Sessions müssen weiter funktionieren.
- Falls User aktuell ein Workbench-Setup hat, das ein Engagement sein sollte: Button **"Promote to Engagement"** in der Workbench, der die aktuelle Konfiguration als Engagement speichert.

---

## Out of Scope (für diesen PR)

- Multi-User / Team-Engagements
- Scheduled / Recurring Engagements
- API-Endpunkt-Änderungen am Backend (nur UI-Restructuring, keine Schema-Migration)

## Definition of Done

- [x] Sidebar in neuer Reihenfolge
- [x] Landing-Page mit zwei Cards + Recent Engagements
- [x] Engagement-Wizard 4-Schritte funktional
- [x] Mindestens 4 vordefinierte Scenario-Templates verfügbar
- [x] Workbench Empty-State angepasst
- [x] Engagement-Detail-View mit Tabs Results/Timeline/Report
- [x] "Promote to Engagement"-Button in Workbench
- [ ] Smoke-Test: Neuer User kommt vom Login in < 60s zum ersten Fire ohne Doku
