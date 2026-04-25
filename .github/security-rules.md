# Security Review Rules

Rules the security-review agent uses to evaluate pull requests.

Each rule has:
- an ID (stable, referenced in findings)
- a severity (CRITICAL / HIGH / MEDIUM / LOW / INFO)
- a description (what to look for)
- a rationale (why it matters in this codebase)

The agent reports any violation it finds. It does not block merges.

---

## hamm0r-001 — No raw SQL outside repository layer
**Severity:** HIGH
**Look for:** Any file outside `db/repository.py` that contains SQL strings
(`SELECT`, `INSERT`, `UPDATE`, `DELETE`, `CREATE TABLE`). Ignore strings in
tests that mock DB responses, and ignore `db/schema.sql`.
**Rationale:** The repository layer is the single enforcement point for
parameterized queries. Raw SQL elsewhere means SQL injection risk and drift
from the schema contract.

## hamm0r-002 — Prompts and responses must never be logged in plaintext
**Severity:** HIGH
**Look for:** Log calls (`logger.info`, `logger.debug`, `print`, `log.write`)
whose arguments include a variable named `prompt`, `response`, `payload`,
`content`, `message`, or string interpolation of likely prompt/response
fields from the `results` table.
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

## hamm0r-005 — New `results` or `verdicts` schema changes require migration
**Severity:** HIGH
**Look for:** Changes to `db/schema.sql` that add, remove, or rename columns
in `results` or `verdicts` without a corresponding migration script in
`db/migrations/` and without an update to `Datamodel.md`.
**Rationale:** These tables are the handoff contract between promt0r and
evaluat0r. Breaking the contract silently breaks evaluat0r for existing DBs.

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

## hamm0r-008 — No new network endpoints without authentication note
**Severity:** HIGH
**Look for:** New routes in `sidecar/` (anything that defines an HTTP
endpoint) without a comment or docstring stating the auth model. The sidecar
binds to localhost, but the PR author must show they considered this.
**Rationale:** Sidecar endpoints have access to the engagement DB. Adding
them without auth thinking is how local-only tools eventually get accessed
by the wider machine.

## hamm0r-009 — httpx clients must have timeouts
**Severity:** MEDIUM
**Look for:** `httpx.AsyncClient()`, `httpx.Client()`, `httpx.get()`,
`httpx.post()` without `timeout=` parameter.
**Rationale:** A target under test may hang forever. A runner without
timeouts hangs with it, and blocks the engagement.

## hamm0r-010 — No broad except clauses in the runner
**Severity:** MEDIUM
**Look for:** `except:` or `except Exception:` in `runner/` without a
specific exception type, unless followed by `raise` or a logged re-raise.
**Rationale:** The runner must surface failures, not swallow them. A
swallowed exception means an attack silently "succeeded" with no verdict.

## hamm0r-011 — Async functions must actually await something
**Severity:** LOW
**Look for:** `async def` functions that contain no `await` statement.
**Rationale:** These either should be `def`, or they are missing an `await`
on an internal call. Either way the author did not mean what they wrote.

## hamm0r-012 — No hardcoded filesystem paths outside `paths.py`
**Severity:** LOW
**Look for:** String literals that look like absolute paths (`/tmp/...`,
`/Users/...`, `C:\...`) or relative paths to runtime artifacts
(`./artifacts/...`, `./default_engagement.db`) outside a central paths
module.
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
interpolation with non-constant values. Even in `db/repository.py`, queries
must use parameter binding (`?` placeholders).

## py-sec-004 — pickle / marshal on untrusted input
**Severity:** CRITICAL
**Look for:** `pickle.load`, `pickle.loads`, `marshal.load`, `marshal.loads`
on data that originates from files, network, or the DB.

## py-sec-005 — YAML unsafe load
**Severity:** HIGH
**Look for:** `yaml.load(...)` without `Loader=yaml.SafeLoader` or
`yaml.Loader`. Should be `yaml.safe_load()`.
