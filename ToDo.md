# TODO

## Status legend
- [ ] not started
- [~] in progress
- [x] done

---

## Milestone 1 — Data contract + prompt library (Week 1)

- [x] Create `db/schema.sql` with full schema from DATAMODEL.md
- [x] Create `db/repository.py` with all read/write functions
- [x] Write DB initialisation function (create tables if not exist)
- [x] Create `prompts/library.yaml` with starter set:
  - [x] 10 x A01 prompt injection (direct)
  - [x] 5 x A01 prompt injection (indirect/RAG)
  - [x] 5 x A06 data exfiltration
  - [x] 5 x A03 identity confusion
  - [x] 5 x A07 misleading content
  - [x] 5 x baseline/benign (control prompts)
- [x] Create `scripts/seed_prompts.py` — loads YAML into DB
- [x] Pydantic schema for prompt validation on load
- [x] Tests for repository functions (in-memory SQLite)

## Milestone 2 — Attack runner CLI (Week 2)

- [x] `runner/target_config.py` — TargetConfig dataclass, load/save JSON
- [x] `runner/http_client.py` — TargetAdapter base + OpenAICompatAdapter
- [x] `runner/http_client.py` — CustomRESTAdapter
- [x] `runner/attack_runner.py` — async runner, reads prompts, writes results
- [x] Graceful stop (SIGINT handler, flush in-flight results)
- [x] Progress callback (for later UI integration)
- [x] CLI entry point: `python -m runner --config target.json --db run.db`
- [x] Tests: mock HTTP responses, verify results written correctly

## Milestone 3 — Tauri UI (Week 3)

- [x] Scaffold Tauri project
- [x] Python sidecar integration (spawn Python, JSON over stdin/stdout)
- [x] UI: Target config panel (URL, endpoint type, auth)
- [x] UI: Prompt library browser (table with filter by category/OWASP)
- [x] UI: Add/edit prompt inline
- [x] UI: Import prompts from CSV
- [x] UI: Run config (tester name, concurrency, delay, output path)
- [x] UI: Attack/Stop button
- [x] UI: Live progress (progress bar, counters, last prompt/response snippet)
- [x] UI: Results table (browse completed results for current run)
- [x] New engagement flow: name + target → create .db file

## Milestone 4 — evaluat0r v0.1 (Week 4)

- [ ] Separate `evaluat0r/` module with its own pyproject.toml
- [ ] Ollama client wrapper (OpenAI-compat local endpoint)
- [ ] Heuristic pre-filter (regex patterns for obvious success/fail)
- [ ] Qwen judge: sends prompt+response, parses JSON verdict
- [ ] Write verdicts to DB
- [ ] Basic PDF report (HTML template → WeasyPrint)
  - [ ] Executive summary with overall risk score
  - [ ] Findings per OWASP category
  - [ ] Evidence table (SUCCESS verdicts with prompt + response)
- [ ] CLI: `python -m evaluat0r --db run.db --output report.pdf`

---

## Backlog (post-MVP)

- [ ] Multi-turn prompt support (mode: multiturn)
- [ ] CustomRESTAdapter field mapping UI
- [ ] Prompt library versioning (track which version was used in each run)
- [ ] Compare two runs against the same target (regression testing)
- [ ] Export prompt library to CSV
- [ ] Remediation recommendations per OWASP category in report
- [ ] Dark mode UI
- [ ] Signed release binaries (Windows + macOS + Linux)
- [ ] Expand prompt library to 200+ prompts
- [ ] A02 memory poisoning prompts (requires multi-turn)
- [ ] A04 privilege escalation prompts
- [ ] A08 supply chain / tool abuse prompts

---

## Decisions log

| Date | Decision | Reason |
|------|----------|--------|
| 2026-03 | SQLite over JSONL | Queryable, atomic writes, single file |
| 2026-03 | Tauri over Electron | Smaller binary, no Node runtime |
| 2026-03 | Qwen 2.5 local via Ollama | German data residency, no cloud |
| 2026-03 | Two separate modules | Clean boundary, evaluat0r is optional |
| 2026-03 | Plain HTML/JS UI | No bundler complexity for v0.1 |
| 2026-03 | uv for package management | Fast, reproducible, modern |