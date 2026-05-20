# hamm0r — Future Ideas

_Things we considered, did not commit to yet, and want to remember.
Not the roadmap. Not the vision. A holding pen._

When promoting an idea from this file to the roadmap or vision, delete
it from here and note the date in the commit message.

---

## Tool-call inspection (deferred)

When the target LLM supports function/tool calling, the response often
contains the invoked tool name and arguments (e.g. OpenAI `tool_calls`,
Anthropic `tool_use` blocks). Hamm0r could parse these and surface in
the report which prompts triggered which tool calls.

**Status:** partly addressable today via *Result Columns* on requests
(user can extract arbitrary JSONPath into report columns). A dedicated
tool-call view would be nicer but is not differentiating enough yet.

---

## Comparison runs between targets

Same scenario, two endpoints (staging vs. prod, GPT-4 vs. Claude),
side-by-side report. Strong sales aid for consultants in pitches.

**Status:** clear use case, but adds report complexity. Revisit once
the single-target report is mature.

---

## RAG poisoning / document-injection tests

Specialized scenarios that probe whether a RAG-backed target leaks
references to internal documents or can be steered to surface
adversarial content. OWASP LLM08.

**Status:** partly covered by existing prompt categories. A dedicated
RAG-probe scenario type may be worth it later.

---

## Cost tracking

Track estimated cost per attempt by parsing token usage from responses.
Useful for OWASP LLM10 (unbounded consumption) findings.

**Status:** depends on the endpoint exposing token counts. Not every
target does. Deferred until we see consistent demand.

---

## Rate-limit / controlled DoS testing

Recursive prompts, context-window flooding, Unicode tarpits — already
partly in the A10 library. A scenario type with explicit escalation
controls and circuit breakers would make this safer to run in
production assessments.

**Status:** prompts exist. Orchestration around them is the open work.

---

_When you add an idea here, write enough that future-you understands
what past-you meant. One paragraph minimum._
