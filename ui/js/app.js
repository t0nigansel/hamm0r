/**
 * app.js — Main UI logic for promt0r.
 *
 * Sidebar + main panel layout with scenario-based testing.
 * All backend communication goes through API.call() (see api.js).
 */

document.addEventListener('DOMContentLoaded', () => {
  // ── State ──────────────────────────────────────────────────────────
  let dbOpen = false;
  let editingPromptId = null;
  let currentScenarioId = null;
  let currentScenarioSteps = []; // local step buffer
  let editingStepIndex = -1; // -1 = add, >= 0 = edit
  let currentRunId = null;
  let progressPollTimer = null;

  // ── DOM refs ───────────────────────────────────────────────────────
  const $ = (sel) => document.querySelector(sel);
  const $$ = (sel) => document.querySelectorAll(sel);

  function esc(str) {
    if (str == null) return '';
    const div = document.createElement('div');
    div.textContent = String(str);
    return div.innerHTML;
  }

  // ── Toast ──────────────────────────────────────────────────────────
  function toast(message, type = 'info') {
    const el = document.createElement('div');
    el.className = `toast toast-${type}`;
    el.textContent = message;
    $('#toast-container').appendChild(el);
    setTimeout(() => el.remove(), 4000);
  }

  // ── Check if DB is open on page load ────────────────────────────────
  async function checkDatabaseStatus() {
    try {
      const status = await API.call('get_db_status', {});
      if (status.open && status.path) {
        dbOpen = true;
        $('#db-label').textContent = status.path;
        loadTargetList();
        toast(`Engagement loaded: ${status.path}`, 'success');
      }
    } catch (err) {
      // DB not available yet, that's ok
    }
  }

  // Check for existing DB when page loads
  checkDatabaseStatus();

  // ── Sidebar navigation ─────────────────────────────────────────────
  $$('.nav-item').forEach(btn => {
    btn.addEventListener('click', () => {
      $$('.nav-item').forEach(b => b.classList.remove('active'));
      $$('.view').forEach(v => v.classList.remove('active'));
      btn.classList.add('active');
      $(`#${btn.dataset.view}`).classList.add('active');

      // Show/hide sidebar sections
      $('#sidebar-scenario-list').style.display =
        btn.dataset.view === 'view-scenarios' ? '' : 'none';
      $('#sidebar-target-list').style.display =
        btn.dataset.view === 'view-targets' ? '' : 'none';

      // Refresh data
      if (dbOpen) {
        if (btn.dataset.view === 'view-prompts') loadPrompts();
        if (btn.dataset.view === 'view-scenarios') loadScenarioList();
        if (btn.dataset.view === 'view-runs') loadRuns();
        if (btn.dataset.view === 'view-targets') loadTargetList();
      }
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
      loadTargetList();
    } catch (err) { toast(err.message, 'error'); }
  });
  $('#eng-name').addEventListener('input', () => {
    const name = $('#eng-name').value.trim().replace(/\s+/g, '-').toLowerCase();
    if (name) $('#eng-path').value = `${name}.db`;
  });
  $('#btn-open-engagement').addEventListener('click', async () => {
    const path = prompt('Enter path to .db file:');
    if (!path) return;
    try {
      await API.call('open_db', { path });
      dbOpen = true;
      $('#db-label').textContent = path;
      toast(`Opened: ${path}`, 'success');
      loadTargetList();
    } catch (err) { toast(err.message, 'error'); }
  });

  // ── Target config: show/hide fields ────────────────────────────────
  $('#target-auth-type').addEventListener('change', () => {
    const v = $('#target-auth-type').value;
    $('#auth-value-row').style.display = v === 'none' ? 'none' : '';
    $('#auth-header-row').style.display = v === 'none' ? 'none' : '';
  });
  $('#target-endpoint').addEventListener('change', () => {
    $('#field-mapping-group').style.display =
      $('#target-endpoint').value === 'custom_rest' ? '' : 'none';
  });
  $('#target-session-strategy').addEventListener('change', () => {
    $('#session-field-row').style.display =
      $('#target-session-strategy').value === 'none' ? 'none' : '';
  });

  // ── Targets: list + CRUD ───────────────────────────────────────────
  async function loadTargetList() {
    if (!dbOpen) return;
    try {
      const targets = await API.call('list_targets', {});
      const ul = $('#target-list');
      ul.innerHTML = '';
      targets.forEach(t => {
        const li = document.createElement('li');
        li.textContent = t.name;
        li.dataset.id = t.id;
        li.addEventListener('click', () => openTargetEditor(t.id));
        ul.appendChild(li);
      });
    } catch (err) { toast(err.message, 'error'); }
  }

  $('#btn-new-target').addEventListener('click', () => {
    $('#target-id').value = '';
    $('#target-form').reset();
    $('#target-form').style.display = '';
    $('#target-empty-msg').style.display = 'none';
  });

  async function openTargetEditor(targetId) {
    try {
      const t = await API.call('get_target', { id: targetId });
      if (!t) return;
      $('#target-id').value = t.id;
      $('#target-name').value = t.name;
      $('#target-url').value = t.url;
      $('#target-endpoint').value = t.endpoint_type;
      $('#target-auth-type').value = t.auth_type;
      $('#target-auth-type').dispatchEvent(new Event('change'));
      $('#target-endpoint').dispatchEvent(new Event('change'));
      $('#target-auth-value').value = t.auth_value || '';
      $('#target-auth-header').value = t.auth_header || '';
      if (t.field_mapping) {
        $('#map-request').value = t.field_mapping.request_field || 'message';
        $('#map-response').value = t.field_mapping.response_field || 'response';
      }
      $('#target-session-strategy').value = t.session_strategy || 'none';
      $('#target-session-strategy').dispatchEvent(new Event('change'));
      $('#target-session-field').value = t.session_field || '';
      $('#target-system-prompt').value = t.system_prompt || '';
      $('#target-form').style.display = '';
      $('#target-empty-msg').style.display = 'none';

      // Highlight in sidebar
      $$('#target-list li').forEach(li => li.classList.toggle('active', li.dataset.id === targetId));
    } catch (err) { toast(err.message, 'error'); }
  }

  $('#target-form').addEventListener('submit', async (e) => {
    e.preventDefault();
    const data = {
      name: $('#target-name').value.trim(),
      url: $('#target-url').value.trim(),
      endpoint_type: $('#target-endpoint').value,
      auth_type: $('#target-auth-type').value,
      session_strategy: $('#target-session-strategy').value,
      session_field: $('#target-session-field').value.trim() || null,
    };
    if ($('#target-id').value) data.id = $('#target-id').value;
    if (data.auth_type !== 'none') {
      data.auth_header = $('#target-auth-header').value.trim() || 'Authorization';
      data.auth_value = $('#target-auth-value').value;
    }
    if (data.endpoint_type === 'custom_rest') {
      data.field_mapping = {
        request_field: $('#map-request').value.trim() || 'message',
        response_field: $('#map-response').value.trim() || 'response',
      };
    }
    const sp = $('#target-system-prompt').value.trim();
    if (sp) data.system_prompt = sp;

    try {
      const saved = await API.call('save_target', data);
      $('#target-id').value = saved.id;
      toast('Target saved', 'success');
      loadTargetList();
    } catch (err) { toast(err.message, 'error'); }
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
    } catch (err) { toast(err.message, 'error'); }
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
        </td>`;
      tbody.appendChild(tr);
    });
    $('#prompt-count').textContent = `${prompts.length} prompts`;
    tbody.querySelectorAll('.btn-edit').forEach(btn =>
      btn.addEventListener('click', () => openPromptEditor(btn.dataset.id)));
    tbody.querySelectorAll('.btn-del').forEach(btn =>
      btn.addEventListener('click', () => deletePrompt(btn.dataset.id)));
  }

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
    $('#prompt-editor').style.display = '';
    if (promptId) {
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
      } catch (err) { toast(err.message, 'error'); }
    } else {
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
    } catch (err) { toast(err.message, 'error'); }
  });

  async function deletePrompt(id) {
    if (!confirm(`Delete prompt ${id}?`)) return;
    try {
      await API.call('delete_prompt', { id });
      toast('Prompt deleted', 'success');
      loadPrompts();
    } catch (err) { toast(err.message, 'error'); }
  }

  // ── Prompts: CSV import + seed ─────────────────────────────────────
  $('#btn-import-csv').addEventListener('click', () => $('#csv-file-input').click());
  $('#csv-file-input').addEventListener('change', async (e) => {
    const file = e.target.files[0];
    if (!file) return;
    const text = await file.text();
    try {
      const result = await API.call('import_csv', { csv_text: text });
      toast(`Imported ${result.imported} prompts`, 'success');
      if (result.errors.length) {
        toast(`${result.errors.length} import errors`, 'error');
      }
      loadPrompts();
    } catch (err) { toast(err.message, 'error'); }
    e.target.value = '';
  });
  $('#btn-seed-library').addEventListener('click', async () => {
    if (!dbOpen) { toast('Open an engagement first', 'error'); return; }
    try {
      const result = await API.call('seed_library', { update: true });
      toast(`Seeded ${result.loaded} prompts`, 'success');
      loadPrompts();
    } catch (err) { toast(err.message, 'error'); }
  });

  // ══════════════════════════════════════════════════════════════════
  // SCENARIOS
  // ══════════════════════════════════════════════════════════════════

  async function loadScenarioList() {
    if (!dbOpen) return;
    try {
      const scenarios = await API.call('list_scenarios', {});
      const ul = $('#scenario-list');
      ul.innerHTML = '';
      scenarios.forEach(s => {
        const li = document.createElement('li');
        li.textContent = s.name;
        li.dataset.id = s.id;
        if (s.id === currentScenarioId) li.classList.add('active');
        li.addEventListener('click', () => openScenario(s.id));
        ul.appendChild(li);
      });
    } catch (err) { toast(err.message, 'error'); }
  }

  $('#btn-new-scenario').addEventListener('click', async () => {
    if (!dbOpen) { toast('Open an engagement first', 'error'); return; }
    try {
      const s = await API.call('create_scenario', { name: 'New Scenario' });
      currentScenarioId = s.id;
      await loadScenarioList();
      openScenario(s.id);
    } catch (err) { toast(err.message, 'error'); }
  });

  async function openScenario(scenarioId) {
    currentScenarioId = scenarioId;
    try {
      const s = await API.call('get_scenario', { id: scenarioId });
      if (!s) return;

      $('#scenario-empty').style.display = 'none';
      $('#scenario-builder').style.display = '';
      $('#scenario-results').style.display = 'none';

      // Fill header fields
      $('#sc-name').value = s.name;
      $('#sc-tags').value = (s.tags || []).join(', ');
      $('#sc-repeat').value = s.repeat_count || 1;

      // Load target dropdown
      await loadTargetDropdown(s.target_id);

      // Load steps
      currentScenarioSteps = s.steps || [];
      renderStepTimeline();

      // Highlight in sidebar
      $$('#scenario-list li').forEach(li =>
        li.classList.toggle('active', li.dataset.id === scenarioId));
    } catch (err) { toast(err.message, 'error'); }
  }

  async function loadTargetDropdown(selectedId) {
    try {
      const targets = await API.call('list_targets', {});
      const sel = $('#sc-target');
      sel.innerHTML = '<option value="">Select target...</option>';
      targets.forEach(t => {
        const opt = document.createElement('option');
        opt.value = t.id;
        opt.textContent = t.name;
        sel.appendChild(opt);
      });
      if (selectedId) sel.value = selectedId;
    } catch (err) { /* ignore */ }
  }

  // ── Save scenario header ───────────────────────────────────────────
  $('#btn-save-scenario').addEventListener('click', async () => {
    if (!currentScenarioId) return;

    // Collect sessions from steps
    const sessions = [...new Set(currentScenarioSteps.map(s => s.session || 'A'))];
    if (sessions.length === 0) sessions.push('A');

    const data = {
      id: currentScenarioId,
      name: $('#sc-name').value.trim() || 'Untitled',
      target_id: $('#sc-target').value || null,
      tags: $('#sc-tags').value.split(',').map(t => t.trim()).filter(Boolean),
      repeat_count: parseInt($('#sc-repeat').value) || 1,
      sessions: sessions,
    };
    try {
      await API.call('update_scenario', data);
      // Save steps too
      await API.call('save_steps', {
        scenario_id: currentScenarioId,
        steps: currentScenarioSteps.map(s => ({
          session: s.session,
          prompt_id: s.prompt_id || null,
          prompt_text: s.prompt_text,
          delay_ms: s.delay_ms || 0,
        })),
      });
      toast('Scenario saved', 'success');
      loadScenarioList();
    } catch (err) { toast(err.message, 'error'); }
  });

  // ── Delete scenario ────────────────────────────────────────────────
  $('#btn-delete-scenario').addEventListener('click', async () => {
    if (!currentScenarioId) return;
    if (!confirm('Delete this scenario?')) return;
    try {
      await API.call('delete_scenario', { id: currentScenarioId });
      currentScenarioId = null;
      currentScenarioSteps = [];
      $('#scenario-builder').style.display = 'none';
      $('#scenario-empty').style.display = '';
      toast('Scenario deleted', 'success');
      loadScenarioList();
    } catch (err) { toast(err.message, 'error'); }
  });

  // ── Step timeline rendering ────────────────────────────────────────
  function renderStepTimeline() {
    const container = $('#step-timeline');
    container.innerHTML = '';
    currentScenarioSteps.forEach((step, i) => {
      const row = document.createElement('div');
      row.className = 'step-row';
      row.innerHTML = `
        <span class="step-num">${i + 1}</span>
        <span class="step-session" data-session="${esc(step.session)}"></span>
        <span class="step-session-label" data-session="${esc(step.session)}">${esc(step.session)}</span>
        <span class="step-text">${esc(step.prompt_text)}</span>
        <span class="step-actions">
          <button class="step-edit" title="Edit">Ed</button>
          <button class="step-up" title="Move up"${i === 0 ? ' disabled' : ''}>&#9650;</button>
          <button class="step-down" title="Move down"${i === currentScenarioSteps.length - 1 ? ' disabled' : ''}>&#9660;</button>
          <button class="step-del" title="Delete">&#10005;</button>
        </span>`;
      // Wire up buttons
      row.querySelector('.step-edit').addEventListener('click', (e) => {
        e.stopPropagation();
        openStepDialog(i);
      });
      row.querySelector('.step-up').addEventListener('click', (e) => {
        e.stopPropagation();
        if (i > 0) {
          [currentScenarioSteps[i - 1], currentScenarioSteps[i]] =
            [currentScenarioSteps[i], currentScenarioSteps[i - 1]];
          renderStepTimeline();
        }
      });
      row.querySelector('.step-down').addEventListener('click', (e) => {
        e.stopPropagation();
        if (i < currentScenarioSteps.length - 1) {
          [currentScenarioSteps[i], currentScenarioSteps[i + 1]] =
            [currentScenarioSteps[i + 1], currentScenarioSteps[i]];
          renderStepTimeline();
        }
      });
      row.querySelector('.step-del').addEventListener('click', (e) => {
        e.stopPropagation();
        currentScenarioSteps.splice(i, 1);
        renderStepTimeline();
      });
      container.appendChild(row);
    });
  }

  // ── Step dialog ────────────────────────────────────────────────────
  $('#btn-add-step').addEventListener('click', () => openStepDialog(-1));

  function openStepDialog(index) {
    editingStepIndex = index;
    $('#step-dialog-title').textContent = index >= 0 ? 'Edit Step' : 'Add Step';

    // Populate session dropdown from current scenario sessions
    const sessions = [...new Set(currentScenarioSteps.map(s => s.session))];
    if (!sessions.includes('A')) sessions.unshift('A');
    // Always offer next letter
    const allLetters = 'ABCDEFGHIJ'.split('');
    const nextLetter = allLetters.find(l => !sessions.includes(l));
    if (nextLetter) sessions.push(nextLetter);

    const sel = $('#step-session');
    sel.innerHTML = '';
    sessions.forEach(s => {
      const opt = document.createElement('option');
      opt.value = s;
      opt.textContent = `Session ${s}`;
      sel.appendChild(opt);
    });

    // Load library prompts
    loadLibraryDropdown();

    if (index >= 0) {
      const step = currentScenarioSteps[index];
      sel.value = step.session;
      if (step.prompt_id) {
        $('#step-source-type').value = 'library';
        $('#step-prompt-id').value = step.prompt_id;
        $('#step-library-row').style.display = '';
        $('#step-custom-row').style.display = 'none';
      } else {
        $('#step-source-type').value = 'custom';
        $('#step-library-row').style.display = 'none';
        $('#step-custom-row').style.display = '';
      }
      $('#step-prompt-text').value = step.prompt_text;
      $('#step-delay').value = step.delay_ms || 0;
    } else {
      $('#step-form').reset();
      $('#step-source-type').value = 'custom';
      $('#step-library-row').style.display = 'none';
      $('#step-custom-row').style.display = '';
    }
    $('#step-dialog').style.display = 'flex';
  }

  async function loadLibraryDropdown() {
    try {
      const prompts = await API.call('list_prompts', {});
      const sel = $('#step-prompt-id');
      sel.innerHTML = '<option value="">Select prompt...</option>';
      prompts.forEach(p => {
        const opt = document.createElement('option');
        opt.value = p.id;
        opt.textContent = `${p.id} — ${p.text.substring(0, 60)}`;
        sel.appendChild(opt);
      });
    } catch (err) { /* ignore */ }
  }

  $('#step-source-type').addEventListener('change', () => {
    const isLibrary = $('#step-source-type').value === 'library';
    $('#step-library-row').style.display = isLibrary ? '' : 'none';
    $('#step-custom-row').style.display = isLibrary ? 'none' : '';
  });

  // When library prompt selected, fill text
  $('#step-prompt-id').addEventListener('change', async () => {
    const id = $('#step-prompt-id').value;
    if (!id) return;
    try {
      const p = await API.call('get_prompt', { id });
      if (p) $('#step-prompt-text').value = p.text;
    } catch (err) { /* ignore */ }
  });

  $('#step-dialog-close').addEventListener('click', () => {
    $('#step-dialog').style.display = 'none';
  });
  $('#step-cancel').addEventListener('click', () => {
    $('#step-dialog').style.display = 'none';
  });

  $('#step-form').addEventListener('submit', (e) => {
    e.preventDefault();
    const isLibrary = $('#step-source-type').value === 'library';
    const step = {
      session: $('#step-session').value,
      prompt_id: isLibrary ? $('#step-prompt-id').value || null : null,
      prompt_text: $('#step-prompt-text').value,
      delay_ms: parseInt($('#step-delay').value) || 0,
    };
    if (editingStepIndex >= 0) {
      currentScenarioSteps[editingStepIndex] = step;
    } else {
      currentScenarioSteps.push(step);
    }
    $('#step-dialog').style.display = 'none';
    renderStepTimeline();
  });

  // ── Run scenario ───────────────────────────────────────────────────
  $('#btn-run-scenario').addEventListener('click', async () => {
    if (!currentScenarioId) return;
    if (!$('#sc-target').value) {
      toast('Select a target first', 'error');
      return;
    }

    // Save scenario first
    $('#btn-save-scenario').click();
    await new Promise(r => setTimeout(r, 300)); // brief wait for save

    $('#btn-run-scenario').disabled = true;
    $('#btn-stop-scenario').disabled = false;
    $('#scenario-progress').style.display = '';
    $('#sc-progress-bar').style.width = '0%';
    $('#sc-progress-completed').textContent = '0';
    $('#sc-progress-total').textContent = '0';
    $('#sc-progress-errors').textContent = '0';

    try {
      const result = await API.call('start_scenario', {
        scenario_id: currentScenarioId,
        tester_name: $('#sc-tester').value.trim() || 'default',
      });
      currentRunId = result.id || result.run_id;
      toast(`Scenario run complete: ${result.status}`, result.status === 'completed' ? 'success' : 'info');

      // Show results
      if (currentRunId) loadScenarioResults(currentRunId);
    } catch (err) {
      toast(err.message, 'error');
    } finally {
      $('#btn-run-scenario').disabled = false;
      $('#btn-stop-scenario').disabled = true;
      stopProgressPoll();
    }
  });

  $('#btn-stop-scenario').addEventListener('click', async () => {
    try {
      await API.call('stop_run', {});
      toast('Stop requested', 'info');
    } catch (err) { toast(err.message, 'error'); }
  });

  async function loadScenarioResults(runId) {
    try {
      const results = await API.call('get_results', { run_id: runId });
      $('#scenario-results').style.display = '';
      const container = $('#scenario-results-list');
      container.innerHTML = '';
      results.forEach(r => {
        const row = document.createElement('div');
        row.className = 'result-step-row';
        const statusClass = r.error_message ? 'status-error' : 'status-ok';
        const statusText = r.error_message ? 'ERR' : `${r.status_code || '?'}`;
        const session = r.session_label || '?';
        row.innerHTML = `
          <span class="step-num">${r.step_order || '-'}</span>
          <span class="step-session" data-session="${esc(session)}"></span>
          <span class="step-session-label" data-session="${esc(session)}">${esc(session)}</span>
          <span class="result-status ${statusClass}">${statusText}</span>
          <span class="result-preview">${esc((r.response_text || r.error_message || '').substring(0, 120))}</span>`;
        row.addEventListener('click', () => showResultDetail(r));
        container.appendChild(row);
      });
    } catch (err) { toast(err.message, 'error'); }
  }

  // ── Runs view ──────────────────────────────────────────────────────
  async function loadRuns() {
    if (!dbOpen) return;
    try {
      const runs = await API.call('list_runs', {});
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
          <td>${esc(r.started_at || '')}</td>`;
        tr.addEventListener('click', () => loadRunResults(r.id));
        tbody.appendChild(tr);
      });
    } catch (err) { toast(err.message, 'error'); }
  }

  async function loadRunResults(runId) {
    try {
      const results = await API.call('get_results', { run_id: runId });
      $('#run-results-section').style.display = '';
      const tbody = $('#results-tbody');
      tbody.innerHTML = '';
      results.forEach(r => {
        const statusClass = r.error_message ? 'status-error' : 'status-ok';
        const statusText = r.error_message ? 'ERROR' : `${r.status_code || '?'}`;
        const tr = document.createElement('tr');
        tr.className = 'clickable';
        tr.innerHTML = `
          <td>${r.step_order || '-'}</td>
          <td>${esc(r.session_label || '-')}</td>
          <td><code>${esc(r.prompt_id)}</code></td>
          <td><span class="${statusClass}">${statusText}</span></td>
          <td><div class="cell-text">${esc(r.prompt_text)}</div></td>
          <td><div class="cell-text">${esc(r.response_text || '')}</div></td>
          <td>${r.latency_ms != null ? r.latency_ms + 'ms' : '-'}</td>`;
        tr.addEventListener('click', () => showResultDetail(r));
        tbody.appendChild(tr);
      });
    } catch (err) { toast(err.message, 'error'); }
  }

  // ── Result detail modal ────────────────────────────────────────────
  function showResultDetail(r) {
    $('#detail-prompt').textContent = r.prompt_text;
    $('#detail-response').textContent = r.response_text || '(no response)';
    $('#detail-meta').innerHTML = `
      <strong>Prompt ID:</strong> ${esc(r.prompt_id)} &nbsp;|&nbsp;
      <strong>Status:</strong> ${r.status_code || 'N/A'} &nbsp;|&nbsp;
      <strong>Latency:</strong> ${r.latency_ms != null ? r.latency_ms + 'ms' : 'N/A'} &nbsp;|&nbsp;
      <strong>Session:</strong> ${esc(r.session_label || '-')} &nbsp;|&nbsp;
      <strong>Step:</strong> ${r.step_order || '-'}
      ${r.error_message ? '<br><strong>Error:</strong> ' + esc(r.error_message) : ''}`;
    $('#result-detail').style.display = 'flex';
  }

  $('#result-detail-close').addEventListener('click', () => {
    $('#result-detail').style.display = 'none';
  });

  // ── Progress polling ───────────────────────────────────────────────
  function startProgressPoll(runId) {
    stopProgressPoll();
    progressPollTimer = setInterval(async () => {
      try {
        const run = await API.call('get_run_progress', { run_id: runId });
        if (run) {
          const pct = run.total_prompts > 0 ? (run.completed / run.total_prompts * 100) : 0;
          $('#sc-progress-bar').style.width = `${pct}%`;
          $('#sc-progress-completed').textContent = run.completed;
          $('#sc-progress-total').textContent = run.total_prompts;
          $('#sc-progress-errors').textContent = run.errors;
          if (run.status !== 'running') stopProgressPoll();
        }
      } catch { /* ignore */ }
    }, 1000);
  }

  function stopProgressPoll() {
    if (progressPollTimer) {
      clearInterval(progressPollTimer);
      progressPollTimer = null;
    }
  }

  // ── Close modals on backdrop click ─────────────────────────────────
  document.querySelectorAll('.modal').forEach(modal => {
    modal.addEventListener('click', (e) => {
      if (e.target === modal) modal.style.display = 'none';
    });
  });
});
