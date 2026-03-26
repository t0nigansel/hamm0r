# Data Model

## Critical rule

The `results` table is the contract between promt0r and evaluat0r.
Never change its schema without coordinating both modules.
All other tables are owned exclusively by one module.

## Full schema

```sql
-- OWNED BY promt0r

CREATE TABLE prompts (
    id          TEXT PRIMARY KEY,          -- e.g. "PI-001", "ME-003"
    text        TEXT NOT NULL,             -- the attack prompt string
    category    TEXT NOT NULL,             -- see Categories below
    owasp_ref   TEXT NOT NULL,             -- e.g. "A01", "A02"
    severity    TEXT NOT NULL,             -- LOW | MEDIUM | HIGH | CRITICAL
    tags        TEXT,                      -- JSON array of strings, e.g. ["rag","indirect"]
    mode        TEXT NOT NULL DEFAULT 'single',  -- single | multiturn
    turns       TEXT,                      -- JSON array of messages if mode=multiturn
    source      TEXT,                      -- where this prompt came from
    created_at  TEXT NOT NULL,             -- ISO 8601
    updated_at  TEXT NOT NULL
);

CREATE TABLE targets (
    id              TEXT PRIMARY KEY,      -- UUID
    name            TEXT NOT NULL,         -- human label, e.g. "Acme HR Chatbot"
    url             TEXT NOT NULL,
    endpoint_type   TEXT NOT NULL,         -- openai_compat | custom_rest | raw_http
    auth_type       TEXT NOT NULL DEFAULT 'none',  -- none | bearer | api_key | basic
    auth_header     TEXT,                  -- header name, e.g. "Authorization"
    field_mapping   TEXT,                  -- JSON: {"message": "input", "response": "output"}
    system_prompt   TEXT,                  -- optional override to test against
    notes           TEXT,
    created_at      TEXT NOT NULL
);

CREATE TABLE runs (
    id              TEXT PRIMARY KEY,      -- UUID
    target_id       TEXT NOT NULL REFERENCES targets(id),
    tester_name     TEXT NOT NULL,
    prompt_set_ids  TEXT NOT NULL,         -- JSON array of prompt IDs used
    concurrency     INTEGER NOT NULL DEFAULT 1,
    delay_ms        INTEGER NOT NULL DEFAULT 0,
    status          TEXT NOT NULL,         -- running | completed | stopped | error
    started_at      TEXT NOT NULL,
    finished_at     TEXT,
    total_prompts   INTEGER,
    completed       INTEGER NOT NULL DEFAULT 0,
    errors          INTEGER NOT NULL DEFAULT 0,
    notes           TEXT
);

-- CONTRACT TABLE — do not modify schema
CREATE TABLE results (
    id              TEXT PRIMARY KEY,      -- UUID
    run_id          TEXT NOT NULL REFERENCES runs(id),
    prompt_id       TEXT NOT NULL REFERENCES prompts(id),
    prompt_text     TEXT NOT NULL,         -- snapshot of prompt at time of run
    response_text   TEXT,                  -- null if request failed
    status_code     INTEGER,               -- HTTP status, null if connection error
    latency_ms      INTEGER,
    error_message   TEXT,                  -- null if request succeeded
    timestamp       TEXT NOT NULL          -- ISO 8601, when request was sent
);

-- OWNED BY evaluat0r — promt0r never writes here
CREATE TABLE verdicts (
    id              TEXT PRIMARY KEY,      -- UUID
    result_id       TEXT NOT NULL REFERENCES results(id),
    verdict         TEXT NOT NULL,         -- SUCCESS | FAIL | PARTIAL | UNCLEAR
    confidence      REAL NOT NULL,         -- 0.0 to 1.0
    reason          TEXT NOT NULL,         -- Qwen's explanation
    model_used      TEXT NOT NULL,         -- e.g. "qwen2.5:14b"
    evaluated_at    TEXT NOT NULL
);

-- Indexes
CREATE INDEX idx_results_run_id ON results(run_id);
CREATE INDEX idx_verdicts_result_id ON verdicts(result_id);
CREATE INDEX idx_prompts_category ON prompts(category);
CREATE INDEX idx_prompts_owasp ON prompts(owasp_ref);
```

## OWASP categories

| Code | Category | Description |
|------|----------|-------------|
| A01  | prompt_injection | Direct and indirect prompt injection |
| A02  | memory_poisoning | Corrupting agent memory or context |
| A03  | identity_confusion | Making agent misrepresent itself |
| A04  | privilege_escalation | Gaining unauthorised access via agent |
| A05  | excessive_agency | Triggering unintended agent actions |
| A06  | data_exfiltration | Extracting sensitive data via agent |
| A07  | misleading_content | Agent produces false/harmful output |
| A08  | supply_chain | Attacks via agent tools/plugins |
| A09  | misinformation | Systematic false information generation |
| A10  | unbounded_consumption | Resource exhaustion via agent |

## Severity levels

- **CRITICAL**: direct data exfiltration, full prompt override, RCE via tool
- **HIGH**: partial injection success, identity confusion, privilege boundary crossed
- **MEDIUM**: indirect influence on output, partial data leakage
- **LOW**: minor deviation from expected behaviour, informational

## Prompt ID convention

Format: `{OWASP_CODE}-{SEQUENCE}` e.g. `A01-001`, `A06-012`
Sequence is zero-padded to 3 digits.
IDs are stable — never reuse a retired ID.

## Repository pattern

All DB access through `db/repository.py`. Functions follow this pattern:

```python
# Read
def get_prompts_by_category(db: sqlite3.Connection, category: str) -> list[Prompt]: ...
def get_run(db: sqlite3.Connection, run_id: str) -> Run | None: ...

# Write — always use transactions
def create_result(db: sqlite3.Connection, result: Result) -> None: ...
def update_run_status(db: sqlite3.Connection, run_id: str, status: str) -> None: ...
```

Open DB connection once per session, pass it through.
Never open a new connection per query.