/**
 * api.js — Tauri command layer for hamm0r (Rust backend).
 *
 * Calls Rust commands directly via window.__TAURI__.core.invoke().
 * Provides a compatibility shim so app.js can keep using API.call().
 */

const API = (() => {
  // Active engagement slug set when the user opens an engagement.
  let _activeSlug = null;

  // Cache of engagements loaded from list_engagements (slug → meta).
  let _engCache = {};

  // Unlisten function for the run-progress event (one run at a time).
  let _progressUnlisten = null;

  // External progress callback registered by app.js for live run updates.
  let _onProgress = null;

  // ── Low-level invoke ────────────────────────────────────────────────

  function invoke(cmd, params = {}) {
    if (!window.__TAURI__) {
      return Promise.reject(new Error(
        'Not running inside Tauri. Start the app with `cargo tauri dev`.'
      ));
    }
    return window.__TAURI__.core.invoke(cmd, params).catch(err => {
      throw new Error(typeof err === 'string' ? err : (err.message || JSON.stringify(err)));
    });
  }

  // ── Event listener ──────────────────────────────────────────────────

  async function listenRunProgress() {
    if (_progressUnlisten) {
      _progressUnlisten();
      _progressUnlisten = null;
    }
    _progressUnlisten = await window.__TAURI__.event.listen('run-progress', ev => {
      if (_onProgress) _onProgress(ev.payload);
    });
  }

  async function waitForRunCompletion(run_id, timeoutMs = 120_000) {
    return new Promise((resolve, reject) => {
      let unlistenFn = null;
      const timeout = setTimeout(() => {
        if (unlistenFn) unlistenFn();
        reject(new Error('Run timed out'));
      }, timeoutMs);

      window.__TAURI__.event.listen('run-progress', ev => {
        const p = ev.payload;
        if (p.run_id !== run_id) return;
        if (_onProgress) _onProgress(p);
        if (!p.finished) return;

        clearTimeout(timeout);
        if (unlistenFn) unlistenFn();
        resolve(p);
      }).then(fn => { unlistenFn = fn; });
    });
  }

  function toUiScenario(s) {
    if (!s) return null;
    return {
      ...s,
      repeat_count: s.repeat || 1,
      tags: [],
      sessions: [...new Set((s.steps || []).map(step => step.session || 'A'))],
      steps: (s.steps || []).map(step => ({
        id: step.id,
        session: step.session || 'A',
        prompt_id: step.prompt_id || null,
        prompt_category: step.prompt_category || null,
        prompt_text: step.prompt_text || '',
        delay_ms: 0,
      })),
    };
  }

  // ── Command handlers ────────────────────────────────────────────────

  const handlers = {

    // ── Engagement ──────────────────────────────────────────────────

    async list_engagements() {
      const list = await invoke('list_engagements');
      list.forEach(e => { _engCache[e.slug] = e; });
      return list;
    },

    async create_engagement({ name }) {
      const meta = await invoke('create_engagement', { name });
      _engCache[meta.slug] = meta;
      return meta;
    },

    // open_db is a client-side-only concept: no Rust call needed.
    // Just record the active slug and return the cached meta.
    async open_db({ path: slug }) {
      _activeSlug = slug;
      const meta = _engCache[slug] || { name: slug, slug };
      return meta;
    },

    get active_slug() { return _activeSlug; },

    // ── Targets ─────────────────────────────────────────────────────

    async list_targets() {
      return invoke('list_targets');
    },

    async get_target({ id }) {
      const targets = await invoke('list_targets');
      return targets.find(t => t.id === id) || null;
    },

    async save_target(dto) {
      return invoke('save_target', { dto });
    },

    async delete_target({ id }) {
      return invoke('delete_target', { id });
    },

    // ── Prompts ─────────────────────────────────────────────────────

    async list_prompts() {
      const map = await invoke('list_prompts');
      // Flatten HashMap<category, Vec<PromptEntry>> → flat array with category field.
      const flat = [];
      for (const [category, entries] of Object.entries(map)) {
        for (const entry of entries) {
          flat.push({ ...entry, category });
        }
      }
      return flat;
    },

    // ── Scenarios ────────────────────────────────────────────────────

    async list_scenarios() {
      const map = await invoke('list_scenarios');
      return Object.values(map).map(toUiScenario).sort((a, b) => a.name.localeCompare(b.name));
    },

    async create_scenario({ name }) {
      const scenario = await invoke('create_scenario', { name });
      return toUiScenario(scenario);
    },

    async get_scenario({ id }) {
      const scenario = await invoke('get_scenario', { id });
      return toUiScenario(scenario);
    },

    async update_scenario(data) {
      const current = await invoke('get_scenario', { id: data.id });
      if (!current) throw new Error(`Scenario '${data.id}' not found`);
      const updated = {
        ...current,
        name: data.name || current.name || 'Untitled',
        target_id: data.target_id || '',
        repeat: Math.max(1, Number(data.repeat_count || current.repeat || 1)),
      };
      const saved = await invoke('save_scenario', { scenario: updated });
      return toUiScenario(saved);
    },

    async save_steps({ scenario_id, steps }) {
      const current = await invoke('get_scenario', { id: scenario_id });
      if (!current) throw new Error(`Scenario '${scenario_id}' not found`);

      const promptIndex = new Map();
      const prompts = await handlers.list_prompts();
      prompts.forEach(p => {
        if (!promptIndex.has(p.id)) promptIndex.set(p.id, p.category || null);
      });

      const normalizedSteps = (steps || []).map((step, idx) => ({
        id: step.id || `step-${String(idx + 1).padStart(3, '0')}`,
        prompt_category:
          step.prompt_category || (step.prompt_id ? (promptIndex.get(step.prompt_id) || null) : null),
        prompt_id: step.prompt_id || null,
        prompt_text: step.prompt_text || '',
        session: step.session || 'A',
      }));

      const saved = await invoke('save_scenario', {
        scenario: { ...current, steps: normalizedSteps },
      });
      return toUiScenario(saved);
    },

    async delete_scenario({ id }) {
      await invoke('delete_scenario', { id });
      return { ok: true };
    },

    // ── Run / fire ───────────────────────────────────────────────────

    async fire_prompt({ target_id, prompt_text, prompt_id }) {
      if (!_activeSlug) throw new Error('No engagement open. Open or create one first.');

      const run_id = await invoke('start_transient_scenario_run', {
        engagementSlug: _activeSlug,
        targetId: target_id,
        promptText: prompt_text,
        promptId: prompt_id || null,
      });

      const progress = await waitForRunCompletion(run_id);
      if (progress.error) {
        return { run_id, response_text: '', status: 0, duration_ms: 0, error: progress.error };
      }

      const body = await invoke('read_response_body', {
        engagementSlug: _activeSlug,
        runId: run_id,
        seq: 1,
      });

      return {
        run_id,
        result_id: `${run_id}-1`,
        response_text: body || '',
        status: progress.status,
        duration_ms: 0,
        error: null,
      };
    },

    async start_scenario({ scenario_id }) {
      if (!_activeSlug) throw new Error('No engagement open. Open or create one first.');
      const run_id = await invoke('start_scenario_run', {
        engagementSlug: _activeSlug,
        scenarioId: scenario_id,
      });
      const progress = await waitForRunCompletion(run_id);
      return {
        run_id,
        id: run_id,
        status: progress.error ? 'failed' : 'completed',
      };
    },

    async stop_run() {
      // Graceful cancellation is planned; keep API shape stable for now.
      return { ok: true };
    },

    async start_run({ engagement_slug, request_id, payloads, parallelism }) {
      return invoke('start_run', {
        engagementSlug: engagement_slug || _activeSlug,
        requestId: request_id,
        payloads: payloads.map(p => ({
          promptId: p.prompt_id,
          payloadId: p.payload_id,
          text: p.text,
        })),
        parallelism,
      });
    },

    async list_runs({ engagement_slug }) {
      return invoke('list_runs', {
        engagementSlug: engagement_slug || _activeSlug,
      });
    },

    async get_run_progress({ engagement_slug, run_id }) {
      return invoke('get_run_progress', {
        engagementSlug: engagement_slug || _activeSlug,
        runId: run_id,
      });
    },

    async read_run_attempts({ engagement_slug, run_id }) {
      return invoke('read_run_attempts', {
        engagementSlug: engagement_slug || _activeSlug,
        runId: run_id,
      });
    },

    async read_run_verdicts({ engagement_slug, run_id }) {
      return invoke('read_run_verdicts', {
        engagementSlug: engagement_slug || _activeSlug,
        runId: run_id,
      });
    },

    async read_response_body({ engagement_slug, run_id, seq }) {
      return invoke('read_response_body', {
        engagementSlug: engagement_slug || _activeSlug,
        runId: run_id,
        seq,
      });
    },

    async get_results({ engagement_slug, run_id }) {
      const attempts = await handlers.read_run_attempts({ engagement_slug, run_id });
      const verdicts = await handlers.read_run_verdicts({ engagement_slug, run_id });
      const verdictBySeq = new Map((verdicts || []).map(v => [Number(v.seq), v]));
      const hasRepeat = attempts.some(a => (a.iteration || 1) > 1);
      const sorted = [...attempts].sort((a, b) => (a.seq || 0) - (b.seq || 0));

      return Promise.all(sorted.map(async a => {
        const response_text = a.response?.body_file
          ? (await handlers.read_response_body({ engagement_slug, run_id, seq: a.seq })) || ''
          : '';
        const judged = verdictBySeq.get(Number(a.seq)) || null;

        const iteration = a.iteration || 1;
        const stepId = a.step_id || a.payload_id || `seq-${a.seq}`;
        const step_order = hasRepeat ? `${iteration}.${stepId}` : (a.step_id || a.seq);

        return {
          run_id: run_id,
          result_id: `${run_id}-${a.seq}`,
          step_id: stepId,
          iteration,
          step_order,
          seq: a.seq,
          session_label: a.session || '-',
          prompt_id: a.prompt_id || 'custom',
          prompt_text: a.prompt_text || '',
          status_code: a.response?.status ?? 0,
          response_text,
          request_url: a.request?.url || '',
          request_method: a.request?.method || '',
          sent_at: a.timing?.sent_at || '',
          received_at: a.timing?.received_at || '',
          error_message: a.response?.error || null,
          latency_ms: a.timing?.duration_ms ?? null,
          judge_verdict: judged?.judge_verdict || '',
          judge_confidence: judged?.judge_confidence ?? null,
          judge_reason: judged?.judge_reason || '',
          judge_model_used: judged?.judge_model_used || '',
          judge_evaluated_at: judged?.judge_evaluated_at || '',
        };
      }));
    },

    // ── Analyzer commands ─────────────────────────────────────────────

    async judge_result({ engagement_slug, result_id, force = false }) {
      if (!_activeSlug && !engagement_slug) throw new Error('No engagement open. Open or create one first.');
      return invoke('judge_result', {
        engagementSlug: engagement_slug || _activeSlug,
        resultId: result_id,
        force: !!force,
      });
    },

    async judge_all({ engagement_slug, result_ids, run_id = null, force = false }) {
      if (!_activeSlug && !engagement_slug) throw new Error('No engagement open. Open or create one first.');
      return invoke('judge_all', {
        engagementSlug: engagement_slug || _activeSlug,
        resultIds: Array.isArray(result_ids) ? result_ids : [],
        runId: run_id,
        force: !!force,
      });
    },

    async start_analysis({ engagement_slug, run_id, force = false }) {
      if (!_activeSlug && !engagement_slug) throw new Error('No engagement open. Open or create one first.');
      return invoke('start_analysis', {
        engagementSlug: engagement_slug || _activeSlug,
        runId: run_id,
        force: !!force,
      });
    },

    async generate_report({ engagement_slug, run_id }) {
      if (!_activeSlug && !engagement_slug) throw new Error('No engagement open. Open or create one first.');
      const slug = engagement_slug || _activeSlug;
      let targetRunId = run_id;

      if (!targetRunId) {
        const runs = await handlers.list_runs({ engagement_slug: slug });
        if (!runs.length) throw new Error('No runs available to report');
        targetRunId = runs[0].id;
      }

      const path = await invoke('generate_report', {
        engagementSlug: slug,
        runId: targetRunId,
      });
      return { path, run_id: targetRunId };
    },

    async read_report_html({ engagement_slug, run_id }) {
      if (!_activeSlug && !engagement_slug) throw new Error('No engagement open. Open or create one first.');
      return invoke('read_report_html', {
        engagementSlug: engagement_slug || _activeSlug,
        runId: run_id,
      });
    },

    // ── Analyzer setup ────────────────────────────────────────────────

    async get_analyzer_status() {
      return invoke('get_analyzer_status');
    },

    async fetch_analyzer_manifest() {
      return invoke('fetch_analyzer_manifest');
    },

    async download_and_install_analyzer({ variant_id }) {
      return invoke('download_and_install_analyzer', { variantId: variant_id });
    },

    async uninstall_analyzer() {
      return invoke('uninstall_analyzer');
    },

    async promote_finding() { throw new Error('Analyzer not yet activated'); },
    async list_findings() { return []; },
    async export_findings_pdf({ run_id } = {}) {
      return handlers.generate_report({ run_id });
    },

    // ── Stubs for library-editing commands (not in M4 scope) ─────────

    async create_prompt() { throw new Error('Prompt editing coming in a future milestone'); },
    async update_prompt() { throw new Error('Prompt editing coming in a future milestone'); },
    async delete_prompt() { throw new Error('Prompt editing coming in a future milestone'); },
    async get_prompt() { throw new Error('Prompt editing coming in a future milestone'); },
    async import_csv() { throw new Error('CSV import coming in a future milestone'); },
    async seed_library() { return { seeded: 0 }; },
    async get_mutations() { return []; },
    async get_db_status() { return { open: !!_activeSlug, slug: _activeSlug }; },
  };

  // ── Public API ──────────────────────────────────────────────────────

  async function call(cmd, params = {}) {
    const handler = handlers[cmd];
    if (typeof handler === 'function') {
      return handler(params);
    }
    throw new Error(`Command '${cmd}' is not implemented`);
  }

  function onProgress(fn) { _onProgress = fn; }

  // Start listening for run-progress events immediately so they aren't lost.
  if (window.__TAURI__) {
    listenRunProgress().catch(console.error);
  }

  return { call, onProgress, get activeSlug() { return _activeSlug; } };
})();
