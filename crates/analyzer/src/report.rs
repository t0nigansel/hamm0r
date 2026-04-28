use std::collections::{BTreeMap, HashMap};

use anyhow::Context as _;
use minijinja::{AutoEscape, Environment};
use serde::Serialize;
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
    pub attempts: Vec<ReportAttempt>,
    pub verdicts: Vec<VerdictEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReportData {
    pub engagement_slug: String,
    pub run_id: String,
    pub generated_at: String,
    pub started_at: String,
    pub finished_at: String,
    pub summary: ReportSummary,
    pub groups_by_category: Vec<ReportGroup>,
    pub groups_by_owasp: Vec<ReportGroup>,
    pub evidence_rows: Vec<ReportEvidenceRow>,
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
}

pub fn build_report_data(input: ReportBuildInput) -> ReportData {
    let mut attempts = input.attempts;
    attempts.sort_by_key(|a| a.seq);

    let mut by_seq: HashMap<u32, VerdictEntry> = HashMap::new();
    for verdict in input.verdicts {
        by_seq.insert(verdict.seq, verdict);
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
            }
        })
        .collect();

    ReportData {
        engagement_slug: input.engagement_slug,
        run_id: input.run_id,
        generated_at: input.generated_at,
        started_at: input.started_at.unwrap_or_else(|| "-".to_owned()),
        finished_at: input.finished_at.unwrap_or_else(|| "-".to_owned()),
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
        })
    }

    #[test]
    fn render_report_snapshot() {
        let html = render_html_report(&sample_data()).unwrap();
        assert_snapshot!("html_report", html);
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
