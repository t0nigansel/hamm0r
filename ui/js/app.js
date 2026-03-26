/**
 * app.js — Main UI logic for promt0r.
 *
 * Stack.md: "plain HTML + CSS + vanilla JS. No React, no Vue, no bundler."
 *
 * This file wires up all UI interactions: tabs, forms, tables, dialogs.
 * All backend communication goes through API.call() (see api.js).
 */

document.addEventListener('DOMContentLoaded', () => {
  // ── State ──────────────────────────────────────────────────────────
  let dbOpen = false;
  let currentRunId = null;
  let progressPollTimer = null;
  let editingPromptId = null; // null = add mode, string = edit mode

  // ── DOM refs ───────────────────────────────────────────────────────
  const $ = (sel) => document.querySelector(sel);
  const $$ = (sel) => document.querySelectorAll(sel);

  // ── Toast notifications ────────────────────────────────────────────
  function toast(message, type = 'info') {
    const el = document.createElement('div');
    el.className = `toast toast-${type}`;
    el.textContent = message;
    $('#toast-container').appendChild(el);
    setTimeout(() => el.remove(), 4000);
  }

  // ── Tab navigation ─────────────────────────────────────────────────
  $$('.tab').forEach(tab => {
    tab.addEventListener('click', () => {
      $$('.tab').forEach(t => t.classList.remove('active'));
      $$('.panel').forEach(p => p.classList.remove('active'));
      tab.classList.add('active');
      $(`#${tab.dataset.panel}`).classList.add('active');

      // Refresh data when switching tabs
      const panel = tab.dataset.panel;
      if (panel === 'panel-prompts' && dbOpen) loadPrompts();
      if (panel === 'panel-run' && dbOpen) loadRuns();
      if (panel === 'panel-results' && dbOpen) loadRunsForSelect();
    });
  });

  // ── Engagement management ──────────────────────────────────────────
  $('#btn-new-engagement').addEventListener('click', () => {
    $('#engagement-dialog').style.display = 'flex';
  });

  $('#engagement-dialog-close').addEventListener('click', () => {
    $('#engagement-dialog').style.display = 'none';
  });

  $('#engagement-form').addEventListener('submit', async (e) => {
    e.preventDefault();
    const name = $('#eng-name').value.trim();
    const path = $('#eng-path').value.trim();
    const seed = $('#eng-seed').checked;

    try {
      await API.call('create_engagement', { name, path });
      dbOpen = true;
      $('#db-label').textContent = path;
      $('#engagement-dialog').style.display = 'none';

      if (seed) {
        await API.call('seed_library', { update: false });
        toast('Prompt library seeded', 'success');
      }

      toast(`Engagement created: ${name}`, 'success');
      loadPrompts();
    } catch (err) {
      toast(err.message, 'error');
    }
  });

  // Auto-fill .db path from name
  $('#eng-name').addEventListener('input', () => {
    const name = $('#eng-name').value.trim().replace(/\s+/g, '-').toLowerCase();
    if (name) $('#eng-path').value = `${name}.db`;
  });

  $('#btn-open-engagement').addEventListener('click', async () => {
    // Decision: For v0.1 without Tauri file dialog, prompt for path.
    // In production, this would use Tauri's dialog plugin.
    const path = prompt('Enter path to .db file:');
    if (!path) return;
    try {
      await API.call('open_db', { path });
      dbOpen = true;
      $('#db-label').textContent = path;
      toast(`Opened: ${path}`, 'success');
      loadPrompts();
    } catch (err) {
      toast(err.message, 'error');
    }
  });

  // ── Target config: show/hide auth fields ───────────────────────────
  $('#target-auth-type').addEventListener('change', () => {
    const authType = $('#target-auth-type').value;
    $('#auth-value-row').style.display = authType === 'none' ? 'none' : '';
    $('#auth-header-row').style.display = authType === 'none' ? 'none' : '';
  });

  $('#target-endpoint').addEventListener('change', () => {
    const ep = $('#target-endpoint').value;
    $('#field-mapping-group').style.display = ep === 'custom_rest' ? '' : 'none';
  });

  // ── Prompts: load and render ───────────────────────────────────────
  async function loadPrompts() {
    if (!dbOpen) return;
    const params = {};
    const owasp = $('#prompt-filter-owasp').value;
    const category = $('#prompt-filter-category').value;
    if (owasp) params.owasp = owasp;
    else if (category) params.category = category;

    try {
      const prompts = await API.call('list_prompts', params);
      renderPrompts(prompts);
    } catch (err) {
      toast(err.message, 'error');
    }
  }

  function renderPrompts(prompts) {
    const tbody = $('#prompts-tbody');
    tbody.innerHTML = '';
    prompts.forEach(p => {
      const tr = document.createElement('tr');
      tr.innerHTML = `
        <td><code>${esc(p.id)}</code></td>
        <td>${esc(p.owasp_ref)}</td>
        <td><span class="badge badge-${p.severity.toLowerCase()}">${esc(p.severity)}</span></td>
        <td>${esc(p.category)}</td>
        <td><div class="cell-text">${esc(p.text)}</div></td>
        <td>${(p.tags || []).map(t => `<span class="tag">${esc(t)}</span>`).join(' ')}</td>
        <td>
          <button class="btn-edit" data-id="${esc(p.id)}">Edit</button>
          <button class="btn-del" data-id="${esc(p.id)}">Del</button>
        </td>
      `;
      tbody.appendChild(tr);
    });

    $('#prompt-count').textContent = `${prompts.length} prompts`;

    // Wire up edit/delete buttons
    tbody.querySelectorAll('.btn-edit').forEach(btn => {
      btn.addEventListener('click', () => openPromptEditor(btn.dataset.id));
    });
    tbody.querySelectorAll('.btn-del').forEach(btn => {
      btn.addEventListener('click', () => deletePrompt(btn.dataset.id));
    });
  }

  // Prompt filters
  $('#prompt-filter-owasp').addEventListener('change', () => {
    $('#prompt-filter-category').value = '';
    loadPrompts();
  });
  $('#prompt-filter-category').addEventListener('change', () => {
    $('#prompt-filter-owasp').value = '';
    loadPrompts();
  });

  // ── Prompts: add/edit ──────────────────────────────────────────────
  $('#btn-add-prompt').addEventListener('click', () => openPromptEditor(null));

  async function openPromptEditor(promptId) {
    editingPromptId = promptId;
    const editor = $('#prompt-editor');
    editor.style.display = '';

    if (promptId) {
      // Edit mode: load existing prompt
      $('#editor-title').textContent = 'Edit Prompt';
      $('#pe-id').readOnly = true;
      try {
        const p = await API.call('get_prompt', { id: promptId });
        if (!p) { toast('Prompt not found', 'error'); return; }
        $('#pe-id').value = p.id;
        $('#pe-text').value = p.text;
        $('#pe-category').value = p.category;
        $('#pe-owasp').value = p.owasp_ref;
        $('#pe-severity').value = p.severity;
        $('#pe-tags').value = (p.tags || []).join(', ');
        $('#pe-source').value = p.source || '';
      } catch (err) {
        toast(err.message, 'error');
      }
    } else {
      // Add mode: clear form
      $('#editor-title').textContent = 'Add Prompt';
      $('#pe-id').readOnly = false;
      $('#prompt-form').reset();
    }
  }

  $('#pe-cancel').addEventListener('click', () => {
    $('#prompt-editor').style.display = 'none';
    editingPromptId = null;
  });

  $('#prompt-form').addEventListener('submit', async (e) => {
    e.preventDefault();
    const data = {
      id: $('#pe-id').value.trim(),
      text: $('#pe-text').value,
      category: $('#pe-category').value,
      owasp_ref: $('#pe-owasp').value,
      severity: $('#pe-severity').value,
      tags: $('#pe-tags').value.split(',').map(t => t.trim()).filter(Boolean),
      mode: 'single',
      source: $('#pe-source').value.trim() || 'manual',
    };

    try {
      if (editingPromptId) {
        await API.call('update_prompt', data);
        toast('Prompt updated', 'success');
      } else {
        await API.call('create_prompt', data);
        toast('Prompt created', 'success');
      }
      $('#prompt-editor').style.display = 'none';
      editingPromptId = null;
      loadPrompts();
    } catch (err) {
      toast(err.message, 'error');
    }
  });

  async function deletePrompt(id) {
    if (!confirm(`Delete prompt ${id}?`)) return;
    try {
      await API.call('delete_prompt', { id });
      toast('Prompt deleted', 'success');
      loadPrompts();
    } catch (err) {
      toast(err.message, 'error');
    }
  }

  // ── Prompts: CSV import ────────────────────────────────────────────
  $('#btn-import-csv').addEventListener('click', () => {
    $('#csv-file-input').click();
  });

  $('#csv-file-input').addEventListener('change', async (e) => {
    const file = e.target.files[0];
    if (!file) return;
    const text = await file.text();
    try {
      const result = await API.call('import_csv', { csv_text: text });
      toast(`Imported ${result.imported} prompts`, 'success');
      if (result.errors.length) {
        toast(`${result.errors.length} import errors (see console)`, 'error');
        console.error('CSV import errors:', result.errors);
      }
      loadPrompts();
    } catch (err) {
      toast(err.message, 'error');
    }
    e.target.value = ''; // reset file input
  });

  // ── Prompts: seed library ──────────────────────────────────────────
  $('#btn-seed-library').addEventListener('click', async () => {
    if (!dbOpen) { toast('Open an engagement first', 'error'); return; }
    try {
      const result = await API.call('seed_library', { update: true });
      toast(`Seeded ${result.loaded} prompts`, 'success');
      loadPrompts();
    } catch (err) {
      toast(err.message, 'error');
    }
  });

  // ── Run: attack/stop ───────────────────────────────────────────────
  function getTargetConfig() {
    const config = {
      name: $('#target-name').value.trim(),
      url: $('#target-url').value.trim(),
      endpoint_type: $('#target-endpoint').value,
      auth_type: $('#target-auth-type').value,
      tester_name: $('#run-tester').value.trim() || 'default',
      concurrency: parseInt($('#run-concurrency').value) || 1,
      delay_ms: parseInt($('#run-delay').value) || 0,
      verify_ssl: $('#run-verify-ssl').checked,
    };

    if (config.auth_type !== 'none') {
      config.auth_value = $('#target-auth-value').value;
      const hdr = $('#target-auth-header').value.trim();
      if (hdr) config.auth_header = hdr;
    }

    if (config.endpoint_type === 'custom_rest') {
      config.field_mapping = {
        request_field: $('#map-request').value.trim() || 'message',
        response_field: $('#map-response').value.trim() || 'response',
      };
    }

    const sysprompt = $('#target-system-prompt').value.trim();
    if (sysprompt) config.system_prompt = sysprompt;

    const owaspFilter = $('#run-filter-owasp').value;
    if (owaspFilter) config.owasp = owaspFilter;

    return config;
  }

  $('#btn-attack').addEventListener('click', async () => {
    if (!dbOpen) { toast('Open an engagement first', 'error'); return; }

    const config = getTargetConfig();
    if (!config.name || !config.url) {
      toast('Fill in target name and URL on the Target tab first', 'error');
      return;
    }

    // Disable attack, enable stop
    $('#btn-attack').disabled = true;
    $('#btn-stop').disabled = false;
    $('#progress-section').style.display = '';
    resetProgress();

    try {
      const result = await API.call('start_run', config);
      currentRunId = result.id || result.run_id;
      toast(`Run complete: ${result.status}`, result.status === 'completed' ? 'success' : 'info');

      // Final update
      updateProgress(result.completed, result.total_prompts, result.errors);
    } catch (err) {
      toast(err.message, 'error');
    } finally {
      $('#btn-attack').disabled = false;
      $('#btn-stop').disabled = true;
      stopProgressPoll();
      loadRuns();
    }
  });

  $('#btn-stop').addEventListener('click', async () => {
    try {
      await API.call('stop_run', {});
      toast('Stop requested — finishing in-flight requests...', 'info');
    } catch (err) {
      toast(err.message, 'error');
    }
  });

  // ── Progress display ───────────────────────────────────────────────
  function resetProgress() {
    $('#progress-bar').style.width = '0%';
    $('#progress-completed').textContent = '0';
    $('#progress-total').textContent = '0';
    $('#progress-errors').textContent = '0';
    $('#progress-last').textContent = '';
  }

  function updateProgress(completed, total, errors) {
    const pct = total > 0 ? (completed / total * 100) : 0;
    $('#progress-bar').style.width = `${pct}%`;
    $('#progress-completed').textContent = completed;
    $('#progress-total').textContent = total;
    $('#progress-errors').textContent = errors;
  }

  function startProgressPoll(runId) {
    stopProgressPoll();
    progressPollTimer = setInterval(async () => {
      try {
        const run = await API.call('get_run_progress', { run_id: runId });
        if (run) {
          updateProgress(run.completed, run.total_prompts, run.errors);
          if (run.status !== 'running') stopProgressPoll();
        }
      } catch { /* ignore poll errors */ }
    }, 1000);
  }

  function stopProgressPoll() {
    if (progressPollTimer) {
      clearInterval(progressPollTimer);
      progressPollTimer = null;
    }
  }

  // ── Runs: load history ─────────────────────────────────────────────
  async function loadRuns() {
    if (!dbOpen) return;
    try {
      const runs = await API.call('list_runs', {});
      renderRuns(runs);
    } catch (err) {
      toast(err.message, 'error');
    }
  }

  function renderRuns(runs) {
    const tbody = $('#runs-tbody');
    tbody.innerHTML = '';
    runs.forEach(r => {
      const tr = document.createElement('tr');
      tr.className = 'clickable';
      tr.innerHTML = `
        <td><code>${esc(r.id.substring(0, 8))}</code></td>
        <td><span class="status-${r.status}">${esc(r.status)}</span></td>
        <td>${r.completed}/${r.total_prompts || '?'}</td>
        <td>${r.errors}</td>
        <td>${esc(r.started_at || '')}</td>
      `;
      tr.addEventListener('click', () => {
        // Switch to results tab and load this run
        $$('.tab').forEach(t => t.classList.remove('active'));
        $$('.panel').forEach(p => p.classList.remove('active'));
        document.querySelector('[data-panel="panel-results"]').classList.add('active');
        $('#panel-results').classList.add('active');
        $('#results-run-select').value = r.id;
        loadResults(r.id);
      });
      tbody.appendChild(tr);
    });
  }

  // ── Results ────────────────────────────────────────────────────────
  async function loadRunsForSelect() {
    if (!dbOpen) return;
    try {
      const runs = await API.call('list_runs', {});
      const sel = $('#results-run-select');
      const currentVal = sel.value;
      sel.innerHTML = '<option value="">Select a run...</option>';
      runs.forEach(r => {
        const opt = document.createElement('option');
        opt.value = r.id;
        opt.textContent = `${r.id.substring(0, 8)} — ${r.status} (${r.completed}/${r.total_prompts || '?'})`;
        sel.appendChild(opt);
      });
      if (currentVal) sel.value = currentVal;
    } catch (err) {
      toast(err.message, 'error');
    }
  }

  $('#results-run-select').addEventListener('change', () => {
    const runId = $('#results-run-select').value;
    if (runId) loadResults(runId);
  });

  async function loadResults(runId) {
    try {
      const results = await API.call('get_results', { run_id: runId });
      renderResults(results);
    } catch (err) {
      toast(err.message, 'error');
    }
  }

  function renderResults(results) {
    const tbody = $('#results-tbody');
    tbody.innerHTML = '';
    results.forEach(r => {
      const statusClass = r.error_message ? 'status-error' : 'status-ok';
      const statusText = r.error_message ? 'ERROR' : `${r.status_code || '?'}`;
      const tr = document.createElement('tr');
      tr.className = 'clickable';
      tr.innerHTML = `
        <td><code>${esc(r.prompt_id)}</code></td>
        <td><span class="${statusClass}">${statusText}</span></td>
        <td><div class="cell-text">${esc(r.prompt_text)}</div></td>
        <td><div class="cell-text">${esc(r.response_text || '')}</div></td>
        <td>${r.latency_ms != null ? r.latency_ms + 'ms' : '-'}</td>
        <td class="cell-truncate">${esc(r.error_message || '')}</td>
      `;
      tr.addEventListener('click', () => showResultDetail(r));
      tbody.appendChild(tr);
    });
    $('#results-count').textContent = `${results.length} results`;
  }

  function showResultDetail(r) {
    $('#detail-prompt').textContent = r.prompt_text;
    $('#detail-response').textContent = r.response_text || '(no response)';
    $('#detail-meta').innerHTML = `
      <strong>Prompt ID:</strong> ${esc(r.prompt_id)} &nbsp;|&nbsp;
      <strong>Status:</strong> ${r.status_code || 'N/A'} &nbsp;|&nbsp;
      <strong>Latency:</strong> ${r.latency_ms != null ? r.latency_ms + 'ms' : 'N/A'} &nbsp;|&nbsp;
      <strong>Time:</strong> ${esc(r.timestamp)}
      ${r.error_message ? '<br><strong>Error:</strong> ' + esc(r.error_message) : ''}
    `;
    $('#result-detail').style.display = 'flex';
  }

  $('#result-detail-close').addEventListener('click', () => {
    $('#result-detail').style.display = 'none';
  });

  // Close modals on backdrop click
  document.querySelectorAll('.modal').forEach(modal => {
    modal.addEventListener('click', (e) => {
      if (e.target === modal) modal.style.display = 'none';
    });
  });

  // ── Utility ────────────────────────────────────────────────────────
  function esc(str) {
    if (str == null) return '';
    const div = document.createElement('div');
    div.textContent = String(str);
    return div.innerHTML;
  }
});
