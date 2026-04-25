# hamm0r — Product Vision

_One-pager. North star. If a decision is hard, this page decides it._

## In one sentence

**hamm0r is a local-first desktop tool that lets security engineers test
LLM-based systems for OWASP-style attacks in five minutes, with no cloud
account and no config files.**

## Who it is for

The primary user is a **security tester or test engineer in an enterprise
setting** who needs to check whether a new LLM-backed feature (chatbot,
RAG system, agent integration) is safe to ship. They know HTTP, they know
OWASP, they know their target. They do not want to spend a day learning a
testing framework.

Secondary users: pentesters doing ad-hoc LLM assessments, developers
validating their own prompts, QA teams running regression suites before
releases.

Not the user: ML researchers benchmarking models, prompt engineers A/B-
testing outputs, data scientists evaluating model quality. These people
have Promptfoo. hamm0r is not for them.

## The job the user is hiring hamm0r to do

> "I have a target URL, some credentials, and 30 minutes. Show me whether
> this LLM system is safe against prompt injection and the rest of the
> OWASP LLM Top 10 — and give me a report I can hand to my CISO."

Not: "let me craft the perfect test suite." Not: "let me compare seven
models." Just: fire, wait, report.

## The five-minute promise

From download to first usable report: under five minutes.

1. Download hamm0r (small installer, no dependencies to resolve)
2. Open the app (no account, no login, no cloud setup)
3. Paste target URL, choose auth type, paste credentials
4. Build a request with `{{prompt}}` as the payload placeholder
5. Click FIRE
6. Wait while hamm0r runs the attack library against the target
7. Get a report

If step 7 takes longer than five minutes on a standard workload, we have
failed the promise. That is the benchmark.

## Two modes, one tool

**Default mode: hamm0r core.** Runs attacks, records responses. No
analyzer, no LLM runtime, no local model. The raw evidence is in the
engagement database. The user can inspect, export, and interpret it
manually. This mode exists because not every run needs automated
analysis — sometimes you just want the raw data.

**Opt-in mode: analyz0r.** The user activates it once. An embedded LLM
(Qwen, Gemma, DeepSeek — whatever is best at the time) is downloaded and
runs locally. It reviews the recorded responses and produces verdicts and
a structured report mapped to OWASP LLM Top 10. The analyzer never
appears by default, never phones home, never leaves the machine.

The reason for two modes: **the tool is useful before the user commits
to a 2 GB download.** Activation is an earned step, not a gate.

## What hamm0r explicitly is not

- **Not a framework.** No YAML configuration for the main workflow. No
  plugin system on day one. Clicks, not files.
- **Not a benchmark tool.** hamm0r does not rank models. It tests a
  target against an attack library.
- **Not a cloud service.** There is no hosted version planned. Data
  residency is the product.
- **Not a replacement for manual review.** The analyzer produces
  findings. A human confirms them. The report is a starting point, not
  a verdict.
- **Not a Promptfoo.** Promptfoo is broader and more configurable. If
  the user needs that breadth, they should use Promptfoo. hamm0r trades
  breadth for speed to first result.

## Wedges against the obvious alternatives

| If the user currently uses… | hamm0r's wedge |
|---|---|
| Promptfoo | Zero-setup. No cloud keys. No Ollama install. Security-focused defaults. |
| Manual Burp-style probing | Curated OWASP attack library. Structured report out of the box. |
| Nothing ("we'll add tests later") | Five minutes from zero to report. No excuse left. |
| Cloud-hosted AI security platforms | No data leaves the machine. No procurement cycle. |

## Core product principles

These are the rules for decisions when the answer is not obvious. They
trump features, performance, and developer convenience.

1. **Local beats clever.** If a feature requires a cloud call, it does
   not ship in the default mode.
2. **Click beats config.** If the standard workflow requires editing a
   YAML file, we have failed.
3. **Report beats dashboard.** The output is a shareable artifact (PDF,
   Markdown), not a session you have to stay logged into.
4. **Narrow and deep beats broad and shallow.** One attack library, well
   curated, beats ten half-maintained ones.
5. **Transparent beats magical.** The user can always see what request
   hamm0r sent and what response came back. No hidden wrapping.
6. **Installable beats open-source purity.** We ship a binary. Users do
   not compile.

## Success signals

We know hamm0r is on the right path when:

- A first-time user runs a complete test and gets a report in the same
  sitting.
- A security tester brings hamm0r into their engagement toolkit next to
  Burp and nuclei.
- Someone writes "I used hamm0r to find a prompt injection in production"
  on LinkedIn without being asked.
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
deliberately — and note what changed, because we will want to know later
what we used to believe._