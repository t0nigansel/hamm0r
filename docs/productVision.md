# hamm0r — Product Vision

_One-pager. North star. If a decision is hard, this page decides it._

## In one sentence

**hamm0r is a local-first desktop tool that lets pentesters and security
consultants test LLM-based systems for OWASP-style attacks in five
minutes, with no cloud account required for the core workflow and no
config files in the standard user journey.**

## Who it is for

The primary user is a **pentester or freelance security consultant**
who gets called in to assess an LLM-backed system (chatbot, RAG,
agent integration) and needs results in the same engagement, not
next sprint. They know HTTP, they know OWASP, they know their target.
They bring hamm0r into their toolkit next to Burp and nuclei.

Secondary users: in-house security testers running pre-release checks,
QA teams validating prompts before deployment.

Not the user: ML researchers benchmarking models, prompt engineers A/B-
testing outputs, data scientists evaluating model quality. These people
have Promptfoo. hamm0r is not for them.

## The job the user is hiring hamm0r to do

> "I have a target URL, some credentials, and 30 minutes. Show me whether
> this LLM system is safe against prompt injection and the rest of the
> OWASP LLM Top 10 — and give me a report I can hand to my client."

Not: "let me craft the perfect test suite." Not: "let me compare seven
models." Just: fire, wait, report.

## The five-minute promise

From download to first usable report: under five minutes.

1. Download hamm0r (small installer, no dependencies to resolve)
2. Open the app (no account, no login, no cloud setup)
3. Paste target URL, choose auth type, paste credentials
4. Build a request with `{{prompt}}` as the payload placeholder
5. Pick a scenario — or fire directly from the request
6. Wait while hamm0r runs the attack library against the target
7. Get a report

If step 7 takes longer than five minutes on a standard workload, we
have failed the promise. That is the benchmark.

## Two modes, one tool

**Default mode: hamm0r core.** Runs attacks, records responses. No
analyzer, no LLM runtime, no local model. The raw evidence is in the
engagement database. The user can inspect, export, and interpret it
manually. This mode exists because not every run needs automated
analysis — sometimes you just want the raw data.

**Opt-in mode: analyz0r.** The user activates it once and chooses
between a local LLM (downloaded on demand) or a configured remote
endpoint. The analyzer reviews recorded responses and produces verdicts
and a structured report mapped to OWASP LLM Top 10. Note: local models
in the consumer range currently produce noticeably weaker verdicts than
hosted frontier models — the user picks the trade-off.

The reason for two modes: **the tool is useful before the user commits
to a local model download or a cloud key.** Activation is an earned
step, not a gate.

## Mental model

hamm0r has four objects. They map one-to-one to the sidebar:

- **Engagement** — the workspace. Holds runs, results, reports.
  This is where the user *does* things.
- **Request** — a single HTTP call template with `{{prompt}}` as
  payload placeholder. Can be fired standalone for debugging.
- **Scenario** — a Cartesian product: requests × prompt-library subset
  × repeat count. The unit of work. Built here, **executed in an
  engagement.** A scenario may run as a single session or as multiple
  parallel sessions (see *Multi-session testing* below).
- **Library** — curated and user-extensible prompt collections,
  organized by OWASP LLM Top 10 and category.

## Multi-session testing

A scenario can fire across N parallel sessions with distinct session
identifiers (cookies, conversation IDs, headers). This exposes a class
of attacks that single-session tools miss:

- **Cross-session data leakage** — session A plants a canary, session
  B probes for it (OWASP LLM02).
- **Tenant isolation failures** — does user-scoped data bleed across
  conversations?
- **Persistent memory leakage** — does the LLM's memory feature carry
  information from one user to another?

Hamm0r generates canary tokens, distributes prompts across sessions in
plant/probe phases, and scans all responses for leaks. This is a first-
class scenario type, not an afterthought.

## Prompt mutation

Static prompt libraries get filtered fast. Hamm0r mutates seed prompts
locally and deterministically to expand coverage without bloating the
library:

- **Encoding:** Base64, ROT13, hex, URL-encoding, Unicode homoglyphs
- **Obfuscation:** whitespace injection, zero-width characters, leetspeak
- **Linguistic:** synonym substitution, translation roundtrips,
  politeness variants
- **Structural:** code-block wrapping, JSON embedding, Markdown comments,
  prefix injections
- **Persona:** authority framing, role-play prefixes, jailbreak templates

Mutations are opt-in per scenario, run locally without an LLM, and are
reproducible. The report shows which mutation cracked the filter, not
just which seed.

The separation between building (Scenario) and running (Engagement)
is deliberate. One screen for configuration, another for execution
and evidence. No screen does both jobs.

