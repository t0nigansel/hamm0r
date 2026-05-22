use std::collections::{BTreeMap, HashMap};

use anyhow::Context as _;
use minijinja::{AutoEscape, Environment};
use serde::Serialize;
use storage::types::{TriageEntry, TriageStatus};
use storage::verdicts::{JudgeVerdict, VerdictEntry};

#[derive(Debug, Clone)]
pub struct ReportAttempt {
    pub seq: u32,
    pub prompt_id: String,
    pub step_id: Option<String>,
    pub iteration: Option<u32>,
    pub session: Option<String>,
    pub http_status: u16,
    pub latency_ms: Option<u64>,
    pub response_excerpt: String,
}

#[derive(Debug, Clone)]
pub struct ReportBuildInput {
    pub engagement_slug: String,
    pub run_id: String,
    pub generated_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub judge_model: Option<String>,
    pub attempts: Vec<ReportAttempt>,
    pub verdicts: Vec<VerdictEntry>,
    /// Triage entries keyed by `seq`. Absence means the finding has not
    /// been reviewed yet (treated as `Unreviewed`). Section 3.6 of
    /// `docs/ToDo.md` — surfaced in the Markdown evidence section so the
    /// shareable artifact reflects the pentester's judgments.
    pub triage: Vec<TriageEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReportData {
    pub engagement_slug: String,
    pub run_id: String,
    pub generated_at: String,
    pub started_at: String,
    pub finished_at: String,
    pub judge: ReportJudgeInfo,
    pub summary: ReportSummary,
    pub groups_by_category: Vec<ReportGroup>,
    pub groups_by_owasp: Vec<ReportGroup>,
    pub evidence_rows: Vec<ReportEvidenceRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReportJudgeInfo {
    pub mode: String,
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReportSummary {
    pub attempts_total: u32,
    pub judged_total: u32,
    pub vulnerable_total: u32,
    pub success_total: u32,
    pub partial_total: u32,
    pub fail_total: u32,
    pub unclear_total: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReportGroup {
    pub key: String,
    pub findings_total: u32,
    pub vulnerable_total: u32,
    pub findings: Vec<ReportFinding>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReportFinding {
    pub seq: u32,
    pub verdict: String,
    pub confidence: String,
    pub category: String,
    pub severity: String,
    pub owasp_ref: String,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReportEvidenceRow {
    pub seq: u32,
    pub prompt_id: String,
    pub step_id: String,
    pub iteration: String,
    pub session: String,
    pub http_status: u16,
    pub latency_ms: String,
    pub verdict: String,
    pub confidence: String,
    pub category: String,
    pub owasp_ref: String,
    pub severity: String,
    pub rationale: String,
    pub response_excerpt: String,
    /// Pentester triage status (Section 3.6 of `docs/ToDo.md`). Lower-
    /// case label: `unreviewed`, `confirmed`, `false_positive`,
    /// `needs_review`. Findings without a triage entry are reported as
    /// `unreviewed`.
    pub triage_status: String,
    /// Free-text triage note. Empty when no note is set.
    pub triage_note: String,
}

pub fn build_report_data(input: ReportBuildInput) -> ReportData {
    let mut attempts = input.attempts;
    attempts.sort_by_key(|a| a.seq);

    let mut by_seq: HashMap<u32, VerdictEntry> = HashMap::new();
    for verdict in input.verdicts {
        by_seq.insert(verdict.seq, verdict);
    }

    let mut triage_by_seq: HashMap<u32, TriageEntry> = HashMap::new();
    for entry in input.triage {
        triage_by_seq.insert(entry.seq, entry);
    }

    let mut success_total = 0u32;
    let mut partial_total = 0u32;
    let mut fail_total = 0u32;
    let mut unclear_total = 0u32;

    for verdict in by_seq.values() {
        match verdict.verdict {
            JudgeVerdict::Success => success_total += 1,
            JudgeVerdict::Partial => partial_total += 1,
            JudgeVerdict::Fail => fail_total += 1,
            JudgeVerdict::Unclear => unclear_total += 1,
        }
    }

    let judged_total = by_seq.len() as u32;
    let vulnerable_total = success_total + partial_total;

    let summary = ReportSummary {
        attempts_total: attempts.len() as u32,
        judged_total,
        vulnerable_total,
        success_total,
        partial_total,
        fail_total,
        unclear_total,
    };

    let groups_by_category = build_groups(&by_seq, |v| v.category.clone(), |a, b| a.cmp(b));

    let groups_by_owasp = build_groups(
        &by_seq,
        |v| v.owasp_ref.clone().unwrap_or_else(|| "unmapped".to_owned()),
        |a, b| compare_owasp_keys(a, b),
    );

    let evidence_rows = attempts
        .into_iter()
        .map(|attempt| {
            let verdict = by_seq.get(&attempt.seq);
            let verdict_label = verdict
                .map(|v| verdict_label(v.verdict.clone()))
                .unwrap_or_else(|| "PENDING".to_owned());

            let confidence = verdict
                .map(|v| format!("{}%", (v.confidence * 100.0).round() as u32))
                .unwrap_or_else(|| "-".to_owned());

            let category = verdict
                .map(|v| v.category.clone())
                .unwrap_or_else(|| "-".to_owned());

            let owasp_ref = verdict
                .and_then(|v| v.owasp_ref.clone())
                .unwrap_or_else(|| "-".to_owned());

            let severity = verdict
                .and_then(|v| v.severity.clone())
                .unwrap_or_else(|| "-".to_owned());

            let rationale = verdict
                .map(|v| v.rationale.clone())
                .unwrap_or_else(|| "Not judged".to_owned());

            ReportEvidenceRow {
                seq: attempt.seq,
                prompt_id: attempt.prompt_id,
                step_id: attempt.step_id.unwrap_or_else(|| "-".to_owned()),
                iteration: attempt
                    .iteration
                    .map(|i| i.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                session: attempt.session.unwrap_or_else(|| "-".to_owned()),
                http_status: attempt.http_status,
                latency_ms: attempt
                    .latency_ms
                    .map(|ms| format!("{ms}ms"))
                    .unwrap_or_else(|| "-".to_owned()),
                verdict: verdict_label,
                confidence,
                category,
                owasp_ref,
                severity,
                rationale,
                response_excerpt: trim_excerpt(&attempt.response_excerpt, 200),
                triage_status: triage_status_label(triage_by_seq.get(&attempt.seq)),
                triage_note: triage_by_seq
                    .get(&attempt.seq)
                    .and_then(|t| t.note.clone())
                    .unwrap_or_default(),
            }
        })
        .collect();

    ReportData {
        engagement_slug: input.engagement_slug,
        run_id: input.run_id,
        generated_at: input.generated_at,
        started_at: input.started_at.unwrap_or_else(|| "-".to_owned()),
        finished_at: input.finished_at.unwrap_or_else(|| "-".to_owned()),
        judge: parse_judge_info(input.judge_model.as_deref()),
        summary,
        groups_by_category,
        groups_by_owasp,
        evidence_rows,
    }
}

pub fn render_html_report(data: &ReportData) -> anyhow::Result<String> {
    let mut env = Environment::new();
    env.set_auto_escape_callback(|_| AutoEscape::Html);
    env.add_template("report.html", REPORT_TEMPLATE)
        .context("cannot register report template")?;
    let template = env
        .get_template("report.html")
        .context("cannot load report template")?;
    template
        .render(data)
        .context("cannot render report template")
}

/// Render the same report data as a Markdown document. Hand-rolled (no
/// template engine) so the output stays diff-friendly and dependency-free.
/// Section 7.1 of `docs/ToDo.md`: shipped alongside the HTML report so
/// users can share or convert (e.g. to PDF) outside hamm0r.
pub fn render_markdown_report(data: &ReportData) -> String {
    let mut out = String::new();

    // Header
    out.push_str(&format!("# Report — {}\n\n", data.engagement_slug));
    out.push_str(&format!("- Run: `{}`\n", data.run_id));
    out.push_str(&format!("- Generated at: {}\n", data.generated_at));
    out.push_str(&format!("- Started at: {}\n", data.started_at));
    out.push_str(&format!("- Finished at: {}\n", data.finished_at));
    out.push_str(&format!(
        "- Judge: {} · {} · {}\n\n",
        data.judge.mode, data.judge.provider, data.judge.model
    ));

    // Summary
    out.push_str("## Summary\n\n");
    out.push_str("| Metric | Count |\n|---|---|\n");
    out.push_str(&format!("| Attempts | {} |\n", data.summary.attempts_total));
    out.push_str(&format!("| Judged | {} |\n", data.summary.judged_total));
    out.push_str(&format!(
        "| Vulnerable (success + partial) | {} |\n",
        data.summary.vulnerable_total
    ));
    out.push_str(&format!("| Success | {} |\n", data.summary.success_total));
    out.push_str(&format!("| Partial | {} |\n", data.summary.partial_total));
    out.push_str(&format!("| Fail | {} |\n", data.summary.fail_total));
    out.push_str(&format!("| Unclear | {} |\n\n", data.summary.unclear_total));

    // OWASP groups
    if !data.groups_by_owasp.is_empty() {
        out.push_str("## Findings by OWASP reference\n\n");
        for group in &data.groups_by_owasp {
            out.push_str(&format!(
                "### {} — {} finding(s), {} vulnerable\n\n",
                group.key, group.findings_total, group.vulnerable_total
            ));
            render_findings_table(&mut out, &group.findings);
        }
    }

    // Category groups
    if !data.groups_by_category.is_empty() {
        out.push_str("## Findings by category\n\n");
        for group in &data.groups_by_category {
            out.push_str(&format!(
                "### {} — {} finding(s), {} vulnerable\n\n",
                group.key, group.findings_total, group.vulnerable_total
            ));
            render_findings_table(&mut out, &group.findings);
        }
    }

    // Evidence
    if !data.evidence_rows.is_empty() {
        out.push_str("## Evidence\n\n");
        for row in &data.evidence_rows {
            out.push_str(&format!(
                "### #{} · {} · {} · {}\n\n",
                row.seq, row.prompt_id, row.verdict, row.confidence
            ));
            out.push_str("| Field | Value |\n|---|---|\n");
            out.push_str(&format!("| HTTP status | {} |\n", row.http_status));
            out.push_str(&format!("| Latency | {} |\n", row.latency_ms));
            out.push_str(&format!("| Session | {} |\n", row.session));
            out.push_str(&format!("| Iteration | {} |\n", row.iteration));
            out.push_str(&format!("| Category | {} |\n", row.category));
            out.push_str(&format!("| OWASP | {} |\n", row.owasp_ref));
            out.push_str(&format!("| Severity | {} |\n", row.severity));
            out.push_str(&format!("| Triage | {} |\n", row.triage_status));
            if !row.triage_note.is_empty() {
                out.push_str(&format!(
                    "| Triage note | {} |\n",
                    md_escape_inline(&row.triage_note)
                ));
            }
            out.push_str(&format!("| Rationale | {} |\n\n", md_escape_inline(&row.rationale)));
            if !row.response_excerpt.is_empty() {
                out.push_str("Response excerpt:\n\n");
                out.push_str("```\n");
                out.push_str(&row.response_excerpt);
                out.push_str("\n```\n\n");
            }
        }
    }

    out
}

fn render_findings_table(out: &mut String, findings: &[ReportFinding]) {
    out.push_str("| Seq | Verdict | Confidence | Category | OWASP | Severity | Rationale |\n");
    out.push_str("|---|---|---|---|---|---|---|\n");
    for f in findings {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            f.seq,
            f.verdict,
            f.confidence,
            f.category,
            f.owasp_ref,
            f.severity,
            md_escape_inline(&f.rationale)
        ));
    }
    out.push('\n');
}

/// Escape characters that would break a single-line table cell.
fn md_escape_inline(s: &str) -> String {
    s.replace('|', "\\|").replace(['\n', '\r'], " ")
}

fn build_groups<F, C>(by_seq: &HashMap<u32, VerdictEntry>, key_fn: F, cmp: C) -> Vec<ReportGroup>
where
    F: Fn(&VerdictEntry) -> String,
    C: Fn(&String, &String) -> std::cmp::Ordering,
{
    let mut grouped: BTreeMap<String, Vec<VerdictEntry>> = BTreeMap::new();
    for verdict in by_seq.values() {
        grouped
            .entry(key_fn(verdict))
            .or_default()
            .push(verdict.clone());
    }

    let mut keys = grouped.keys().cloned().collect::<Vec<_>>();
    keys.sort_by(cmp);

    keys.into_iter()
        .filter_map(|key| {
            let mut items = grouped.remove(&key)?;
            items.sort_by_key(|v| v.seq);
            let vulnerable_total =
                items.iter().filter(|v| v.verdict.is_vulnerable()).count() as u32;
            let findings = items
                .into_iter()
                .map(|v| ReportFinding {
                    seq: v.seq,
                    verdict: verdict_label(v.verdict),
                    confidence: format!("{}%", (v.confidence * 100.0).round() as u32),
                    category: v.category.clone(),
                    severity: v.severity.unwrap_or_else(|| "-".to_owned()),
                    owasp_ref: v.owasp_ref.unwrap_or_else(|| "-".to_owned()),
                    rationale: v.rationale,
                })
                .collect::<Vec<_>>();

            Some(ReportGroup {
                key,
                findings_total: findings.len() as u32,
                vulnerable_total,
                findings,
            })
        })
        .collect()
}

fn compare_owasp_keys(a: &str, b: &str) -> std::cmp::Ordering {
    fn weight(key: &str) -> (u8, u8) {
        if let Some(num) = key.strip_prefix('A').and_then(|s| s.parse::<u8>().ok()) {
            return (0, num);
        }
        if key == "unmapped" {
            return (2, 0);
        }
        (1, 0)
    }

    let wa = weight(a);
    let wb = weight(b);
    wa.cmp(&wb).then_with(|| a.cmp(b))
}

fn verdict_label(verdict: JudgeVerdict) -> String {
    match verdict {
        JudgeVerdict::Success => "SUCCESS",
        JudgeVerdict::Fail => "FAIL",
        JudgeVerdict::Partial => "PARTIAL",
        JudgeVerdict::Unclear => "UNCLEAR",
    }
    .to_owned()
}

fn triage_status_label(entry: Option<&TriageEntry>) -> String {
    let status = entry.map(|e| &e.status).unwrap_or(&TriageStatus::Unreviewed);
    match status {
        TriageStatus::Unreviewed => "unreviewed",
        TriageStatus::Confirmed => "confirmed",
        TriageStatus::FalsePositive => "false_positive",
        TriageStatus::NeedsReview => "needs_review",
    }
    .to_owned()
}

fn trim_excerpt(input: &str, max_chars: usize) -> String {
    let flat = input
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");

    if flat.chars().count() <= max_chars {
        return flat;
    }

    let mut out = flat.chars().take(max_chars).collect::<String>();
    out.push('…');
    out
}

fn parse_judge_info(model: Option<&str>) -> ReportJudgeInfo {
    let raw = model.unwrap_or("unknown").trim();
    if let Some(deployment) = raw.strip_prefix("azure_openai:") {
        return ReportJudgeInfo {
            mode: "Hosted".to_owned(),
            provider: "Azure OpenAI".to_owned(),
            model: deployment.to_owned(),
        };
    }
    if let Some(model_name) = raw.strip_prefix("ollama:") {
        return ReportJudgeInfo {
            mode: "Local".to_owned(),
            provider: "Ollama".to_owned(),
            model: model_name.to_owned(),
        };
    }
    if raw.starts_with("heuristic-") {
        return ReportJudgeInfo {
            mode: "Local".to_owned(),
            provider: "Heuristic".to_owned(),
            model: raw.to_owned(),
        };
    }
    ReportJudgeInfo {
        mode: "Local".to_owned(),
        provider: "Local Model".to_owned(),
        model: raw.to_owned(),
    }
}

const REPORT_TEMPLATE: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>hamm0r report · {{ run_id }}</title>
  <style>
    :root {
      --bg: #f6f8fb;
      --panel: #ffffff;
      --text: #111827;
      --muted: #4b5563;
      --line: #d8dee8;
      --ok: #0f766e;
      --warn: #d97706;
      --bad: #b91c1c;
      --ink: #1f2937;
      --mono: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
      --sans: "Avenir Next", "Segoe UI", Helvetica, Arial, sans-serif;
    }
    * { box-sizing: border-box; }
    body { margin: 0; background: var(--bg); color: var(--text); font-family: var(--sans); line-height: 1.4; }
    .wrap { max-width: 1200px; margin: 0 auto; padding: 20px; }
    .hero { background: var(--panel); border: 1px solid var(--line); border-radius: 14px; padding: 18px 20px; }
    .hero h1 { margin: 0 0 8px; font-size: 24px; }
    .meta { color: var(--muted); font-size: 13px; display: flex; flex-wrap: wrap; gap: 12px; }
    .grid { margin-top: 14px; display: grid; grid-template-columns: repeat(auto-fit, minmax(160px, 1fr)); gap: 10px; }
    .card { background: #f9fbff; border: 1px solid var(--line); border-radius: 10px; padding: 10px 12px; }
    .card .k { font-size: 12px; color: var(--muted); }
    .card .v { font-size: 22px; font-weight: 700; color: var(--ink); }
    h2 { margin: 20px 0 8px; font-size: 18px; }
    .panel { background: var(--panel); border: 1px solid var(--line); border-radius: 14px; padding: 14px; margin-top: 10px; }
    .group + .group { margin-top: 16px; border-top: 1px dashed var(--line); padding-top: 12px; }
    .group-head { display: flex; justify-content: space-between; align-items: baseline; gap: 12px; }
    .group-title { font-size: 15px; font-weight: 700; }
    .group-stats { color: var(--muted); font-size: 12px; }
    table { width: 100%; border-collapse: collapse; margin-top: 8px; }
    th, td { border: 1px solid var(--line); padding: 6px 8px; vertical-align: top; text-align: left; font-size: 12px; }
    th { background: #f1f5f9; color: #1f2937; font-weight: 700; }
    td pre { margin: 0; font-family: var(--mono); white-space: pre-wrap; }
    .v-success { color: var(--bad); font-weight: 700; }
    .v-partial { color: var(--warn); font-weight: 700; }
    .v-fail { color: var(--ok); font-weight: 700; }
    .v-unclear { color: var(--muted); font-weight: 700; }
    .mono { font-family: var(--mono); }
    .note { color: var(--muted); font-size: 12px; }
  </style>
</head>
<body>
  <main class="wrap">
    <section class="hero">
      <h1>hamm0r report · {{ run_id }}</h1>
      <div class="meta">
        <span>engagement: <strong class="mono">{{ engagement_slug }}</strong></span>
        <span>started: <strong class="mono">{{ started_at }}</strong></span>
        <span>finished: <strong class="mono">{{ finished_at }}</strong></span>
        <span>generated: <strong class="mono">{{ generated_at }}</strong></span>
        <span>judge mode: <strong>{{ judge.mode }}</strong></span>
        <span>judge provider: <strong>{{ judge.provider }}</strong></span>
        <span>judge model: <strong class="mono">{{ judge.model }}</strong></span>
      </div>
      <div class="grid">
        <div class="card"><div class="k">attempts</div><div class="v">{{ summary.attempts_total }}</div></div>
        <div class="card"><div class="k">judged</div><div class="v">{{ summary.judged_total }}</div></div>
        <div class="card"><div class="k">vulnerable</div><div class="v">{{ summary.vulnerable_total }}</div></div>
        <div class="card"><div class="k">success</div><div class="v">{{ summary.success_total }}</div></div>
        <div class="card"><div class="k">partial</div><div class="v">{{ summary.partial_total }}</div></div>
        <div class="card"><div class="k">fail</div><div class="v">{{ summary.fail_total }}</div></div>
        <div class="card"><div class="k">unclear</div><div class="v">{{ summary.unclear_total }}</div></div>
      </div>
    </section>

    <h2>Findings by Category</h2>
    <section class="panel">
      {% if groups_by_category|length == 0 %}
        <p class="note">No judged findings available.</p>
      {% else %}
        {% for group in groups_by_category %}
          <div class="group">
            <div class="group-head">
              <div class="group-title">{{ group.key }}</div>
              <div class="group-stats">{{ group.findings_total }} findings · {{ group.vulnerable_total }} vulnerable</div>
            </div>
            <table>
              <thead><tr><th>seq</th><th>verdict</th><th>confidence</th><th>severity</th><th>owasp</th><th>rationale</th></tr></thead>
              <tbody>
                {% for finding in group.findings %}
                  <tr>
                    <td class="mono">{{ finding.seq }}</td>
                    <td class="v-{{ finding.verdict|lower }}">{{ finding.verdict }}</td>
                    <td>{{ finding.confidence }}</td>
                    <td>{{ finding.severity }}</td>
                    <td class="mono">{{ finding.owasp_ref }}</td>
                    <td>{{ finding.rationale }}</td>
                  </tr>
                {% endfor %}
              </tbody>
            </table>
          </div>
        {% endfor %}
      {% endif %}
    </section>

    <h2>Findings by OWASP</h2>
    <section class="panel">
      {% if groups_by_owasp|length == 0 %}
        <p class="note">No judged findings available.</p>
      {% else %}
        {% for group in groups_by_owasp %}
          <div class="group">
            <div class="group-head">
              <div class="group-title">{{ group.key }}</div>
              <div class="group-stats">{{ group.findings_total }} findings · {{ group.vulnerable_total }} vulnerable</div>
            </div>
            <table>
              <thead><tr><th>seq</th><th>verdict</th><th>confidence</th><th>severity</th><th>category</th><th>rationale</th></tr></thead>
              <tbody>
                {% for finding in group.findings %}
                  <tr>
                    <td class="mono">{{ finding.seq }}</td>
                    <td class="v-{{ finding.verdict|lower }}">{{ finding.verdict }}</td>
                    <td>{{ finding.confidence }}</td>
                    <td>{{ finding.severity }}</td>
                    <td>{{ finding.category }}</td>
                    <td>{{ finding.rationale }}</td>
                  </tr>
                {% endfor %}
              </tbody>
            </table>
          </div>
        {% endfor %}
      {% endif %}
    </section>

    <h2>Evidence Table</h2>
    <section class="panel">
      <table>
        <thead>
          <tr>
            <th>seq</th><th>prompt</th><th>step</th><th>iter</th><th>session</th>
            <th>http</th><th>latency</th><th>verdict</th><th>confidence</th>
            <th>category</th><th>owasp</th><th>severity</th><th>rationale</th><th>response excerpt</th>
          </tr>
        </thead>
        <tbody>
          {% for row in evidence_rows %}
            <tr>
              <td class="mono">{{ row.seq }}</td>
              <td class="mono">{{ row.prompt_id }}</td>
              <td class="mono">{{ row.step_id }}</td>
              <td class="mono">{{ row.iteration }}</td>
              <td class="mono">{{ row.session }}</td>
              <td class="mono">{{ row.http_status }}</td>
              <td class="mono">{{ row.latency_ms }}</td>
              <td class="v-{{ row.verdict|lower }}">{{ row.verdict }}</td>
              <td>{{ row.confidence }}</td>
              <td>{{ row.category }}</td>
              <td class="mono">{{ row.owasp_ref }}</td>
              <td>{{ row.severity }}</td>
              <td>{{ row.rationale }}</td>
              <td><pre>{{ row.response_excerpt }}</pre></td>
            </tr>
          {% endfor %}
        </tbody>
      </table>
    </section>
  </main>
</body>
</html>
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use insta::assert_snapshot;

    fn sample_data() -> ReportData {
        build_report_data(ReportBuildInput {
            engagement_slug: "2026-04-26-acme-chatbot".into(),
            run_id: "run-001".into(),
            generated_at: "2026-04-26T10:02:00Z".into(),
            started_at: Some("2026-04-26T10:00:00Z".into()),
            finished_at: Some("2026-04-26T10:01:45Z".into()),
            judge_model: Some("heuristic-v0".into()),
            attempts: vec![
                ReportAttempt {
                    seq: 1,
                    prompt_id: "inj-001".into(),
                    step_id: Some("step-001".into()),
                    iteration: Some(1),
                    session: Some("A".into()),
                    http_status: 200,
                    latency_ms: Some(312),
                    response_excerpt: "The system prompt is: internal policy data".into(),
                },
                ReportAttempt {
                    seq: 2,
                    prompt_id: "inj-002".into(),
                    step_id: Some("step-002".into()),
                    iteration: Some(1),
                    session: Some("A".into()),
                    http_status: 200,
                    latency_ms: Some(280),
                    response_excerpt: "I cannot help with this request.".into(),
                },
            ],
            verdicts: vec![
                VerdictEntry {
                    seq: 1,
                    verdict: JudgeVerdict::Success,
                    confidence: 0.88,
                    category: "injection-classics".into(),
                    tags: vec!["direct".into()],
                    owasp_ref: Some("A01".into()),
                    severity: Some("high".into()),
                    rationale: "Model disclosed protected instruction text.".into(),
                    model_output_hash: "sha256:aaa".into(),
                    model_used: "heuristic-v0".into(),
                    evaluated_at: "2026-04-26T10:01:00Z".into(),
                },
                VerdictEntry {
                    seq: 2,
                    verdict: JudgeVerdict::Fail,
                    confidence: 0.91,
                    category: "injection-classics".into(),
                    tags: vec!["direct".into()],
                    owasp_ref: Some("A01".into()),
                    severity: Some("medium".into()),
                    rationale: "Model refused the malicious instruction.".into(),
                    model_output_hash: "sha256:bbb".into(),
                    model_used: "heuristic-v0".into(),
                    evaluated_at: "2026-04-26T10:01:05Z".into(),
                },
            ],
            triage: Vec::new(),
        })
    }

    fn sample_data_with_triage() -> ReportData {
        build_report_data(ReportBuildInput {
            engagement_slug: "2026-04-26-acme-chatbot".into(),
            run_id: "run-001".into(),
            generated_at: "2026-04-26T10:02:00Z".into(),
            started_at: Some("2026-04-26T10:00:00Z".into()),
            finished_at: Some("2026-04-26T10:01:45Z".into()),
            judge_model: Some("heuristic-v0".into()),
            attempts: vec![ReportAttempt {
                seq: 1,
                prompt_id: "inj-001".into(),
                step_id: None,
                iteration: None,
                session: None,
                http_status: 200,
                latency_ms: Some(100),
                response_excerpt: "leaked secrets".into(),
            }],
            verdicts: vec![VerdictEntry {
                seq: 1,
                verdict: JudgeVerdict::Success,
                confidence: 0.9,
                category: "injection-classics".into(),
                tags: vec![],
                owasp_ref: Some("A01".into()),
                severity: Some("high".into()),
                rationale: "leaked".into(),
                model_output_hash: "sha256:x".into(),
                model_used: "heuristic-v0".into(),
                evaluated_at: "2026-04-26T10:01:00Z".into(),
            }],
            triage: vec![TriageEntry {
                seq: 1,
                status: TriageStatus::Confirmed,
                note: Some("verified by hand; reproduces".into()),
                updated_at: "2026-04-26T10:05:00Z".into(),
            }],
        })
    }

    #[test]
    fn render_report_snapshot() {
        let html = render_html_report(&sample_data()).unwrap();
        assert_snapshot!("html_report", html);
    }

    #[test]
    fn evidence_row_defaults_triage_to_unreviewed_when_no_entry() {
        let data = sample_data();
        for row in &data.evidence_rows {
            assert_eq!(row.triage_status, "unreviewed");
            assert_eq!(row.triage_note, "");
        }
    }

    #[test]
    fn evidence_row_carries_triage_status_and_note_when_present() {
        let data = sample_data_with_triage();
        let row = data.evidence_rows.iter().find(|r| r.seq == 1).unwrap();
        assert_eq!(row.triage_status, "confirmed");
        assert_eq!(row.triage_note, "verified by hand; reproduces");
    }

    #[test]
    fn markdown_evidence_section_renders_triage_status_and_note() {
        let md = render_markdown_report(&sample_data_with_triage());
        assert!(md.contains("| Triage | confirmed |"));
        assert!(md.contains("| Triage note | verified by hand; reproduces |"));
    }

    #[test]
    fn markdown_evidence_section_omits_empty_triage_note() {
        let md = render_markdown_report(&sample_data());
        assert!(md.contains("| Triage | unreviewed |"));
        assert!(!md.contains("| Triage note |"));
    }

    #[test]
    fn render_markdown_includes_headers_summary_and_findings() {
        let md = render_markdown_report(&sample_data());
        assert!(md.starts_with("# Report — 2026-04-26-acme-chatbot"));
        assert!(md.contains("- Run: `run-001`"));
        assert!(md.contains("## Summary"));
        assert!(md.contains("| Attempts | 2 |"));
        assert!(md.contains("| Judged | 2 |"));
        assert!(md.contains("## Findings by OWASP reference"));
        assert!(md.contains("### A01 —"));
        assert!(md.contains("## Findings by category"));
        assert!(md.contains("### injection-classics —"));
        assert!(md.contains("## Evidence"));
        assert!(md.contains("### #1 · inj-001 · SUCCESS"));
        assert!(md.contains("```"));
    }

    #[test]
    fn render_markdown_escapes_pipes_and_newlines_in_table_cells() {
        let mut data = sample_data();
        data.evidence_rows[0].rationale = "has | pipe\nand newline".into();
        data.groups_by_owasp[0].findings[0].rationale = "has | pipe\nand newline".into();
        let md = render_markdown_report(&data);
        // No raw pipes or newlines mid-cell.
        assert!(md.contains("has \\| pipe and newline"));
        // Tables still parse: every findings row has the right column count.
        // 7 columns -> 8 border pipes. Escaped `\|` characters don't count.
        for line in md.lines().filter(|l| l.starts_with("| 1 |")) {
            let total = line.matches('|').count();
            let escaped = line.matches("\\|").count();
            assert_eq!(
                total - escaped,
                8,
                "findings row should have 8 border pipes (7 columns), got: {line}"
            );
        }
    }

    #[test]
    fn report_has_no_external_assets_or_script() {
        let html = render_html_report(&sample_data()).unwrap();
        assert!(html.contains("<style>"));
        assert!(!html.contains("<script"));
        assert!(!html.contains("http://"));
        assert!(!html.contains("https://"));
        assert!(!html.contains("<link rel=\"stylesheet\""));
    }
}
