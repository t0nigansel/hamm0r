/**
 * api.js — Tauri command layer for hamm0r (Rust backend).
 *
 * Calls Rust commands directly via window.__TAURI__.core.invoke().
 * Provides a compatibility shim so app.js can use the same API.call()
 * pattern it used with the old Python sidecar.
 *
 * Command mapping (old name → new Rust command):
 *   list_engagements  → list_engagements   (Vec<EngagementMeta>)
 *   create_engagement → create_engagement  (EngagementMeta)
 *   open_db           → client-side only   (sets activeSlug)
 *   list_targets      → list_targets       (Vec<TargetDto>)
 *   get_target        → list_targets + find
 *   save_target       → save_target        (TargetDto)
 *   delete_target     → delete_target
 *   list_prompts      → list_prompts       (HashMap → flat array)
 *   list_scenarios    → list_scenarios     (HashMap → array)
 *   fire_prompt       → start_run + event wait
 *   read_run_attempts → read_run_attempts
 *   read_response_body→ read_response_body
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
      return Object.values(map);
    },

    // ── Run / fire ───────────────────────────────────────────────────

    /**
     * fire_prompt — maps the old single-fire API to start_run + wait for
     * the completion event, then reads the response body.
     *
     * Returns { run_id, response_text, status, duration_ms, error }.
     */
    async fire_prompt({ target_id, prompt_text, prompt_id }) {
      if (!_activeSlug) throw new Error('No engagement open. Open or create one first.');

      // start_run needs a request_id; targets use the same id as their request.
      const run_id = await invoke('start_run', {
        engagementSlug: _activeSlug,
        requestId: target_id,
        payloads: [{
          promptId: prompt_id || 'manual',
          payloadId: 'manual-001',
          text: prompt_text,
        }],
        parallelism: 1,
      });

      // Wait for the finished event for this run.
      return new Promise((resolve, reject) => {
        let unlistenFn = null;
        const timeout = setTimeout(() => {
          if (unlistenFn) unlistenFn();
          reject(new Error('Run timed out'));
        }, 120_000);

        window.__TAURI__.event.listen('run-progress', ev => {
          const p = ev.payload;
          if (p.run_id !== run_id) return;
          if (_onProgress) _onProgress(p);
          if (!p.finished) return;

          clearTimeout(timeout);
          if (unlistenFn) unlistenFn();

          if (p.error) {
            resolve({ run_id, response_text: '', status: 0, duration_ms: 0, error: p.error });
            return;
          }

          invoke('read_response_body', {
            engagementSlug: _activeSlug,
            runId: run_id,
            seq: 1,
          }).then(body => {
            resolve({
              run_id,
              result_id: `${run_id}-1`,
              response_text: body || '',
              status: p.status,
              duration_ms: 0,
              error: null,
            });
          }).catch(reject);
        }).then(fn => { unlistenFn = fn; });
      });
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

    async read_run_attempts({ engagement_slug, run_id }) {
      return invoke('read_run_attempts', {
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

    // ── Stubs for analyzer commands (not in M4 scope) ────────────────

    async judge_result() { throw new Error('Analyzer not yet activated'); },
    async judge_all() { throw new Error('Analyzer not yet activated'); },
    async promote_finding() { throw new Error('Analyzer not yet activated'); },
    async list_findings() { return []; },
    async export_findings_pdf() { throw new Error('PDF export not yet available'); },

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
