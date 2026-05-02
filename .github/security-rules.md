# Security Review Rules

Rules the security-review agent uses to evaluate pull requests.

Each rule has:
- an ID (stable, referenced in findings)
- a severity (CRITICAL / HIGH / MEDIUM / LOW / INFO)
- a description (what to look for)
- a rationale (why it matters in this codebase)

The agent reports any violation it finds. It does not block merges.

---

## hamm0r-001 — No direct artifact I/O outside storage crate
**Severity:** HIGH
**Look for:** New reads or writes of prompts, requests, engagements, run logs,
verdict logs, or response files outside `crates/storage/`.
**Rationale:** The storage crate is the single enforcement point for path
resolution, append-only semantics, masking, and atomic writes.

## hamm0r-002 — Prompts and responses must never be logged in plaintext
**Severity:** HIGH
**Look for:** Log calls (`tracing`, `log`, `println!`, `eprintln!`, JS
`console.*`) whose arguments include prompt text, payload text, raw response
bodies, extracted content, or similarly sensitive fields.
**Rationale:** hamm0r is a security tool. Attack prompts and their responses
are often sensitive (credentials extracted from target systems, internal
instructions). Logs leave the trust boundary.

## hamm0r-003 — No cloud calls from the runner
**Severity:** CRITICAL
**Look for:** HTTP(S) calls in `runner/` to anything other than `localhost`,
`127.0.0.1`, `0.0.0.0`, or a URL from a user-provided configuration object.
Specifically flag hardcoded domains (`api.openai.com`, `api.anthropic.com`,
any `.com`/`.io`/`.ai` URL).
**Rationale:** Data residency is a product feature. Prompts may contain the
user's test data or real system information. Silent cloud calls break that
contract.

## hamm0r-004 — No secrets in code or YAML
**Severity:** CRITICAL
**Look for:** String literals matching common secret patterns:
API keys (`sk-...`, `Bearer ...`), tokens longer than 32 chars that look
random, passwords in config files, `.env`-style `KEY=value` lines in any
committed file.
**Rationale:** Secrets in the repo leak the moment the repo goes public
(even if only to a contractor). No exceptions.

## hamm0r-005 — Run or verdict schema changes require contract updates
**Severity:** HIGH
**Look for:** Changes to `RunRecord`, run JSONL fields, verdict JSONL fields,
or report inputs without a corresponding update to `docs/Datamodel.md` and an
explicit migration/backward-compatibility note.
**Rationale:** The run and verdict JSONL files are the handoff contract
between core and analyzer. Silent drift breaks existing engagements.

## hamm0r-006 — No eval/exec/os.system
**Severity:** CRITICAL
**Look for:** Usage of `eval()`, `exec()`, `os.system()`, `compile()` with
dynamic input, or `subprocess.run(..., shell=True)` with non-constant
arguments.
**Rationale:** hamm0r processes attacker-controlled input (that is the whole
point). Dynamic execution of strings that touch attack prompts is a direct
RCE path.

## hamm0r-007 — Subprocess calls must use explicit argument lists
**Severity:** MEDIUM
**Look for:** `subprocess.run(cmd)`, `subprocess.Popen(cmd)` where `cmd` is a
single string rather than a list. Also `shell=True` anywhere.
**Rationale:** Shell strings with interpolation are injection vulnerabilities.
Lists of arguments cannot be reinterpreted by a shell.

## hamm0r-008 — No new local server surface without an explicit decision
**Severity:** HIGH
**Look for:** New HTTP listeners, websocket listeners, or ad-hoc IPC servers
in the app runtime without a documented architectural decision.
**Rationale:** hamm0r is deliberately a single-process desktop app. Adding a
server surface changes the trust boundary and packaging model.

## hamm0r-009 — reqwest clients must have explicit timeouts
**Severity:** MEDIUM
**Look for:** `reqwest::Client::builder()` or request paths in the runner
without explicit connect/request timeout configuration.
**Rationale:** A target under test may hang forever. A runner without
timeouts hangs with it, and blocks the engagement.

## hamm0r-010 — Runner errors must not be silently swallowed
**Severity:** MEDIUM
**Look for:** Catch-all error handling in `crates/runner/` that drops context,
returns success on failure, or suppresses write/reporting of failed attempts.
**Rationale:** The runner must surface failures, not swallow them. A
swallowed exception means an attack silently "succeeded" with no verdict.

## hamm0r-011 — Async functions must actually await something
**Severity:** LOW
**Look for:** `async fn` functions that contain no `.await` or obviously
defer all work synchronously without reason.
**Rationale:** These either should be `def`, or they are missing an `await`
on an internal call. Either way the author did not mean what they wrote.

## hamm0r-012 — No hardcoded filesystem paths outside path helpers
**Severity:** LOW
**Look for:** String literals that look like absolute paths (`/tmp/...`,
`/Users/...`, `C:\...`) or direct relative paths to runtime artifacts outside
the storage/path helper layer.
**Rationale:** Hardcoded paths break on other machines and in CI. Centralize
them.

---

## Generic Python rules

These are standard but worth naming so findings are unambiguous.

## py-sec-001 — Hardcoded credentials
**Severity:** CRITICAL
**Look for:** Any variable assignment where the name suggests a secret
(`password`, `token`, `api_key`, `secret`, `auth`) and the value is a
non-empty string literal.

## py-sec-002 — Use of weak cryptography
**Severity:** HIGH
**Look for:** `hashlib.md5(...)`, `hashlib.sha1(...)`, `random.random()` for
security-relevant contexts (token generation, IDs that must be
unguessable).

## py-sec-003 — SQL string formatting
**Severity:** CRITICAL
**Look for:** SQL queries built with f-strings, `.format()`, or `%`
interpolation with non-constant values. Where SQL exists in repo tooling, it
must use parameter binding instead of string interpolation.

## py-sec-004 — pickle / marshal on untrusted input
**Severity:** CRITICAL
**Look for:** `pickle.load`, `pickle.loads`, `marshal.load`, `marshal.loads`
on data that originates from files, network, or the DB.

## py-sec-005 — YAML unsafe load
**Severity:** HIGH
**Look for:** `yaml.load(...)` without `Loader=yaml.SafeLoader` or
`yaml.Loader`. Should be `yaml.safe_load()`.
