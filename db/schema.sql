-- promt0r database schema
-- Source of truth: Datamodel.md
-- Applied by db/repository.py:init_db()

-- OWNED BY promt0r

CREATE TABLE IF NOT EXISTS prompts (
    id          TEXT PRIMARY KEY,          -- e.g. "A01-001", "A06-003"
    text        TEXT NOT NULL,             -- the attack prompt string
    category    TEXT NOT NULL,             -- see Datamodel.md Categories
    owasp_ref   TEXT NOT NULL,             -- e.g. "A01", "A02"
    severity    TEXT NOT NULL,             -- LOW | MEDIUM | HIGH | CRITICAL
    tags        TEXT,                      -- JSON array of strings
    mode        TEXT NOT NULL DEFAULT 'single',  -- single | multiturn
    turns       TEXT,                      -- JSON array of messages if mode=multiturn
    source      TEXT,                      -- where this prompt came from
    created_at  TEXT NOT NULL,             -- ISO 8601
    updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS targets (
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

CREATE TABLE IF NOT EXISTS runs (
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

-- CONTRACT TABLE — do not modify schema without coordinating promt0r + evaluat0r
CREATE TABLE IF NOT EXISTS results (
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
CREATE TABLE IF NOT EXISTS verdicts (
    id              TEXT PRIMARY KEY,      -- UUID
    result_id       TEXT NOT NULL REFERENCES results(id),
    verdict         TEXT NOT NULL,         -- SUCCESS | FAIL | PARTIAL | UNCLEAR
    confidence      REAL NOT NULL,         -- 0.0 to 1.0
    reason          TEXT NOT NULL,         -- Qwen's explanation
    model_used      TEXT NOT NULL,         -- e.g. "qwen2.5:14b"
    evaluated_at    TEXT NOT NULL
);

-- Indexes
CREATE INDEX IF NOT EXISTS idx_results_run_id ON results(run_id);
CREATE INDEX IF NOT EXISTS idx_verdicts_result_id ON verdicts(result_id);
CREATE INDEX IF NOT EXISTS idx_prompts_category ON prompts(category);
CREATE INDEX IF NOT EXISTS idx_prompts_owasp ON prompts(owasp_ref);