## Pentester workflow

A report is not the end. Pentesters work *with* findings. Hamm0r
supports the loop:

- **Triage** — each finding can be marked confirmed, false-positive,
  or needs-review, with a free-text note. The report stays a working
  document until export.
- **Replay** — any single attempt in the report can be re-fired with
  one click, optionally with a tweaked prompt. No need to rebuild the
  scenario. This matches how pentesters actually iterate: "interesting
  response, let me try a small variation."
- **Per-request repeat counts** — a scenario's global repeat
  multiplies with per-request overrides. Login fires once per
  scenario iteration, chat fires five. At repeat 10 globally: login
  10×, chat 50×.

## What hamm0r explicitly is not

- **Not a framework.** No YAML configuration for the main workflow.
  No plugin system on day one. Clicks, not files.
- **Not a wizard-driven app.** A single-screen guided start for first-
  time users is acceptable; multi-step wizards forcing the standard
  workflow are not. The user always sees the full UI and can deviate.
- **Not a benchmark tool.** hamm0r does not rank models. It tests a
  target against an attack library.
- **Not a cloud service.** There is no hosted hamm0r product planned.
  The core workflow is local-first; hosted judging is an opt-in
  analyzer option, not the product model.
- **Not a replacement for manual review.** The analyzer produces
  findings. A human confirms them. The report is a starting point,
  not a verdict.
- **Not a competitor to Promptfoo, Garak, or PyRIT in scope.** Those
  tools are broader and more configurable. hamm0r trades breadth for
  speed-to-first-result and a tighter UX. The author uses hamm0r in
  place of them because it does what he actually needs — that is the
  bar.

## Wedges against the obvious alternatives

| If the user currently uses…           | hamm0r's wedge                                                                |
|---------------------------------------|-------------------------------------------------------------------------------|
| Promptfoo / Garak / PyRIT             | Zero-setup. No cloud keys. No YAML. Security-focused defaults. Desktop UI.    |
| Manual Burp-style probing             | Curated OWASP attack library. Structured report out of the box.               |
| Nothing ("we'll add tests later")     | Five minutes from zero to report. No excuse left.                             |
| Cloud-hosted AI security platforms    | No data leaves the machine. No procurement cycle.                             |

## Core product principles

These are the rules for decisions when the answer is not obvious.
They trump features, performance, and developer convenience.

1. **Local beats clever.** If a feature requires a cloud call, it
   does not ship in the default core mode.
2. **Click beats config.** If the standard workflow requires editing
   a YAML file, we have failed.
3. **No wizards.** A single-screen guided start is fine. Forcing the
   user through a multi-step funnel for the standard workflow is not.
   The user always has the full UI in reach.
4. **One screen, one job.** Building a scenario and running it are
   different jobs. They live on different screens. No screen tries
   to do both.
5. **Report beats dashboard.** The output is a shareable artifact
   (PDF, Markdown), not a session you have to stay logged into.
6. **Narrow and deep beats broad and shallow.** Curated default
   libraries beat a sprawling unmaintained one. Users may extend the
   library; the defaults stay opinionated.
7. **Transparent beats magical.** The user can always see what request
   hamm0r sent and what response came back. No hidden wrapping.
8. **Installable beats open-source purity.** We ship a binary. Users
   do not compile.

## UI conventions

- **Sidebar = nouns.** Home, Engagements, Requests, Scenarios, Library,
  Settings. One entry per object type, nothing else.
- **Top bar = context + active run.** Breadcrumb on the left. When a
  run is active, a compact progress bar (`[===.       ] 3/10`) sits
  in the top bar and can be expanded. No `+`, no global `▶`, no empty
  help button. Actions live where their object lives.
- **Fire from where it belongs.** A Request fires standalone (for
  debugging) from the Request screen. A Scenario fires only inside
  an Engagement. There is no global "fire something" button.

## Success signals

We know hamm0r is on the right path when:

- A first-time user runs a complete test and gets a report in the
  same sitting.
- A pentester brings hamm0r into their engagement toolkit next to
  Burp and nuclei.
- Someone writes "I used hamm0r to find a prompt injection in
  production" on LinkedIn without being asked.
- A competitor (Promptfoo included) adopts one of our UX patterns.

## What we ignore, for now

- Multi-user collaboration
- Cloud sync of engagements
- Plugin/extension marketplace
- Non-English UI
- Mobile companion apps
- Integrations with Jira, Slack, or anything enterprise-glue

These are not bad ideas. They are not this year's ideas.

---

_When something in this repo contradicts this document, this document
wins. Update the repo. If this document itself feels wrong, update it
deliberately — and note what changed, because we will want to know
later what we used to believe.