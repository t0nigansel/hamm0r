/**
 * app.js — Main UI logic for hamm0r.
 *
 * Sidebar + main panel layout with scenario-based testing.
 * All backend communication goes through API.call() (see api.js).
 */

document.addEventListener('DOMContentLoaded', () => {
  // ── State ──────────────────────────────────────────────────────────
  let dbOpen = false;
  let activeEngagementSlug = null;
  let editingPromptId = null;
  let currentScenarioId = null;
  let currentScenarioSteps = []; // local step buffer
  let editingStepIndex = -1; // -1 = add, >= 0 = edit
  let currentRunId = null;
  let progressPollTimer = null;
  const ARCHIVED_ENGAGEMENTS_KEY = 'hamm0r.archivedEngagements.v1';

  const engagementDetail = {
    slug: null,
    name: '',
    activeRunId: null,
    runs: [],
    resultsByRunId: new Map(),
    targets: [],
    scenarios: [],
    targetMatch: null,
    scenarioName: '—',
  };

  // ── DOM refs ───────────────────────────────────────────────────────
  const $ = (sel) => document.querySelector(sel);
  const $$ = (sel) => document.querySelectorAll(sel);

  function esc(str) {
    if (str == null) return '';
    const div = document.createElement('div');
    div.textContent = String(str);
    return div.innerHTML;
  }

  function renderInlineMarkdown(text) {
    let safe = esc(text || '');
    safe = safe.replace(/`([^`]+)`/g, '<code>$1</code>');
    safe = safe.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
    safe = safe.replace(/\*([^*]+)\*/g, '<em>$1</em>');
    safe = safe.replace(
      /\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/g,
      '<a href="$2" target="_blank" rel="noopener noreferrer">$1</a>',
    );
    return safe;
  }

  function splitTableRow(line) {
    let trimmed = String(line || '').trim();
    if (trimmed.startsWith('|')) trimmed = trimmed.slice(1);
    if (trimmed.endsWith('|')) trimmed = trimmed.slice(0, -1);
    return trimmed.split('|').map(c => c.trim());
  }

  function renderMarkdownToHtml(markdown) {
    const lines = String(markdown || '').replace(/\r\n/g, '\n').split('\n');
    const out = [];
    let inUl = false;
    let inOl = false;
    let i = 0;

    function closeLists() {
      if (inUl) {
        out.push('</ul>');
        inUl = false;
      }
      if (inOl) {
        out.push('</ol>');
        inOl = false;
      }
    }

    while (i < lines.length) {
      const line = lines[i];
      const trimmed = line.trim();

      if (trimmed === '') {
        closeLists();
        i += 1;
        continue;
      }

      if (trimmed.startsWith('```')) {
        closeLists();
        const codeLines = [];
        i += 1;
        while (i < lines.length && !lines[i].trim().startsWith('```')) {
          codeLines.push(lines[i]);
          i += 1;
        }
        if (i < lines.length) i += 1;
        out.push(`<pre><code>${esc(codeLines.join('\n'))}</code></pre>`);
        continue;
      }

      const heading = /^(#{1,6})\s+(.+)$/.exec(trimmed);
      if (heading) {
        closeLists();
        const level = heading[1].length;
        out.push(`<h${level}>${renderInlineMarkdown(heading[2])}</h${level}>`);
        i += 1;
        continue;
      }

      if (trimmed.includes('|') && i + 1 < lines.length) {
        const separator = lines[i + 1].trim();
        const isSeparator = /^\|?[\s:-]+(?:\|[\s:-]+)+\|?$/.test(separator);
        if (isSeparator) {
          closeLists();
          const headerCells = splitTableRow(trimmed);
          const rows = [];
          i += 2;
          while (i < lines.length) {
            const row = lines[i].trim();
            if (!row || !row.includes('|')) break;
            rows.push(splitTableRow(row));
            i += 1;
          }
          out.push('<table><thead><tr>');
          headerCells.forEach(c => out.push(`<th>${renderInlineMarkdown(c)}</th>`));
          out.push('</tr></thead><tbody>');
          rows.forEach(r => {
            out.push('<tr>');
            r.forEach(c => out.push(`<td>${renderInlineMarkdown(c)}</td>`));
            out.push('</tr>');
          });
          out.push('</tbody></table>');
          continue;
        }
      }

      const ul = /^[-*]\s+(.+)$/.exec(trimmed);
      if (ul) {
        if (inOl) {
          out.push('</ol>');
          inOl = false;
        }
        if (!inUl) {
          out.push('<ul>');
          inUl = true;
        }
        out.push(`<li>${renderInlineMarkdown(ul[1])}</li>`);
        i += 1;
        continue;
      }

      const ol = /^\d+\.\s+(.+)$/.exec(trimmed);
      if (ol) {
        if (inUl) {
          out.push('</ul>');
          inUl = false;
        }
        if (!inOl) {
          out.push('<ol>');
          inOl = true;
        }
        out.push(`<li>${renderInlineMarkdown(ol[1])}</li>`);
        i += 1;
        continue;
      }

      closeLists();
      out.push(`<p>${renderInlineMarkdown(trimmed)}</p>`);
      i += 1;
    }

    closeLists();
    return out.join('');
  }

  function formatRunStarted(ts) {
    if (!ts) return '';
    const d = new Date(ts);
    if (Number.isNaN(d.getTime())) return ts;
    const y = String(d.getFullYear());
    const m = String(d.getMonth() + 1).padStart(2, '0');
    const day = String(d.getDate()).padStart(2, '0');
    const hh = String(d.getHours()).padStart(2, '0');
    const mm = String(d.getMinutes()).padStart(2, '0');
    return `${y}${m}${day} ${hh}:${mm}`;
  }

  // ── Toast ──────────────────────────────────────────────────────────
  function toast(message, type = 'info') {
    const el = document.createElement('div');
    el.className = `toast toast-${type}`;
    el.textContent = message;
    $('#toast-container').appendChild(el);
    setTimeout(() => el.remove(), 4000);
  }

  // ── Workbench state ────────────────────────────────────────────────
  const wb = {
    activeTargetId: null,
    activeTarget: null,
    availableTargets: [],
    selectedPromptId: null,
    selectedPrompt: null,
    activeCardEl: null,
    baselineCardEl: null,
    baselineResultId: null,
    findings: [],
    allPrompts: [],       // full unfiltered prompt list for coverage grid
  };

  function getBaselineCard() {
    if (wb.baselineResultId) {
      const byId = [...$$('.response-card')].find(c => c.dataset.resultId === wb.baselineResultId);
      if (byId) {
        wb.baselineCardEl = byId;
        return byId;
      }
    }
    if (wb.baselineCardEl && document.body.contains(wb.baselineCardEl)) return wb.baselineCardEl;
    return null;
  }

  function renderDiffEmpty(message) {
    const diffEl = $('#detail-diff');
    diffEl.innerHTML = `<div class="diff-empty">${esc(message)}</div>`;
  }

  function updateBaselineIndicators() {
    const baselineCard = getBaselineCard();
    $$('.response-card').forEach(card => {
      const marker = card.querySelector('[data-baseline-marker]');
      const isBaseline = baselineCard === card;
      card.classList.toggle('baseline', isBaseline);
      if (marker) marker.classList.toggle('hidden', !isBaseline);
    });
  }

  function setBaselineCard(cardEl, notify = true) {
    if (!cardEl) {
      if (notify) toast('Select a response card first', 'info');
      return false;
    }
    const responseText = (cardEl.dataset.responseText || '').trim();
    if (!responseText) {
      if (notify) toast('Selected response has no text to use as baseline', 'error');
      return false;
    }

    wb.baselineCardEl = cardEl;
    wb.baselineResultId = cardEl.dataset.resultId || null;
    updateBaselineIndicators();
    if (notify) toast('Baseline set', 'success');
    return true;
  }

  function setVerdictBadge(verdictEl, verdict) {
    if (!verdictEl) return;
    const normalized = String(verdict || '').toUpperCase();
    if (normalized === 'SUCCESS') {
      verdictEl.textContent = 'success';
      verdictEl.className = 'verdict-badge verdict-success';
    } else if (normalized === 'FAIL') {
      verdictEl.textContent = 'fail';
      verdictEl.className = 'verdict-badge verdict-fail';
    } else if (normalized === 'PARTIAL') {
      verdictEl.textContent = 'partial';
      verdictEl.className = 'verdict-badge verdict-partial';
    } else if (normalized === 'UNCLEAR') {
      verdictEl.textContent = 'unclear';
      verdictEl.className = 'verdict-badge verdict-pending';
    } else {
      verdictEl.textContent = 'pending';
      verdictEl.className = 'verdict-badge verdict-pending';
    }
  }

  function applyJudgeToCard(cardEl, judgeData) {
    if (!cardEl || !judgeData || !judgeData.judge_verdict) return;
    cardEl.dataset.judgeVerdict = judgeData.judge_verdict || '';
    cardEl.dataset.judgeConfidence = judgeData.judge_confidence != null ? String(judgeData.judge_confidence) : '';
    cardEl.dataset.judgeReason = judgeData.judge_reason || '';
    cardEl.dataset.judgeModelUsed = judgeData.judge_model_used || '';
    cardEl.dataset.judgeEvaluatedAt = judgeData.judge_evaluated_at || '';

    setVerdictBadge(cardEl.querySelector('[data-verdict]'), judgeData.judge_verdict);
  }

  async function judgeCard(cardEl, { force = false, switchToJudge = true } = {}) {
    if (!cardEl) {
      toast('Select a response first', 'info');
      return null;
    }
    const resultId = cardEl.dataset.resultId;
    if (!resultId) {
      toast('Selected card has no result ID', 'error');
      return null;
    }

    const payload = { result_id: resultId };
    if (force) payload.force = true;

    const res = await API.call('judge_result', payload);
    applyJudgeToCard(cardEl, res);

    if (wb.activeCardEl === cardEl) {
      const signals = JSON.parse(cardEl.dataset.signals || '[]');
      renderJudgePanel(cardEl, signals);
      if (switchToJudge) switchDetailTab('judge');
    }

    if (res.status === 'skipped') toast('Already judged (use re-judge to refresh)', 'info');
    else toast(`Judge verdict: ${res.judge_verdict || 'UNCLEAR'}`, 'success');

    return res;
  }

  function renderJudgePanel(cardEl, signals) {
    const judgeContent = $('#judge-content');
    if (!cardEl) {
      judgeContent.textContent = 'select a response to inspect';
      return;
    }

    const resultId = cardEl.dataset.resultId || '—';
    const judgedVerdict = cardEl.dataset.judgeVerdict || '';
    const judgedReason = cardEl.dataset.judgeReason || '';
    const judgedModel = cardEl.dataset.judgeModelUsed || '';
    const judgedAt = cardEl.dataset.judgeEvaluatedAt || '';
    const confidenceRaw = Number(cardEl.dataset.judgeConfidence || '');
    const confidence = Number.isFinite(confidenceRaw) ? `${Math.round(confidenceRaw * 100)}%` : 'n/a';

    if (!judgedVerdict) {
      const signalSummary = signals.length
        ? `${signals.length} signal${signals.length === 1 ? '' : 's'} detected`
        : 'No signals detected';
      judgeContent.innerHTML = `<div class="judge-block">
        <div class="judge-header">model judge</div>
        <div class="judge-verdict">Result ${esc(resultId)} has not been judged yet.</div>
        <div class="judge-reason">${esc(signalSummary)}</div>
        <div class="response-actions" style="margin-top:10px;">
          <button class="ra-btn ra-promote" id="judge-run-btn">Run Judge</button>
        </div>
      </div>`;
      $('#judge-run-btn')?.addEventListener('click', async () => {
        const btn = $('#judge-run-btn');
        if (btn) btn.disabled = true;
        try {
          await judgeCard(cardEl, { force: false, switchToJudge: true });
        } catch (err) {
          toast(`Judge failed: ${err.message}`, 'error');
        } finally {
          if (btn) btn.disabled = false;
        }
      });
      return;
    }

    judgeContent.innerHTML = `<div class="judge-block">
      <div class="judge-header">model judge</div>
      <div class="judge-verdict">Verdict: ${esc(judgedVerdict)} · confidence: ${esc(confidence)}</div>
      <div class="judge-reason">${esc(judgedReason || 'No reason provided')}</div>
      <div class="detail-meta" style="margin-top:8px;">model: ${esc(judgedModel || 'heuristic')} · evaluated: ${esc(judgedAt || '—')}</div>
      <div class="response-actions" style="margin-top:10px;">
        <button class="ra-btn" id="judge-rerun-btn">Re-judge</button>
      </div>
    </div>`;
    $('#judge-rerun-btn')?.addEventListener('click', async () => {
      const btn = $('#judge-rerun-btn');
      if (btn) btn.disabled = true;
      try {
        await judgeCard(cardEl, { force: true, switchToJudge: true });
      } catch (err) {
        toast(`Re-judge failed: ${err.message}`, 'error');
      } finally {
        if (btn) btn.disabled = false;
      }
    });
  }

  function resetDetailPane() {
    renderDiffEmpty('select a response card to compare with baseline');
    $('#signals-list').innerHTML = '<div style="color:var(--text-3);font-family:var(--mono);font-size:11px;text-align:center;padding:40px 20px;">no signals detected</div>';
    $('#signals-badge').textContent = '0';
    $('#detail-raw-pre').textContent = '—';
    renderJudgePanel(null, []);
    switchDetailTab('diff');
  }

  function setWorkbenchInteractivity(enabled) {
    $('#btn-wb-fire').disabled = !enabled;
    $('#btn-wb-baseline').disabled = !enabled;
    $('#btn-wb-judge-all').disabled = !enabled;
    $('#wb-prompt-textarea').disabled = !enabled;
    $('#wb-prompt-textarea').placeholder = enabled
      ? 'select a prompt from the library or type an attack prompt…'
      : 'pick a target first…';
  }

  function renderWorkbenchEmptyState() {
    const stream = $('#wb-response-stream');
    let empty = $('#wb-response-empty');
    if (!empty) {
      empty = document.createElement('div');
      empty.id = 'wb-response-empty';
      empty.className = 'response-empty';
      stream.appendChild(empty);
    }

    if (!wb.activeTargetId) {
      empty.innerHTML = `
        <div class="response-empty-title">Pick Target</div>
        <div class="response-empty-subtitle">Select a target to start working in Workbench.</div>
        <div class="response-empty-actions">
          <button class="btn btn-primary" id="wb-empty-pick-target">Pick Target</button>
          <a href="#" class="response-empty-banner" id="wb-empty-start-engagement">Looking for guided testing? Start an Engagement →</a>
        </div>`;
      $('#wb-empty-pick-target')?.addEventListener('click', () => openWorkbenchTargetDialog());
      $('#wb-empty-start-engagement')?.addEventListener('click', (e) => {
        e.preventDefault();
        openEngagementWizard();
      });
      return;
    }

    empty.innerHTML = `<div class="response-empty-title">Ready</div><div class="response-empty-subtitle">fire a prompt to begin</div>`;
  }

  function resetResponseStream() {
    const stream = $('#wb-response-stream');
    stream.querySelectorAll('.response-card').forEach(el => el.remove());
    renderWorkbenchEmptyState();
  }
  setWorkbenchInteractivity(false);
  renderWorkbenchEmptyState();

  // ── Check if DB is open on page load ────────────────────────────────
  async function checkDatabaseStatus() {
    try {
      const status = await API.call('get_db_status', {});
      if (status.open && status.slug) {
        dbOpen = true;
        activeEngagementSlug = status.slug;
        onDbOpen(status.name || status.slug, status.slug);
      }
    } catch (err) {
      // Not running inside Tauri yet — ok in dev
    } finally {
      // Always load global data (targets + prompts live outside engagements)
      loadWorkbenchTargets();
      loadPickerPrompts();
      loadHomeRecentEngagements();
    }
  }

  function onDbOpen(name, slug) {
    if (slug) activeEngagementSlug = slug;
    $('#db-label').textContent = name;
    $('#breadcrumb-engagement').textContent = name;
    $('#engagement-dot').classList.add('active');
    wb.activeCardEl = null;
    wb.baselineCardEl = null;
    wb.baselineResultId = null;
    updateBaselineIndicators();
    resetResponseStream();
    resetDetailPane();
    loadTargetList();
    loadWorkbenchTargets();
    loadPickerPrompts();   // also updates coverage grid client-side
    loadFindings();
    if ($('#view-home').classList.contains('active')) {
      loadHomeRecentEngagements();
    }
    // refresh engagement list if runs view is visible
    if ($('#view-runs').classList.contains('active')) loadEngagementList({ autoOpen: false });
  }

  checkDatabaseStatus();

  // ── T-05 · Sidebar navigation ──────────────────────────────────────
  const VIEW_LABELS = {
    'view-home': 'home',
    'view-workbench': 'workbench',
    'view-targets': 'targets',
    'view-library': 'library',
    'view-scenarios': 'scenarios',
    'view-runs': 'runs',
  };

  function showView(viewId) {
    $$('.nav-item[data-view]').forEach(b => b.classList.remove('active'));
    $$('.view').forEach(v => v.classList.remove('active'));

    const btn = $(`.nav-item[data-view="${viewId}"]`);
    if (btn) btn.classList.add('active');
    const viewEl = $(`#${viewId}`);
    if (viewEl) viewEl.classList.add('active');

    $('#breadcrumb-view').textContent = VIEW_LABELS[viewId] || viewId;


    if (viewId === 'view-home') loadHomeRecentEngagements();
    if (viewId === 'view-library') loadPrompts();
    if (viewId === 'view-targets') loadTargetList();
    if (viewId === 'view-workbench') { loadWorkbenchTargets(); loadPickerPrompts(); if (dbOpen) loadFindings(); }
    if (viewId === 'view-runs') loadEngagementList();
    if (viewId !== 'view-runs') clearEngagementRoute({ replace: true });
    if (dbOpen) {
      if (viewId === 'view-scenarios') loadScenarioList();
    }
  }

  $$('.nav-item[data-view]').forEach(btn => {
    btn.addEventListener('click', () => showView(btn.dataset.view));
  });

  // Settings nav item (no view yet)
  const settingsNavBtn = $('.nav-item[data-nav="settings"]');
  if (settingsNavBtn) {
    settingsNavBtn.addEventListener('click', () => toast('Settings coming soon', 'info'));
  }

  // ── Engagement management ──────────────────────────────────────────
  const WIZARD_STORAGE_KEY = 'hamm0r.engagementWizard.v1';
  const WIZARD_SCENARIOS = [
    {
      id: 'quick_scan',
      name: 'Quick Scan',
      description: 'Small OWASP sample, single-shot, optimized for first signal.',
      estimatedPrompts: 20,
      estimatedRuntime: '~3m',
      coverage: ['A01', 'A02', 'A03', 'A06'],
      defaultOwaspRefs: ['A01', 'A02', 'A03', 'A06'],
      defaultCustomLibraries: [],
    },
    {
      id: 'owasp_full',
      name: 'OWASP LLM Top 10 Full',
      description: 'Complete A01–A10 coverage for broader baseline validation.',
      estimatedPrompts: 120,
      estimatedRuntime: '~18m',
      coverage: ['A01', 'A02', 'A03', 'A04', 'A05', 'A06', 'A07', 'A08', 'A09', 'A10'],
      defaultOwaspRefs: ['A01', 'A02', 'A03', 'A04', 'A05', 'A06', 'A07', 'A08', 'A09', 'A10'],
      defaultCustomLibraries: [],
    },
    {
      id: 'injection_deep_dive',
      name: 'Prompt Injection Deep Dive',
      description: 'A01-focused run with higher depth and mutation-like pressure.',
      estimatedPrompts: 48,
      estimatedRuntime: '~9m',
      coverage: ['A01'],
      defaultOwaspRefs: ['A01'],
      defaultCustomLibraries: ['injection-classics'],
    },
    {
      id: 'jailbreak_battery',
      name: 'Jailbreak Battery',
      description: 'Curated jailbreak-focused prompts for refusal and bypass checks.',
      estimatedPrompts: 36,
      estimatedRuntime: '~7m',
      coverage: ['A01', 'A07'],
      defaultOwaspRefs: ['A01', 'A07'],
      defaultCustomLibraries: [],
    },
    {
      id: 'custom',
      name: 'Custom',
      description: 'Pick everything manually.',
      estimatedPrompts: 0,
      estimatedRuntime: 'custom',
      coverage: [],
      defaultOwaspRefs: [],
      defaultCustomLibraries: [],
    },
  ];

  function defaultWizardState() {
    return {
      step: 1,
      targetMode: 'existing',
      existingTargetId: '',
      newTarget: {
        name: '',
        baseUrl: '',
        protocol: 'openai_compat',
        auth: 'none',
        authEnv: '',
        authHeader: '',
        sessionHandling: 'none',
        sessionField: '',
      },
      tested: {
        ok: false,
        message: 'Run a target check before continuing.',
      },
      scenarioId: 'quick_scan',
      selectedOwaspRefs: ['A01', 'A02', 'A03', 'A06'],
      selectedCustomLibraries: [],
      engagementName: '',
      saveAsTemplate: false,
    };
  }

  function loadWizardState() {
    try {
      const raw = localStorage.getItem(WIZARD_STORAGE_KEY);
      if (!raw) return defaultWizardState();
      const parsed = JSON.parse(raw);
      return {
        ...defaultWizardState(),
        ...parsed,
        newTarget: {
          ...defaultWizardState().newTarget,
          ...(parsed.newTarget || {}),
        },
        tested: {
          ...defaultWizardState().tested,
          ...(parsed.tested || {}),
        },
      };
    } catch (_err) {
      return defaultWizardState();
    }
  }

  let wizardState = loadWizardState();
  const wizardCatalog = {
    targets: [],
    prompts: [],
    customLibraries: [],
  };

  function persistWizardState() {
    try {
      localStorage.setItem(WIZARD_STORAGE_KEY, JSON.stringify(wizardState));
    } catch (_err) {
      // ignore persistence failures silently
    }
  }

  function resetWizardState() {
    wizardState = defaultWizardState();
    try {
      localStorage.removeItem(WIZARD_STORAGE_KEY);
    } catch (_err) {
      // ignore
    }
  }

  function wizardScenarioById(id) {
    return WIZARD_SCENARIOS.find(s => s.id === id) || WIZARD_SCENARIOS[0];
  }

  function clearWizardTestStatus() {
    wizardState.tested = {
      ok: false,
      message: 'Run a target check before continuing.',
    };
  }

  function applyWizardScenarioDefaults(scenarioId, force = false) {
    const scenario = wizardScenarioById(scenarioId);
    if (!scenario) return;

    if (force || (wizardState.selectedOwaspRefs.length === 0 && wizardState.selectedCustomLibraries.length === 0)) {
      wizardState.selectedOwaspRefs = [...scenario.defaultOwaspRefs];
      wizardState.selectedCustomLibraries = [...scenario.defaultCustomLibraries];
    }
  }

  function wizardSelectedPrompts() {
    return wizardCatalog.prompts.filter((p) => {
      const inOwasp = wizardState.selectedOwaspRefs.includes(p.owasp_ref);
      const inCustom = wizardState.selectedCustomLibraries.includes(p.category || '');
      return inOwasp || inCustom;
    });
  }

  function wizardTargetDisplayName() {
    if (wizardState.targetMode === 'existing') {
      const t = wizardCatalog.targets.find(x => x.id === wizardState.existingTargetId);
      return t?.name || 'not selected';
    }
    return wizardState.newTarget.name || 'new target';
  }

  function wizardEstimatedRuntime(promptCount) {
    const minutes = Math.max(1, Math.ceil(promptCount / 7));
    return `~${minutes}m`;
  }

  function wizardEstimatedCost(promptCount) {
    // rough UX estimate only (input/output + judge combined)
    const low = (promptCount * 0.002).toFixed(2);
    const high = (promptCount * 0.008).toFixed(2);
    return `~$${low} - $${high}`;
  }

  function wizardSetStep(step) {
    wizardState.step = Math.max(1, Math.min(4, Number(step) || 1));
    persistWizardState();
    renderWizard();
  }

  function wizardProtocolToEndpoint(protocol) {
    if (protocol === 'openai_compat') return 'openai_compat';
    return 'custom_rest';
  }

  function wizardAuthToApi(auth) {
    if (auth === 'custom_header') return 'api_key';
    return auth;
  }

  function wizardSuggestEngagementName() {
    const scenario = wizardScenarioById(wizardState.scenarioId);
    return `${wizardTargetDisplayName()} · ${scenario.name}`;
  }

  async function loadWizardCatalog() {
    const [targets, prompts] = await Promise.all([
      API.call('list_targets', {}),
      API.call('list_prompts', {}),
    ]);

    wizardCatalog.targets = targets || [];
    wizardCatalog.prompts = prompts || [];
    wizardCatalog.customLibraries = [...new Set((prompts || []).map(p => p.category).filter(Boolean))].sort();

    if (wizardState.targetMode === 'existing' && !wizardState.existingTargetId && wizardCatalog.targets.length > 0) {
      wizardState.existingTargetId = wizardCatalog.targets[0].id;
    }

    if (!wizardState.engagementName) {
      wizardState.engagementName = wizardSuggestEngagementName();
    }
    persistWizardState();
  }

  function renderWizardStep1() {
    const existingBtn = $('#wizard-target-mode-existing');
    const newBtn = $('#wizard-target-mode-new');
    const existingRow = $('#wizard-existing-row');
    const newRow = $('#wizard-new-row');
    const existingSelect = $('#wizard-existing-target');

    existingBtn.classList.toggle('active', wizardState.targetMode === 'existing');
    newBtn.classList.toggle('active', wizardState.targetMode === 'new');
    existingRow.style.display = wizardState.targetMode === 'existing' ? '' : 'none';
    newRow.style.display = wizardState.targetMode === 'new' ? '' : 'none';

    existingSelect.innerHTML = '';
    if (wizardCatalog.targets.length === 0) {
      existingSelect.innerHTML = '<option value="">No targets saved yet</option>';
    } else {
      wizardCatalog.targets.forEach((t) => {
        const opt = document.createElement('option');
        opt.value = t.id;
        opt.textContent = t.name;
        existingSelect.appendChild(opt);
      });
    }
    existingSelect.value = wizardState.existingTargetId || existingSelect.value || '';

    $('#wizard-new-target-name').value = wizardState.newTarget.name;
    $('#wizard-new-base-url').value = wizardState.newTarget.baseUrl;
    $('#wizard-new-protocol').value = wizardState.newTarget.protocol;
    $('#wizard-new-auth').value = wizardState.newTarget.auth;
    $('#wizard-new-auth-env').value = wizardState.newTarget.authEnv;
    $('#wizard-new-auth-header').value = wizardState.newTarget.authHeader;
    $('#wizard-new-session').value = wizardState.newTarget.sessionHandling;
    $('#wizard-new-session-field').value = wizardState.newTarget.sessionField;

    const auth = wizardState.newTarget.auth;
    $('#wizard-new-auth-env-row').style.display = auth === 'none' ? 'none' : '';
    $('#wizard-new-auth-header-row').style.display = auth === 'custom_header' ? '' : 'none';

    const sessionHandling = wizardState.newTarget.sessionHandling;
    $('#wizard-new-session-row').style.display = sessionHandling === 'none' ? 'none' : '';

    const note = $('#wizard-test-status');
    note.textContent = wizardState.tested.message;
    note.classList.remove('ok', 'error');
    note.classList.add(wizardState.tested.ok ? 'ok' : (wizardState.tested.message.includes('failed') ? 'error' : ''));
  }

  function renderWizardStep2() {
    const grid = $('#wizard-scenario-cards');
    grid.innerHTML = '';
    WIZARD_SCENARIOS.forEach((scenario) => {
      const card = document.createElement('button');
      card.type = 'button';
      card.className = `wizard-scenario-card${wizardState.scenarioId === scenario.id ? ' active' : ''}`;
      const badges = scenario.coverage.length
        ? scenario.coverage.map(c => `<span class="wizard-badge">${esc(c)}</span>`).join('')
        : '<span class="wizard-badge">manual</span>';
      card.innerHTML = `
        <h4>${esc(scenario.name)}</h4>
        <p>${esc(scenario.description)}</p>
        <div class="wizard-scenario-meta">
          <span>${scenario.estimatedPrompts ? `${scenario.estimatedPrompts} prompts` : 'custom size'}</span>
          <span>${esc(scenario.estimatedRuntime)}</span>
        </div>
        <div class="wizard-scenario-badges">${badges}</div>
      `;
      card.addEventListener('click', () => {
        wizardState.scenarioId = scenario.id;
        applyWizardScenarioDefaults(scenario.id, true);
        clearWizardTestStatus();
        if (!wizardState.engagementName || wizardState.engagementName === wizardSuggestEngagementName()) {
          wizardState.engagementName = wizardSuggestEngagementName();
        }
        persistWizardState();
        renderWizard();
      });
      grid.appendChild(card);
    });
  }

  function renderWizardStep3() {
    const owaspWrap = $('#wizard-lib-owasp');
    const customWrap = $('#wizard-lib-custom');
    const refs = ['A01', 'A02', 'A03', 'A04', 'A05', 'A06', 'A07', 'A08', 'A09', 'A10'];

    owaspWrap.innerHTML = '';
    refs.forEach((ref) => {
      const label = document.createElement('label');
      label.className = 'wizard-check';
      label.innerHTML = `<input type="checkbox" value="${ref}" ${wizardState.selectedOwaspRefs.includes(ref) ? 'checked' : ''}><span>${ref}</span>`;
      label.querySelector('input').addEventListener('change', (e) => {
        if (e.target.checked) wizardState.selectedOwaspRefs.push(ref);
        else wizardState.selectedOwaspRefs = wizardState.selectedOwaspRefs.filter(x => x !== ref);
        wizardState.selectedOwaspRefs = [...new Set(wizardState.selectedOwaspRefs)];
        persistWizardState();
        updateWizardLibraryCounter();
      });
      owaspWrap.appendChild(label);
    });

    customWrap.innerHTML = '';
    if (wizardCatalog.customLibraries.length === 0) {
      customWrap.innerHTML = '<span class="wizard-inline-note">No custom libraries found.</span>';
    } else {
      wizardCatalog.customLibraries.forEach((lib) => {
        const label = document.createElement('label');
        label.className = 'wizard-check';
        label.innerHTML = `<input type="checkbox" value="${esc(lib)}" ${wizardState.selectedCustomLibraries.includes(lib) ? 'checked' : ''}><span>${esc(lib)}</span>`;
        label.querySelector('input').addEventListener('change', (e) => {
          if (e.target.checked) wizardState.selectedCustomLibraries.push(lib);
          else wizardState.selectedCustomLibraries = wizardState.selectedCustomLibraries.filter(x => x !== lib);
          wizardState.selectedCustomLibraries = [...new Set(wizardState.selectedCustomLibraries)];
          persistWizardState();
          updateWizardLibraryCounter();
        });
        customWrap.appendChild(label);
      });
    }

    updateWizardLibraryCounter();
  }

  function updateWizardLibraryCounter() {
    const selected = wizardSelectedPrompts();
    $('#wizard-lib-counter').textContent = `${selected.length} prompts selected, ${selected.length} judged`;
  }

  function renderWizardStep4() {
    const selectedPrompts = wizardSelectedPrompts();
    const review = $('#wizard-review-list');
    const scenario = wizardScenarioById(wizardState.scenarioId);
    const refs = wizardState.selectedOwaspRefs.join(', ') || 'none';
    const custom = wizardState.selectedCustomLibraries.join(', ') || 'none';
    const estRuntime = wizardEstimatedRuntime(selectedPrompts.length);
    const estCost = wizardEstimatedCost(selectedPrompts.length);

    review.innerHTML = `
      <div class="wizard-review-row"><span class="k">Target</span><span class="v">${esc(wizardTargetDisplayName())}</span></div>
      <div class="wizard-review-row"><span class="k">Scenario</span><span class="v">${esc(scenario.name)}</span></div>
      <div class="wizard-review-row"><span class="k">Libraries</span><span class="v">${esc(`${refs} · ${custom}`)}</span></div>
      <div class="wizard-review-row"><span class="k">Prompts</span><span class="v">${selectedPrompts.length}</span></div>
      <div class="wizard-review-row"><span class="k">Estimated Runtime</span><span class="v">${esc(estRuntime)}</span></div>
      <div class="wizard-review-row"><span class="k">Estimated Cost</span><span class="v">${esc(estCost)}</span></div>
    `;
    $('#wizard-engagement-name').value = wizardState.engagementName || wizardSuggestEngagementName();
    $('#wizard-save-template').checked = !!wizardState.saveAsTemplate;
  }

  function renderWizard() {
    const step = wizardState.step;
    $$('.wizard-step').forEach((el, idx) => el.classList.toggle('active', idx + 1 === step));
    $$('.wizard-progress-item').forEach((el, idx) => el.classList.toggle('active', idx + 1 === step));

    $('#wizard-prev').style.visibility = step === 1 ? 'hidden' : 'visible';
    $('#wizard-next').style.display = step === 4 ? 'none' : '';
    $('#wizard-fire').style.display = step === 4 ? '' : 'none';

    if (step === 1) renderWizardStep1();
    if (step === 2) renderWizardStep2();
    if (step === 3) renderWizardStep3();
    if (step === 4) renderWizardStep4();
  }

  async function openEngagementWizard() {
    $('#wizard-modal').style.display = 'flex';
    try {
      await loadWizardCatalog();
      applyWizardScenarioDefaults(wizardState.scenarioId, false);
      renderWizard();
    } catch (err) {
      toast(`Wizard load failed: ${err.message}`, 'error');
    }
  }

  function closeEngagementWizard() {
    $('#wizard-modal').style.display = 'none';
    persistWizardState();
  }

  function validateWizardTargetStep() {
    if (wizardState.targetMode === 'existing') {
      if (!wizardState.existingTargetId) {
        wizardState.tested = { ok: false, message: 'Connection test failed: pick an existing target first.' };
        persistWizardState();
        renderWizard();
        return false;
      }
      wizardState.tested = { ok: true, message: 'Connection test passed: existing target configuration looks valid.' };
      persistWizardState();
      renderWizard();
      return true;
    }

    const target = wizardState.newTarget;
    if (!target.name.trim()) {
      wizardState.tested = { ok: false, message: 'Connection test failed: target name is required.' };
      persistWizardState();
      renderWizard();
      return false;
    }
    try {
      // eslint-disable-next-line no-new
      new URL(target.baseUrl);
    } catch (_err) {
      wizardState.tested = { ok: false, message: 'Connection test failed: base URL is invalid.' };
      persistWizardState();
      renderWizard();
      return false;
    }
    if (target.auth !== 'none' && !target.authEnv.trim()) {
      wizardState.tested = { ok: false, message: 'Connection test failed: auth env var is required.' };
      persistWizardState();
      renderWizard();
      return false;
    }
    if (target.auth === 'custom_header' && !target.authHeader.trim()) {
      wizardState.tested = { ok: false, message: 'Connection test failed: custom header name is required.' };
      persistWizardState();
      renderWizard();
      return false;
    }
    wizardState.tested = { ok: true, message: 'Connection test passed: new target configuration looks valid.' };
    persistWizardState();
    renderWizard();
    return true;
  }

  function wizardCanContinueFromStep() {
    if (wizardState.step === 1) {
      if (!wizardState.tested.ok) {
        toast('Run "Test connection" before continuing', 'info');
        return false;
      }
    }
    if (wizardState.step === 3) {
      if (wizardSelectedPrompts().length === 0) {
        toast('Select at least one library before continuing', 'info');
        return false;
      }
    }
    return true;
  }

  async function fireWizardEngagement() {
    const fireBtn = $('#wizard-fire');
    const originalText = fireBtn.textContent;
    fireBtn.disabled = true;
    fireBtn.textContent = 'Starting…';

    try {
      const selectedPrompts = wizardSelectedPrompts();
      if (selectedPrompts.length === 0) {
        throw new Error('No prompts selected for this engagement.');
      }

      let target = null;
      if (wizardState.targetMode === 'existing') {
        target = wizardCatalog.targets.find(t => t.id === wizardState.existingTargetId) || null;
        if (!target) throw new Error('Selected target was not found.');
      } else {
        const nt = wizardState.newTarget;
        const targetId = `target-${Date.now().toString(36)}`;
        target = await API.call('save_target', {
          id: targetId,
          name: nt.name.trim(),
          url: nt.baseUrl.trim(),
          endpoint_type: wizardProtocolToEndpoint(nt.protocol),
          auth_type: wizardAuthToApi(nt.auth),
          auth_env: nt.authEnv.trim() || null,
          auth_header: nt.authHeader.trim() || null,
          session_strategy: nt.sessionHandling,
          session_field: nt.sessionField.trim() || null,
          request_field: nt.protocol === 'openai_compat' ? null : 'prompt',
          response_field: nt.protocol === 'openai_compat' ? null : 'response',
          notes: nt.protocol === 'anthropic'
            ? 'Wizard protocol anthropic mapped to custom_rest adapter.'
            : null,
        });
        wizardCatalog.targets = [target, ...wizardCatalog.targets.filter(t => t.id !== target.id)];
      }

      const scenario = wizardScenarioById(wizardState.scenarioId);
      const engagementName = (wizardState.engagementName || '').trim() || `${target.name} · ${scenario.name}`;
      const created = await API.call('create_engagement', { name: engagementName });
      await API.call('open_db', { path: created.slug });
      dbOpen = true;
      onDbOpen(created.name || engagementName, created.slug);

      if (wizardState.saveAsTemplate) {
        try {
          const templateScenario = await API.call('create_scenario', { name: `${scenario.name} Template` });
          await API.call('update_scenario', {
            id: templateScenario.id,
            name: `${scenario.name} Template`,
            target_id: target.id,
            repeat_count: 1,
          });
          await API.call('save_steps', {
            scenario_id: templateScenario.id,
            steps: selectedPrompts.map((p) => ({
              session: 'A',
              prompt_id: p.id,
              prompt_text: p.text,
              delay_ms: 0,
            })),
          });
        } catch (_err) {
          toast('Engagement started, but template save failed.', 'info');
        }
      }

      const payloads = selectedPrompts.map((p, idx) => ({
        prompt_id: p.id,
        payload_id: `wizard-${String(idx + 1).padStart(3, '0')}`,
        text: p.text,
      }));

      const runId = await API.call('start_run', {
        request_id: target.id,
        payloads,
        parallelism: 4,
      });

      closeEngagementWizard();
      resetWizardState();
      showView('view-runs');
      $('#runs-section-title').textContent = created.name || engagementName;
      $('#runs-section').style.display = '';
      $('#runs-empty').style.display = 'none';
      $('#run-results-section').style.display = 'none';
      loadEngagementList();
      loadRuns();
      toast(`Engagement started (${runId})`, 'success');
    } catch (err) {
      toast(err.message, 'error');
    } finally {
      fireBtn.disabled = false;
      fireBtn.textContent = originalText;
    }
  }

  async function openEngagementDialog() {
    $('#engagement-dialog').style.display = 'flex';
    const list = $('#engagement-list');
    list.innerHTML = '<div class="eng-list-empty">loading…</div>';
    try {
      const engagements = (await API.call('list_engagements', {}))
        .filter(eng => !isEngagementArchived(eng.slug));
      list.innerHTML = '';
      if (engagements.length === 0) {
        list.innerHTML = '<div class="eng-list-empty">no engagements yet — create one below</div>';
      } else {
        engagements.forEach(eng => {
          const card = document.createElement('div');
          card.className = 'engagement-card';
          const date = eng.created_at ? eng.created_at.substring(0, 10) : '';
          card.innerHTML = `
            <span class="eng-name">${esc(eng.name)}</span>
            <span class="eng-meta">${esc(eng.slug)}${date ? ' · ' + esc(date) : ''}</span>`;
          card.addEventListener('click', async () => {
            try {
              const result = await API.call('open_db', { path: eng.slug });
              dbOpen = true;
              $('#engagement-dialog').style.display = 'none';
              onDbOpen(result.name || eng.name, result.slug);
              toast(`Opened: ${result.name || eng.name}`, 'success');
            } catch (err) { toast(err.message, 'error'); }
          });
          list.appendChild(card);
        });
      }
    } catch (err) {
      list.innerHTML = '<div class="eng-list-empty">could not load engagements</div>';
      toast(err.message, 'error');
    }
  }

  function loadArchivedEngagementSlugs() {
    try {
      const raw = localStorage.getItem(ARCHIVED_ENGAGEMENTS_KEY);
      if (!raw) return [];
      const parsed = JSON.parse(raw);
      return Array.isArray(parsed) ? parsed.filter(Boolean) : [];
    } catch (_err) {
      return [];
    }
  }

  const archivedEngagementSlugs = new Set(loadArchivedEngagementSlugs());

  function saveArchivedEngagementSlugs() {
    try {
      localStorage.setItem(ARCHIVED_ENGAGEMENTS_KEY, JSON.stringify([...archivedEngagementSlugs]));
    } catch (_err) {
      // ignore storage errors
    }
  }

  function isEngagementArchived(slug) {
    return !!slug && archivedEngagementSlugs.has(slug);
  }

  function archiveEngagementSlug(slug) {
    if (!slug) return;
    archivedEngagementSlugs.add(slug);
    saveArchivedEngagementSlugs();
  }

  function getEngagementSlugFromRoute() {
    const path = window.location.pathname || '/';
    const match = path.match(/\/engagements\/([^/]+)$/);
    if (!match) return null;
    try {
      return decodeURIComponent(match[1]);
    } catch (_err) {
      return match[1];
    }
  }

  function setEngagementRoute(slug, { replace = false } = {}) {
    if (!slug || !window.history?.pushState) return;
    const nextPath = `/engagements/${encodeURIComponent(slug)}`;
    if (window.location.pathname === nextPath) return;
    const method = replace ? 'replaceState' : 'pushState';
    window.history[method]({ engagementSlug: slug }, '', nextPath);
  }

  function clearEngagementRoute({ replace = false } = {}) {
    if (!window.history?.pushState) return;
    if (!getEngagementSlugFromRoute()) return;
    const method = replace ? 'replaceState' : 'pushState';
    window.history[method]({}, '', '/');
  }

  function normalizeLandingStatus(runs) {
    if (!runs || runs.length === 0) return { label: 'No Runs', css: 'none' };
    const latest = runs[0];
    const status = String(latest.status || '').toLowerCase();
    if (status === 'running') return { label: 'Running', css: 'running' };
    if (status === 'completed') return { label: 'Done', css: 'done' };
    if (status === 'crashed' || status === 'aborted') return { label: 'Failed', css: 'failed' };
    return { label: status || 'Unknown', css: 'none' };
  }

  function formatLandingDate(iso) {
    if (!iso) return 'unknown date';
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return iso;
    const y = String(d.getFullYear());
    const m = String(d.getMonth() + 1).padStart(2, '0');
    const day = String(d.getDate()).padStart(2, '0');
    return `${y}-${m}-${day}`;
  }

  async function quickResumeEngagement(eng) {
    showView('view-runs');
    await openEngagementDetail(eng, { syncRoute: true });
    toast(`Resumed: ${eng.name}`, 'success');
  }

  async function loadHomeRecentEngagements() {
    const list = $('#home-recent-list');
    if (!list) return;

    list.innerHTML = '<div class="landing-empty">loading recent engagements…</div>';
    try {
      const engagements = (await API.call('list_engagements', {}))
        .filter(eng => !isEngagementArchived(eng.slug));
      const recent = [...engagements]
        .sort((a, b) => (b.created_at || '').localeCompare(a.created_at || ''))
        .slice(0, 5);

      if (recent.length === 0) {
        list.innerHTML = '<div class="landing-empty">no engagements yet — start your first one</div>';
        return;
      }

      const rows = await Promise.all(recent.map(async (eng) => {
        try {
          const runs = await API.call('list_runs', { engagement_slug: eng.slug });
          return { eng, runs };
        } catch (err) {
          return { eng, runs: [] };
        }
      }));

      list.innerHTML = '';
      rows.forEach(({ eng, runs }) => {
        const status = normalizeLandingStatus(runs);
        const row = document.createElement('div');
        row.className = 'landing-recent-row';
        row.innerHTML = `
          <div class="landing-recent-main">
            <span class="landing-recent-name">${esc(eng.name)}</span>
            <span class="landing-recent-meta">${esc(formatLandingDate(eng.created_at))} · ${runs.length} run${runs.length === 1 ? '' : 's'}</span>
          </div>
          <span class="landing-status ${status.css}">${esc(status.label)}</span>
          <button class="btn btn-ghost btn-home-resume" type="button">Resume</button>
        `;
        row.querySelector('.landing-recent-main').addEventListener('click', () => {
          quickResumeEngagement(eng).catch(err => toast(err.message, 'error'));
        });
        row.querySelector('.btn-home-resume').addEventListener('click', () => {
          quickResumeEngagement(eng).catch(err => toast(err.message, 'error'));
        });
        list.appendChild(row);
      });
    } catch (err) {
      list.innerHTML = '<div class="landing-empty">could not load recent engagements</div>';
      toast(err.message, 'error');
    }
  }

  $('#btn-new-engagement').addEventListener('click', openEngagementWizard);
  $('#btn-open-engagement').addEventListener('click', openEngagementDialog);
  $('#btn-home-start-engagement')?.addEventListener('click', openEngagementWizard);
  $('#btn-home-open-workbench')?.addEventListener('click', () => showView('view-workbench'));
  $('#btn-home-refresh-recents')?.addEventListener('click', () => {
    loadHomeRecentEngagements();
  });
  $('#btn-help')?.addEventListener('click', () => {
    toast('I believe in you. Swing again.', 'info');
  });
  $('#engagement-dialog-close').addEventListener('click', () => {
    $('#engagement-dialog').style.display = 'none';
  });
  $('#engagement-dialog-cancel').addEventListener('click', () => {
    $('#engagement-dialog').style.display = 'none';
  });
  $('#engagement-form').addEventListener('submit', async (e) => {
    e.preventDefault();
    const name = $('#eng-name').value.trim();
    const seed = $('#eng-seed').checked;
    if (!name) return;
    try {
      const result = await API.call('create_engagement', { name });
      await API.call('open_db', { path: result.slug });
      dbOpen = true;
      $('#engagement-dialog').style.display = 'none';
      $('#eng-name').value = '';
      if (seed) {
        await API.call('seed_library', { update: false });
        toast('Prompt library seeded', 'success');
      }
      toast(`Engagement created: ${name}`, 'success');
      onDbOpen(result.name || name, result.slug);
    } catch (err) { toast(err.message, 'error'); }
  });

  // ── Wizard bindings ────────────────────────────────────────────────
  $('#wizard-close').addEventListener('click', closeEngagementWizard);
  $('#wizard-cancel').addEventListener('click', closeEngagementWizard);
  $('#wizard-prev').addEventListener('click', () => wizardSetStep(wizardState.step - 1));
  $('#wizard-next').addEventListener('click', () => {
    if (!wizardCanContinueFromStep()) return;
    wizardSetStep(wizardState.step + 1);
  });
  $('#wizard-fire').addEventListener('click', fireWizardEngagement);
  $('#wizard-test-connection').addEventListener('click', validateWizardTargetStep);

  $('#wizard-target-mode-existing').addEventListener('click', () => {
    wizardState.targetMode = 'existing';
    clearWizardTestStatus();
    persistWizardState();
    renderWizard();
  });
  $('#wizard-target-mode-new').addEventListener('click', () => {
    wizardState.targetMode = 'new';
    clearWizardTestStatus();
    persistWizardState();
    renderWizard();
  });
  $('#wizard-existing-target').addEventListener('change', (e) => {
    wizardState.existingTargetId = e.target.value;
    clearWizardTestStatus();
    persistWizardState();
  });

  const wizardInputBindings = [
    ['wizard-new-target-name', 'name'],
    ['wizard-new-base-url', 'baseUrl'],
    ['wizard-new-protocol', 'protocol'],
    ['wizard-new-auth', 'auth'],
    ['wizard-new-auth-env', 'authEnv'],
    ['wizard-new-auth-header', 'authHeader'],
    ['wizard-new-session', 'sessionHandling'],
    ['wizard-new-session-field', 'sessionField'],
  ];
  wizardInputBindings.forEach(([id, key]) => {
    const el = $(`#${id}`);
    el.addEventListener('input', () => {
      wizardState.newTarget[key] = el.value;
      clearWizardTestStatus();
      persistWizardState();
      if (id === 'wizard-new-auth' || id === 'wizard-new-session') renderWizardStep1();
    });
    el.addEventListener('change', () => {
      wizardState.newTarget[key] = el.value;
      clearWizardTestStatus();
      persistWizardState();
      if (id === 'wizard-new-auth' || id === 'wizard-new-session') renderWizardStep1();
    });
  });

  $('#wizard-engagement-name').addEventListener('input', (e) => {
    wizardState.engagementName = e.target.value;
    persistWizardState();
  });
  $('#wizard-save-template').addEventListener('change', (e) => {
    wizardState.saveAsTemplate = !!e.target.checked;
    persistWizardState();
  });

  // ── Workbench: target selector ─────────────────────────────────────
  async function loadWorkbenchTargets() {
    try {
      const targets = await API.call('list_targets', {});
      wb.availableTargets = targets;
      renderWorkbenchTargetDialogList();

      if (targets.length === 0) {
        clearWorkbenchTarget(true);
        return;
      }

      if (wb.activeTargetId) {
        const active = targets.find(t => t.id === wb.activeTargetId);
        if (active) {
          setWorkbenchTarget(active);
          return;
        }
      }

      clearWorkbenchTarget(true);
    } catch (err) { toast(`Failed to load targets: ${err.message}`, 'error'); }
  }

  function renderWorkbenchTargetDialogList() {
    const list = $('#wb-target-dialog-list');
    if (!list) return;

    list.innerHTML = '';
    if (wb.availableTargets.length === 0) {
      list.innerHTML = '<div class="eng-list-empty">No targets yet. Create one in Targets.</div>';
      return;
    }

    wb.availableTargets.forEach((t) => {
      const row = document.createElement('div');
      row.className = 'engagement-card';
      row.innerHTML = `
        <span class="eng-name">${esc(t.name)}</span>
        <span class="eng-meta">${esc((t.url || '').replace(/^https?:\/\//, ''))}</span>`;
      row.addEventListener('click', () => {
        setWorkbenchTarget(t);
        closeWorkbenchTargetDialog();
      });
      list.appendChild(row);
    });
  }

  function openWorkbenchTargetDialog() {
    renderWorkbenchTargetDialogList();
    $('#wb-target-dialog').style.display = 'flex';
  }

  function closeWorkbenchTargetDialog() {
    $('#wb-target-dialog').style.display = 'none';
  }

  function clearWorkbenchTarget(resetCards = true) {
    wb.activeTargetId = null;
    wb.activeTarget = null;
    $('#wb-target-name').textContent = 'Pick Target';
    $('#wb-target-url').textContent = '';
    $('#wb-target-selector').title = 'Pick target';
    $('#wb-target-selector').classList.remove('has-target');
    $('#wb-status-dot').classList.remove('online');
    $('#wb-meta-endpoint').innerHTML = '<strong>—</strong>';
    $('#wb-meta-auth').textContent = '—';
    $('#wb-meta-session').textContent = '—';
    $('#btn-wb-promote').style.display = 'none';
    setWorkbenchInteractivity(false);

    if (resetCards) {
      wb.activeCardEl = null;
      wb.baselineCardEl = null;
      wb.baselineResultId = null;
      updateBaselineIndicators();
      resetResponseStream();
      resetDetailPane();
    } else {
      renderWorkbenchEmptyState();
    }
  }

  function setWorkbenchTarget(t) {
    const targetChanged = wb.activeTargetId && wb.activeTargetId !== t.id;
    wb.activeTargetId = t.id;
    wb.activeTarget = t;
    $('#wb-target-name').textContent = t.name;
    $('#wb-target-url').textContent = '→ ' + (t.url || '');
    $('#wb-target-selector').title = `Switch target (current: ${t.name})`;
    $('#wb-target-selector').classList.add('has-target');
    $('#wb-status-dot').classList.add('online');
    $('#wb-meta-endpoint').innerHTML = `<strong>${esc(t.endpoint_type || '—')}</strong>`;
    $('#wb-meta-auth').textContent = t.auth_type || 'none';
    $('#wb-meta-session').textContent = t.session_strategy || 'none';
    $('#btn-wb-promote').style.display = '';
    setWorkbenchInteractivity(true);

    if (targetChanged) {
      wb.activeCardEl = null;
      wb.baselineCardEl = null;
      wb.baselineResultId = null;
      updateBaselineIndicators();
      resetResponseStream();
      resetDetailPane();
    } else if (!$$('#wb-response-stream .response-card').length) {
      renderWorkbenchEmptyState();
    }
  }

  $('#wb-target-selector').addEventListener('click', openWorkbenchTargetDialog);
  $('#wb-target-dialog-close').addEventListener('click', closeWorkbenchTargetDialog);
  $('#wb-target-dialog-cancel').addEventListener('click', closeWorkbenchTargetDialog);
  $('#wb-target-manage').addEventListener('click', () => {
    closeWorkbenchTargetDialog();
    showView('view-targets');
  });
  $('#wb-empty-pick-target')?.addEventListener('click', openWorkbenchTargetDialog);
  $('#wb-empty-start-engagement')?.addEventListener('click', (e) => {
    e.preventDefault();
    openEngagementWizard();
  });
  $('#btn-wb-promote').addEventListener('click', () => {
    if (!wb.activeTargetId) return;
    resetWizardState();
    wizardState.targetMode = 'existing';
    wizardState.existingTargetId = wb.activeTargetId;
    wizardState.tested = { ok: true, message: 'Target pre-selected from Workbench.' };
    persistWizardState();
    openEngagementWizard();
  });

  // ── Workbench: prompt picker ───────────────────────────────────────
  let pickerFilter = { owasp: '', search: '', tab: 'library' };
  let pickerDebounceTimer = null;

  async function loadPickerPrompts() {
    try {
      // Always fetch the full unfiltered list to keep coverage grid accurate
      const all = await API.call('list_prompts', {});
      wb.allPrompts = all;
      updateCoverageGrid(all);

      // Apply OWASP + search filters for the visible list
      const prompts = applyPromptFilter(all, pickerFilter.owasp, pickerFilter.search);
      renderPickerPrompts(prompts);
      $('#picker-prompt-count').textContent = `${prompts.length} prompts`;
    } catch (err) { toast(`Failed to load prompts: ${err.message}`, 'error'); }
  }

  // ── W-02 · OWASP coverage grid (client-side from loaded prompts) ──
  function updateCoverageGrid(prompts) {
    const refs = ['A01','A02','A03','A04','A05','A06','A07','A08','A09','A10'];
    refs.forEach(ref => {
      const count = prompts.filter(p => p.owasp_ref === ref).length;
      const level = count >= 10 ? 'high' : count >= 5 ? 'med' : count >= 1 ? 'low' : 'none';
      const cell = $(`.coverage-cell[data-owasp="${ref}"]`);
      if (cell) cell.dataset.cov = level;
    });
  }

  function renderPickerPrompts(prompts) {
    const list = $('#picker-prompt-list');
    list.innerHTML = '';
    if (prompts.length === 0) {
      list.innerHTML = '<div style="padding:20px 14px;font-family:var(--mono);font-size:11px;color:var(--text-3);text-align:center;">no prompts match filter</div>';
      return;
    }
    prompts.forEach(p => {
      const sevKey = (p.severity || 'low').toLowerCase();
      const tags = (p.tags || []).map(t => `<span class="tag">${esc(t)}</span>`).join('');
      const row = document.createElement('div');
      row.className = 'prompt-row';
      row.dataset.id = p.id;
      row.innerHTML = `
        <div class="meta">
          <span class="id">${esc(p.id)}</span>
          <span class="owasp prompt-name">${esc(p.category || '')}</span>
          <span class="sev sev-${esc(sevKey)}">${esc(p.severity || '')}</span>
        </div>
        <div class="text">${esc(p.text)}</div>
        ${tags ? `<div class="tags">${tags}</div>` : ''}`;
      row.addEventListener('click', () => selectPickerPrompt(p, row));
      list.appendChild(row);
    });
  }

  function selectPickerPrompt(p, rowEl) {
    $$('#picker-prompt-list .prompt-row').forEach(r => r.classList.remove('selected'));
    rowEl.classList.add('selected');
    wb.selectedPromptId = p.id;
    wb.selectedPrompt = p;
    $('#wb-prompt-textarea').value = p.text;
    $('#wb-active-id').textContent = p.id;
    const sevKey = (p.severity || 'low').toLowerCase();
    const sevEl = $('#wb-active-sev');
    sevEl.textContent = p.severity || '';
    sevEl.className = `active-sev sev-${sevKey}`;
    updateCharCount();
  }

  // OWASP chip filter
  $$('#picker-chips .chip').forEach(chip => {
    chip.addEventListener('click', () => {
      $$('#picker-chips .chip').forEach(c => c.classList.remove('active'));
      chip.classList.add('active');
      pickerFilter.owasp = chip.dataset.owasp;
      loadPickerPrompts();
    });
  });

  // Picker tab switching (T-20: mutations tab wired)
  $$('.picker-tab').forEach(tab => {
    tab.addEventListener('click', () => {
      $$('.picker-tab').forEach(t => t.classList.remove('active'));
      tab.classList.add('active');
      pickerFilter.tab = tab.dataset.tab;
      if (pickerFilter.tab === 'library') loadPickerPrompts();
      else if (pickerFilter.tab === 'mutations') loadMutationsPicker();
    });
  });

  // ── T-20 · Mutations picker tab ───────────────────────────────────
  async function loadMutationsPicker() {
    const text = $('#wb-prompt-textarea').value.trim();
    const list = $('#picker-prompt-list');
    if (!text) {
      list.innerHTML = '<div style="padding:20px 14px;font-family:var(--mono);font-size:11px;color:var(--text-3);text-align:center;">select or type a prompt first</div>';
      return;
    }
    list.innerHTML = '<div style="padding:20px 14px;font-family:var(--mono);font-size:11px;color:var(--text-3);text-align:center;">generating…</div>';
    try {
      const mutations = await API.call('get_mutations', { prompt_text: text });
      list.innerHTML = '';
      mutations.forEach(m => {
        const row = document.createElement('div');
        row.className = 'prompt-row';
        row.innerHTML = `
          <div class="meta"><span class="id">${esc(m.label)}</span></div>
          <div class="text">${esc(m.text)}</div>`;
        row.addEventListener('click', () => {
          $$('#picker-prompt-list .prompt-row').forEach(r => r.classList.remove('selected'));
          row.classList.add('selected');
          $('#wb-prompt-textarea').value = m.text;
          $('#wb-active-id').textContent = m.label;
          updateCharCount();
        });
        list.appendChild(row);
      });
      $('#picker-prompt-count').textContent = `${mutations.length} mutations`;
    } catch (err) {
      list.innerHTML = `<div style="padding:20px 14px;font-family:var(--mono);font-size:11px;color:var(--text-3);text-align:center;">${esc(err.message)}</div>`;
    }
  }

  // Search
  $('#picker-search').addEventListener('input', (e) => {
    clearTimeout(pickerDebounceTimer);
    pickerDebounceTimer = setTimeout(() => {
      pickerFilter.search = e.target.value;
      loadPickerPrompts();
    }, 200);
  });

  // Char counter
  function updateCharCount() {
    const text = $('#wb-prompt-textarea').value;
    $('#wb-char-count').textContent = `${text.length} chars`;
  }
  $('#wb-prompt-textarea').addEventListener('input', updateCharCount);
  resetDetailPane();

  // ── T-16 · Fire prompt ────────────────────────────────────────────
  async function firePrompt() {
    if (!dbOpen) { toast('Open an engagement first', 'error'); return; }
    if (!wb.activeTargetId) { toast('Select a target first', 'error'); return; }
    const text = $('#wb-prompt-textarea').value.trim();
    if (!text) { toast('Enter a prompt to fire', 'error'); return; }

    const fireBtn = $('#btn-wb-fire');
    fireBtn.disabled = true;

    // Remove empty placeholder
    const emptyEl = $('#wb-response-empty');
    if (emptyEl) emptyEl.remove();

    // Prepend a pending card immediately
    const cardEl = createResponseCard({
      promptText: text,
      promptId: wb.selectedPromptId,
      pending: true,
    });
    $('#wb-response-stream').prepend(cardEl);

    try {
      const result = await API.call('fire_prompt', {
        target_id: wb.activeTargetId,
        prompt_text: text,
        prompt_id: wb.selectedPromptId || undefined,
      });
      updateResponseCard(cardEl, result);
      if (!getBaselineCard() && (result.response_text || '').trim()) {
        setBaselineCard(cardEl, false);
      }
      selectResponseCard(cardEl);
    } catch (err) {
      updateResponseCard(cardEl, { error: err.message });
      selectResponseCard(cardEl);
      toast(err.message, 'error');
    } finally {
      fireBtn.disabled = false;
    }
  }

  function createResponseCard({ promptText, promptId, pending }) {
    const now = new Date();
    const ts = `${String(now.getHours()).padStart(2,'0')}:${String(now.getMinutes()).padStart(2,'0')}:${String(now.getSeconds()).padStart(2,'0')}`;
    const card = document.createElement('div');
    card.className = 'response-card' + (pending ? ' pending' : '');
    card.innerHTML = `
      <div class="response-meta">
        <span class="response-ts">${esc(ts)}</span>
        ${promptId ? `<span class="response-pid">${esc(promptId)}</span>` : ''}
        <span class="response-baseline hidden" data-baseline-marker>baseline</span>
        <span class="verdict-badge verdict-pending" data-verdict>pending</span>
        <span class="response-status" data-status></span>
        <span class="response-latency" data-latency></span>
      </div>
      <div class="response-prompt-preview">${esc(promptText.substring(0, 120))}</div>
      <div class="response-body" data-body>
        <span class="response-loading">…</span>
      </div>
      <div class="signal-pills" data-signals></div>
      <div class="response-actions" data-actions style="display:none">
        <button class="mini-btn btn-set-baseline">Set Baseline</button>
        <button class="mini-btn btn-judge">Judge</button>
        <button class="mini-btn btn-promote">Promote to Finding</button>
        <button class="mini-btn btn-rerun">Re-run</button>
        <button class="mini-btn btn-copy-repro">Copy Repro</button>
        <button class="mini-btn btn-diff-baseline">Diff vs Baseline</button>
      </div>`;
    card.addEventListener('click', (e) => {
      if (e.target.closest('button')) return;
      selectResponseCard(card);
    });
    return card;
  }

  function updateResponseCard(cardEl, result) {
    cardEl.classList.remove('pending');
    cardEl.dataset.resultId = result.result_id || '';
    cardEl.dataset.runId = result.run_id || '';
    cardEl.dataset.promptText = cardEl.querySelector('.response-prompt-preview').textContent;
    cardEl.dataset.responseText = result.response_text || '';
    cardEl.dataset.signals = JSON.stringify(result.signals || []);
    cardEl.dataset.judgeVerdict = result.judge_verdict || '';
    cardEl.dataset.judgeConfidence = result.judge_confidence != null ? String(result.judge_confidence) : '';
    cardEl.dataset.judgeReason = result.judge_reason || '';
    cardEl.dataset.judgeModelUsed = result.judge_model_used || '';
    cardEl.dataset.judgeEvaluatedAt = result.judge_evaluated_at || '';

    const statusEl = cardEl.querySelector('[data-status]');
    const latencyEl = cardEl.querySelector('[data-latency]');
    const bodyEl = cardEl.querySelector('[data-body]');
    const signalsEl = cardEl.querySelector('[data-signals]');
    const actionsEl = cardEl.querySelector('[data-actions]');
    const verdictEl = cardEl.querySelector('[data-verdict]');

    if (result.error && !result.response_text) {
      setVerdictBadge(verdictEl, 'FAIL');
      bodyEl.innerHTML = `<span class="response-error">${esc(result.error)}</span>`;
    } else if (result.judge_verdict) {
      setVerdictBadge(verdictEl, result.judge_verdict);
      bodyEl.textContent = result.response_text || '(empty response)';
    } else {
      setVerdictBadge(verdictEl, '');
      bodyEl.textContent = result.response_text || '(empty response)';
    }

    if (result.status_code) statusEl.textContent = `HTTP ${result.status_code}`;
    if (result.latency_ms != null) latencyEl.textContent = `${result.latency_ms}ms`;

    // Signal pills
    signalsEl.innerHTML = '';
    (result.signals || []).forEach(sig => {
      const pill = document.createElement('span');
      const typeClass = { pii: 'signal-pii', sys_prompt: 'signal-sys', injection_echo: 'signal-echo', internal_hostname: 'signal-internal' }[sig.type] || '';
      pill.className = `signal ${typeClass}`;
      pill.textContent = sig.label;
      signalsEl.appendChild(pill);
    });

    actionsEl.style.display = '';
    wireCardActions(cardEl, result);
    updateBaselineIndicators();
  }

  function selectResponseCard(cardEl) {
    $$('.response-card').forEach(c => c.classList.remove('active'));
    cardEl.classList.add('active');
    wb.activeCardEl = cardEl;

    const signals = JSON.parse(cardEl.dataset.signals || '[]');
    const responseText = cardEl.dataset.responseText || '';

    // Signals tab
    const signalsList = $('#signals-list');
    signalsList.innerHTML = '';
    if (signals.length === 0) {
      signalsList.innerHTML = '<div style="color:var(--text-3);font-family:var(--mono);font-size:11px;text-align:center;padding:40px 20px;">no signals detected</div>';
    } else {
      signals.forEach(sig => {
        const row = document.createElement('div');
        row.className = 'signal-row';
        const evidence = sig.evidence?.length ? sig.evidence.join(', ') : '';
        const sevClass = (
          sig.type === 'pii' ? 'sev-high' :
          sig.type === 'sys_prompt' ? 'sev-medium' :
          sig.type === 'injection_echo' ? 'sev-critical' :
          'sev-low'
        );
        row.innerHTML = `<span class="s-name">${esc(sig.label)}</span><span class="s-severity sev ${sevClass}">x${sig.count}</span>${evidence ? `<span class="s-evidence">${esc(evidence)}</span>` : ''}`;
        signalsList.appendChild(row);
      });
    }
    $('#signals-badge').textContent = signals.length;

    // Raw tab
    $('#detail-raw-pre').textContent = responseText || '—';

    // Judge tab
    renderJudgePanel(cardEl, signals);

    // Diff tab
    const baselineCard = getBaselineCard();
    const baselineText = baselineCard?.dataset.responseText || '';
    if (!baselineCard) {
      renderDiffEmpty('set a baseline response first, then select another result');
    } else if (baselineCard === cardEl) {
      renderDiffEmpty('selected response is the baseline');
    } else if (!baselineText.trim() || !responseText.trim()) {
      renderDiffEmpty('baseline or current response has no text to compare');
    } else {
      renderDiff(baselineText, responseText);
    }

    // Default to diff when possible, otherwise signals/raw.
    if (baselineCard && baselineCard !== cardEl && baselineText.trim() && responseText.trim()) {
      switchDetailTab('diff');
    } else if (signals.length > 0) {
      switchDetailTab('signals');
    } else {
      switchDetailTab('raw');
    }
  }

  function switchDetailTab(panel) {
    $$('.detail-tab').forEach(t => t.classList.remove('active'));
    $$('.detail-panel').forEach(p => p.classList.remove('active'));
    const tabEl = $(`.detail-tab[data-panel="${panel}"]`);
    const panelEl = $(`#detail-${panel}`);
    if (tabEl) tabEl.classList.add('active');
    if (panelEl) panelEl.classList.add('active');
  }

  $('#btn-wb-fire').addEventListener('click', firePrompt);

  $('#btn-wb-baseline').addEventListener('click', () => {
    const selected = wb.activeCardEl || [...$$('.response-card')].find(c => (c.dataset.responseText || '').trim());
    if (!selected) {
      toast('Fire and select a response first', 'info');
      return;
    }
    if (setBaselineCard(selected)) {
      if (wb.activeCardEl) selectResponseCard(wb.activeCardEl);
    }
  });

  $('#btn-wb-judge-all').addEventListener('click', async () => {
    const btn = $('#btn-wb-judge-all');
    const cards = [...$$('.response-card')].filter(c => c.dataset.resultId);
    if (cards.length === 0) {
      toast('No responses available to judge', 'info');
      return;
    }

    const original = btn.textContent;
    btn.disabled = true;
    btn.textContent = 'Judging…';
    try {
      const runIds = [...new Set(cards.map(c => c.dataset.runId).filter(Boolean))];
      const payload = { result_ids: cards.map(c => c.dataset.resultId).filter(Boolean) };
      if (runIds.length === 1) payload.run_id = runIds[0];
      const res = await API.call('judge_all', payload);
      const byResultId = new Map((res.results || []).map(r => [r.result_id, r]));
      cards.forEach(card => {
        const judged = byResultId.get(card.dataset.resultId);
        if (judged) applyJudgeToCard(card, judged);
      });

      if (wb.activeCardEl) {
        const signals = JSON.parse(wb.activeCardEl.dataset.signals || '[]');
        renderJudgePanel(wb.activeCardEl, signals);
        switchDetailTab('judge');
      }

      toast(`Judge complete: ${res.judged} judged, ${res.skipped_existing} skipped`, 'success');
    } catch (err) {
      toast(`Judge all failed: ${err.message}`, 'error');
    } finally {
      btn.disabled = false;
      btn.textContent = original;
    }
  });

  $('#btn-wb-save-as').addEventListener('click', () => {
    const text = $('#wb-prompt-textarea').value.trim();
    if (!text) { toast('Enter a prompt to save', 'error'); return; }
    if (!dbOpen) { toast('Open an engagement first', 'error'); return; }
    const id = `custom-${Date.now()}`;
    API.call('create_prompt', {
      id,
      text,
      category: 'prompt_injection',
      owasp_ref: 'A01',
      severity: 'MEDIUM',
      tags: [],
      mode: 'single',
      source: 'workbench',
    }).then(() => {
      toast(`Saved as ${id}`, 'success');
      wb.allPrompts = [];
      loadPickerPrompts();
    }).catch(err => toast(err.message, 'error'));
  });

  $('#btn-wb-duplicate').addEventListener('click', () => {
    const text = $('#wb-prompt-textarea').value.trim();
    if (!text) { toast('Nothing in editor to duplicate', 'error'); return; }
    $('#wb-prompt-textarea').value = text;
    $('#wb-active-id').textContent = '—';
    wb.selectedPromptId = null;
    wb.selectedPrompt = null;
    updateCharCount();
    toast('Duplicated into editor — edit and fire or save as new', 'info');
  });

  // ── T-17 · Response card actions ─────────────────────────────────
  function wireCardActions(cardEl, result) {
    cardEl.querySelector('.btn-set-baseline')?.addEventListener('click', () => {
      if (setBaselineCard(cardEl)) {
        if (wb.activeCardEl) selectResponseCard(wb.activeCardEl);
      }
    });

    cardEl.querySelector('.btn-judge')?.addEventListener('click', async () => {
      try {
        await judgeCard(cardEl, { force: false, switchToJudge: true });
      } catch (err) {
        toast(`Judge failed: ${err.message}`, 'error');
      }
    });

    cardEl.querySelector('.btn-rerun')?.addEventListener('click', async () => {
      const promptText = cardEl.querySelector('.response-prompt-preview').textContent;
      if (!promptText) return;
      const fireBtn = $('#btn-wb-fire');
      fireBtn.disabled = true;
      const newCard = createResponseCard({ promptText, promptId: null, pending: true });
      $('#wb-response-stream').prepend(newCard);
      try {
        const res = await API.call('fire_prompt', {
          target_id: wb.activeTargetId,
          prompt_text: promptText,
        });
        updateResponseCard(newCard, res);
        if (!getBaselineCard() && (res.response_text || '').trim()) {
          setBaselineCard(newCard, false);
        }
        selectResponseCard(newCard);
      } catch (err) {
        updateResponseCard(newCard, { error: err.message });
        selectResponseCard(newCard);
      } finally {
        fireBtn.disabled = false;
      }
    });

    cardEl.querySelector('.btn-copy-repro')?.addEventListener('click', () => {
      const promptText = cardEl.querySelector('.response-prompt-preview').textContent;
      navigator.clipboard.writeText(promptText).then(() => toast('Prompt copied to clipboard', 'success'));
    });

    cardEl.querySelector('.btn-diff-baseline')?.addEventListener('click', () => {
      const responseText = cardEl.dataset.responseText || '';
      const baselineCard = getBaselineCard();
      if (!baselineCard) { toast('Set a baseline first', 'info'); return; }
      if (baselineCard === cardEl) { toast('Select a non-baseline response for diff', 'info'); return; }
      renderDiff(baselineCard.dataset.responseText || '', responseText);
      switchDetailTab('diff');
    });

    cardEl.querySelector('.btn-promote')?.addEventListener('click', () => {
      if (!result.result_id) { toast('No result ID to promote', 'error'); return; }
      showPromoteModal(result.result_id);
    });
  }

  // Promote to finding modal
  const OWASP_REFS = ['A01','A02','A03','A04','A05','A06','A07','A08','A09','A10'];

  let _promoteResultId = null;
  function showPromoteModal(resultId) {
    _promoteResultId = resultId;
    let modal = $('#promote-modal');
    if (!modal) {
      const refCheckboxes = OWASP_REFS.map(r =>
        `<label class="owasp-ref-check"><input type="checkbox" value="${r}"><span>${r}</span></label>`
      ).join('');
      modal = document.createElement('div');
      modal.id = 'promote-modal';
      modal.className = 'modal';
      modal.innerHTML = `<div class="modal-content modal-small">
        <button class="modal-close" id="promote-modal-close">&times;</button>
        <h3 style="margin-bottom:1rem;font-family:var(--mono);font-size:13px;text-transform:uppercase;letter-spacing:0.08em;">Promote to Finding</h3>
        <div class="form-row">
          <label for="promote-title">Title</label>
          <input id="promote-title" type="text" placeholder="Describe the finding…" required>
        </div>
        <div class="form-row">
          <label for="promote-severity">Severity</label>
          <select id="promote-severity">
            <option value="LOW">LOW</option>
            <option value="MEDIUM">MEDIUM</option>
            <option value="HIGH" selected>HIGH</option>
            <option value="CRITICAL">CRITICAL</option>
          </select>
        </div>
        <div class="form-row">
          <label>OWASP refs</label>
          <div class="owasp-ref-grid" id="promote-owasp-refs">${refCheckboxes}</div>
        </div>
        <div class="editor-actions">
          <button class="btn btn-primary" id="promote-confirm">Promote</button>
          <button class="btn btn-ghost" id="promote-cancel">Cancel</button>
        </div>
      </div>`;
      document.body.appendChild(modal);
      modal.addEventListener('click', e => { if (e.target === modal) modal.style.display = 'none'; });
      modal.querySelector('#promote-modal-close').addEventListener('click', () => modal.style.display = 'none');
      modal.querySelector('#promote-cancel').addEventListener('click', () => modal.style.display = 'none');
      modal.querySelector('#promote-confirm').addEventListener('click', async () => {
        const title = modal.querySelector('#promote-title').value.trim();
        const severity = modal.querySelector('#promote-severity').value;
        const owasp_refs = [...modal.querySelectorAll('#promote-owasp-refs input:checked')].map(cb => cb.value);
        if (!title) { toast('Enter a title', 'error'); return; }
        try {
          await API.call('promote_finding', { result_id: _promoteResultId, title, severity, owasp_refs });
          toast('Finding promoted', 'success');
          modal.style.display = 'none';
          $('#findings-drawer').classList.remove('collapsed');
          loadFindings();
        } catch (err) { toast(err.message, 'error'); }
      });
    }
    modal.querySelector('#promote-title').value = '';
    modal.querySelectorAll('#promote-owasp-refs input').forEach(cb => cb.checked = false);
    // pre-check the OWASP ref from the selected prompt if available
    if (wb.selectedPrompt?.owasp_ref) {
      const cb = modal.querySelector(`#promote-owasp-refs input[value="${wb.selectedPrompt.owasp_ref}"]`);
      if (cb) cb.checked = true;
    }
    modal.style.display = 'flex';
  }

  // ── T-18 · Detail pane: diff helper ──────────────────────────────
  function renderDiff(baseText, newText) {
    const diffEl = $('#detail-diff');
    const baseWords = String(baseText || '').trim().split(/\s+/).filter(Boolean);
    const newWords = String(newText || '').trim().split(/\s+/).filter(Boolean);
    const newSet = new Set(newWords);
    const baseSet = new Set(baseWords);

    const leftHtml = baseWords.map(w =>
      newSet.has(w) ? esc(w) : `<span class="diff-del">${esc(w)}</span>`
    ).join(' ');
    const rightHtml = newWords.map(w =>
      baseSet.has(w) ? esc(w) : `<span class="diff-add">${esc(w)}</span>`
    ).join(' ');

    diffEl.innerHTML = `<div class="diff-row">
      <div class="diff-col baseline">
        <div class="label">baseline</div>
        <div class="body">${leftHtml || '—'}</div>
      </div>
      <div class="diff-col attack">
        <div class="label">current</div>
        <div class="body">${rightHtml || '—'}</div>
      </div>
    </div>`;
  }

  // ── Workbench: detail tabs ─────────────────────────────────────────
  $$('.detail-tab').forEach(tab => {
    tab.addEventListener('click', () => {
      switchDetailTab(tab.dataset.panel);
    });
  });

  // ── T-19 · Findings drawer ────────────────────────────────────────
  $('#findings-header').addEventListener('click', () => {
    $('#findings-drawer').classList.toggle('collapsed');
  });

  async function loadFindings() {
    if (!dbOpen) return;
    try {
      const findings = await API.call('list_findings', {});
      wb.findings = findings;
      renderFindings(findings);
    } catch (err) { toast(`Failed to load findings: ${err.message}`, 'error'); }
  }

  function renderFindings(findings) {
    const body = $('#findings-body');
    const countEl = $('#findings-count');
    const critEl = $('#findings-stat-crit');
    const highEl = $('#findings-stat-high');
    const medEl = $('#findings-stat-med');

    const counts = { CRITICAL: 0, HIGH: 0, MEDIUM: 0, LOW: 0 };
    findings.forEach(f => { if (f.severity in counts) counts[f.severity]++; });

    countEl.textContent = findings.length;
    critEl.textContent = counts.CRITICAL;
    highEl.textContent = counts.HIGH;
    medEl.textContent = counts.MEDIUM;

    body.innerHTML = '';
    if (findings.length === 0) {
      body.innerHTML = '<div style="padding:16px 20px;font-family:var(--mono);font-size:11px;color:var(--text-3);">no findings yet — promote a response to add one</div>';
      return;
    }
    findings.forEach(f => {
      const row = document.createElement('div');
      row.className = 'finding-row';
      const sevKey = (f.severity || 'low').toLowerCase();
      const refs = (f.owasp_refs || []).map(r => `<span class="chip chip-sm">${esc(r)}</span>`).join('');
      const date = f.promoted_at ? f.promoted_at.substring(0, 10) : '';
      row.innerHTML = `
        <span class="sev sev-${esc(sevKey)}">${esc(f.severity)}</span>
        <span class="finding-title">${esc(f.title)}</span>
        <span class="finding-refs">${refs}</span>
        <span class="finding-date">${esc(date)}</span>`;
      body.appendChild(row);
    });
  }

  $('#btn-export-pdf').addEventListener('click', async () => {
    if (!dbOpen) { toast('Open an engagement first', 'error'); return; }
    try {
      const result = await API.call('export_findings_pdf', {});
      toast(`Report saved: ${result.path}`, 'success');
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
    try {
      const targets = await API.call('list_targets', {});
      const ul = $('#target-list');
      ul.innerHTML = '';
      targets.forEach(t => {
        const li = document.createElement('li');
        li.className = 'target-card-row';
        li.dataset.id = t.id;
        li.innerHTML = `
          <div class="target-card-name">${esc(t.name)}</div>
          <div class="target-card-url">${esc((t.url || '').replace(/^https?:\/\//, ''))}</div>`;
        li.addEventListener('click', () => openTargetEditor(t.id));
        ul.appendChild(li);
      });

      const welcomeEl = $('#target-welcome');
      const contentEl = $('#target-content');
      if (targets.length === 0) {
        welcomeEl.style.display = '';
        contentEl.style.display = 'none';
        $('#target-form').style.display = 'none';
      } else {
        welcomeEl.style.display = 'none';
        contentEl.style.display = '';
      }
    } catch (err) { toast(err.message, 'error'); }
  }

  function startNewTarget() {
    $('#target-id').value = '';
    $('#target-form').reset();
    $('#target-form').style.display = '';
    $('#btn-delete-target').style.display = 'none';
    $('#target-welcome').style.display = 'none';
    $('#target-content').style.display = '';
  }

  $('#btn-new-target').addEventListener('click', startNewTarget);
  $('#btn-get-started').addEventListener('click', startNewTarget);

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
      $('#target-auth-value').value = t.auth_env || '';
      $('#target-auth-header').value = t.auth_header || '';
      $('#map-request').value = t.request_field || 'message';
      $('#map-response').value = t.response_field || 'response';
      $('#target-session-strategy').value = t.session_strategy || 'none';
      $('#target-session-strategy').dispatchEvent(new Event('change'));
      $('#target-session-field').value = t.session_field || '';
      $('#target-system-prompt').value = t.notes || '';
      $('#target-form').style.display = '';
      $('#btn-delete-target').style.display = '';
      $('#target-welcome').style.display = 'none';
      $('#target-content').style.display = '';

      // Highlight in sidebar
      $$('#target-list .target-card-row').forEach(li => li.classList.toggle('active', li.dataset.id === targetId));
    } catch (err) { toast(err.message, 'error'); }
  }

  $('#btn-delete-target').addEventListener('click', async () => {
    const id = $('#target-id').value;
    if (!id) return;
    if (!confirm('Delete this target? This cannot be undone.')) return;
    try {
      await API.call('delete_target', { id });
      toast('Target deleted', 'success');
      $('#target-form').style.display = 'none';
      $('#btn-delete-target').style.display = 'none';
      $('#target-id').value = '';
      loadTargetList();
      if (wb.activeTargetId === id) {
        wb.activeTargetId = null;
        loadWorkbenchTargets();
      }
    } catch (err) { toast(err.message, 'error'); }
  });

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
    const existingId = $('#target-id').value.trim();
    data.id = existingId || data.name.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') + '-' + Date.now().toString(36);
    if (data.auth_type !== 'none') {
      data.auth_header = $('#target-auth-header').value.trim() || null;
      data.auth_env = $('#target-auth-value').value.trim() || null;
    }
    if (data.endpoint_type === 'custom_rest') {
      data.request_field = $('#map-request').value.trim() || 'message';
      data.response_field = $('#map-response').value.trim() || 'response';
    }
    const sp = $('#target-system-prompt').value.trim();
    if (sp) data.notes = sp;

    try {
      const saved = await API.call('save_target', data);
      $('#target-id').value = saved.id;
      toast('Target saved', 'success');
      loadTargetList();
    } catch (err) { toast(err.message, 'error'); }
  });

  // ── Shared prompt filter (used by library + picker) ──────────────
  function applyPromptFilter(prompts, owaspFilter, searchText) {
    let result = prompts;
    if (owaspFilter === 'baseline') {
      result = result.filter(p => (p.tags || []).includes('baseline'));
    } else if (owaspFilter) {
      result = result.filter(p => p.owasp_ref === owaspFilter);
    }
    const q = (searchText || '').toLowerCase();
    if (q) {
      result = result.filter(p =>
        p.text.toLowerCase().includes(q) ||
        p.id.toLowerCase().includes(q) ||
        (p.category || '').toLowerCase().includes(q));
    }
    return result;
  }

  // ── Library: load and render ───────────────────────────────────────
  let libraryFilter = { owasp: '', search: '' };
  let libraryDebounce = null;

  async function loadPrompts() {
    try {
      const all = await API.call('list_prompts', {});
      const prompts = applyPromptFilter(all, libraryFilter.owasp, libraryFilter.search);
      renderPrompts(prompts);
      $('#prompt-count').textContent = `${prompts.length} prompts`;
    } catch (err) { toast(err.message, 'error'); }
  }

  function renderPrompts(prompts) {
    const list = $('#library-prompt-list');
    list.innerHTML = '';
    if (prompts.length === 0) {
      list.innerHTML = '<div style="padding:20px 14px;font-family:var(--mono);font-size:11px;color:var(--text-3);text-align:center;">no prompts match filter</div>';
      return;
    }
    prompts.forEach(p => {
      const sevKey = (p.severity || 'low').toLowerCase();
      const tags = (p.tags || []).map(t => `<span class="tag">${esc(t)}</span>`).join('');
      const row = document.createElement('div');
      row.className = 'prompt-row';
      row.dataset.id = p.id;
      row.innerHTML = `
        <div class="meta">
          <span class="id">${esc(p.id)}</span>
          <span class="owasp">${esc(p.owasp_ref)}</span>
          <span class="owasp prompt-name">${esc(p.category || '')}</span>
          <span class="sev sev-${esc(sevKey)}">${esc(p.severity || '')}</span>
        </div>
        <div class="text">${esc(p.text)}</div>
        ${tags ? `<div class="tags">${tags}</div>` : ''}
        <div class="prompt-row-actions">
          <button class="mini-btn btn-edit" data-id="${esc(p.id)}">Edit</button>
          <button class="mini-btn btn-del" data-id="${esc(p.id)}">Delete</button>
        </div>`;
      row.querySelector('.btn-edit').addEventListener('click', e => {
        e.stopPropagation();
        openPromptEditor(p.id);
      });
      row.querySelector('.btn-del').addEventListener('click', e => {
        e.stopPropagation();
        deletePrompt(p.id);
      });
      list.appendChild(row);
    });
  }

  // Library chip filters
  $$('#library-chips .chip').forEach(chip => {
    chip.addEventListener('click', () => {
      $$('#library-chips .chip').forEach(c => c.classList.remove('active'));
      chip.classList.add('active');
      libraryFilter.owasp = chip.dataset.owasp;
      loadPrompts();
    });
  });

  // Library search
  $('#library-search').addEventListener('input', e => {
    clearTimeout(libraryDebounce);
    libraryDebounce = setTimeout(() => {
      libraryFilter.search = e.target.value;
      loadPrompts();
    }, 200);
  });

  // ── Prompts: add/edit ──────────────────────────────────────────────
  $('#btn-add-prompt').addEventListener('click', () => openPromptEditor(null));

  async function openPromptEditor(promptId) {
    editingPromptId = promptId;
    const editorEl = $('#prompt-editor');
    editorEl.style.display = '';
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
      if (wb.allPrompts.length) loadPickerPrompts(); // keep workbench picker in sync
    } catch (err) { toast(err.message, 'error'); }
  });

  async function deletePrompt(id) {
    if (!confirm(`Delete prompt ${id}?`)) return;
    try {
      await API.call('delete_prompt', { id });
      toast('Prompt deleted', 'success');
      loadPrompts();
      if (wb.allPrompts.length) loadPickerPrompts(); // keep workbench picker in sync
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
    } catch (err) { toast(`Failed to load targets: ${err.message}`, 'error'); }
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
    const preselected = index >= 0 && currentScenarioSteps[index]?.prompt_id
      ? [currentScenarioSteps[index].prompt_id]
      : [];
    loadLibraryChecklist(preselected);

    if (index >= 0) {
      const step = currentScenarioSteps[index];
      sel.value = step.session;
      if (step.prompt_id) {
        $('#step-source-type').value = 'library';
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

  async function loadLibraryChecklist(selectedIds = []) {
    try {
      const prompts = await API.call('list_prompts', {});
      const list = $('#step-library-list');
      list.innerHTML = '';
      const selected = new Set(selectedIds || []);
      prompts.forEach(p => {
        const row = document.createElement('label');
        row.className = 'library-check-row';
        row.innerHTML = `
          <input type="checkbox" class="step-library-checkbox"
                 value="${esc(p.id)}"
                 data-text="${esc(p.text)}"
                 ${selected.has(p.id) ? 'checked' : ''}>
          <span><code>${esc(p.id)}</code> — ${esc(p.text.substring(0, 110))}</span>
        `;
        list.appendChild(row);
      });
    } catch (err) { toast(`Failed to load library: ${err.message}`, 'error'); }
  }

  $('#step-source-type').addEventListener('change', () => {
    const isLibrary = $('#step-source-type').value === 'library';
    $('#step-library-row').style.display = isLibrary ? '' : 'none';
    $('#step-custom-row').style.display = isLibrary ? 'none' : '';
  });

  $('#step-library-select-all').addEventListener('click', () => {
    $$('#step-library-list .step-library-checkbox').forEach(cb => { cb.checked = true; });
  });

  $('#step-library-clear').addEventListener('click', () => {
    $$('#step-library-list .step-library-checkbox').forEach(cb => { cb.checked = false; });
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

    if (isLibrary) {
      const session = $('#step-session').value;
      const delayMs = parseInt($('#step-delay').value) || 0;
      const selected = [...$$('#step-library-list .step-library-checkbox:checked')];
      if (selected.length === 0) {
        toast('Select at least one library prompt', 'error');
        return;
      }

      const selectedSteps = selected.map(cb => ({
        session: session,
        prompt_id: cb.value,
        prompt_text: cb.dataset.text || '',
        delay_ms: delayMs,
      }));

      if (editingStepIndex >= 0) {
        currentScenarioSteps[editingStepIndex] = selectedSteps[0];
      } else {
        currentScenarioSteps.push(...selectedSteps);
      }
    } else {
      const step = {
        session: $('#step-session').value,
        prompt_id: null,
        prompt_text: $('#step-prompt-text').value,
        delay_ms: parseInt($('#step-delay').value) || 0,
      };
      if (editingStepIndex >= 0) {
        currentScenarioSteps[editingStepIndex] = step;
      } else {
        currentScenarioSteps.push(step);
      }
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

  // ── E-01/E-02 · Engagement list in Runs view ─────────────────────
  function setEngagementDetailTab(tab) {
    $$('.eng-detail-tab').forEach((btn) => {
      btn.classList.toggle('active', btn.dataset.engTab === tab);
    });
    $$('.eng-detail-panel').forEach((panel) => {
      panel.classList.toggle('active', panel.id === `eng-panel-${tab}`);
    });
  }

  $$('.eng-detail-tab').forEach((btn) => {
    btn.addEventListener('click', () => setEngagementDetailTab(btn.dataset.engTab));
  });

  function formatEngagementDateTime(iso) {
    if (!iso) return '—';
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return iso;
    const y = String(d.getFullYear());
    const m = String(d.getMonth() + 1).padStart(2, '0');
    const day = String(d.getDate()).padStart(2, '0');
    const hh = String(d.getHours()).padStart(2, '0');
    const mm = String(d.getMinutes()).padStart(2, '0');
    return `${y}-${m}-${day} ${hh}:${mm}`;
  }

  function shortText(text, max = 120) {
    const value = String(text || '').replace(/\s+/g, ' ').trim();
    if (!value) return '—';
    return value.length > max ? `${value.slice(0, max - 1)}…` : value;
  }

  function sequenceMatchesRepeated(actual, expected) {
    if (!actual.length || !expected.length) return false;
    if (actual.length % expected.length !== 0) return false;
    for (let i = 0; i < actual.length; i += 1) {
      if (String(actual[i] || '') !== String(expected[i % expected.length] || '')) return false;
    }
    return true;
  }

  function inferScenarioNameFromResults(results, targetId) {
    const promptIds = results
      .map(r => String(r.prompt_id || '').trim())
      .filter(Boolean);
    if (!promptIds.length) return '—';

    const candidates = engagementDetail.scenarios
      .filter(sc => !targetId || !sc.target_id || sc.target_id === targetId);

    for (const sc of candidates) {
      const scPromptIds = (sc.steps || [])
        .map(s => String(s.prompt_id || '').trim())
        .filter(Boolean);
      if (scPromptIds.length && sequenceMatchesRepeated(promptIds, scPromptIds)) {
        return sc.name;
      }
    }

    if (promptIds.length === 1) return 'Workbench single-shot';
    return 'Custom / Wizard run';
  }

  async function hydrateEngagementDetailCatalogs() {
    if (!engagementDetail.targets.length) {
      try { engagementDetail.targets = await API.call('list_targets', {}); } catch (_err) { engagementDetail.targets = []; }
    }
    if (!engagementDetail.scenarios.length) {
      try { engagementDetail.scenarios = await API.call('list_scenarios', {}); } catch (_err) { engagementDetail.scenarios = []; }
    }
  }

  function resolveTargetFromResults(results) {
    const requestUrl = String(results.find(r => r.request_url)?.request_url || '').trim();
    if (!requestUrl) return { id: null, name: '—', url: '' };

    const match = engagementDetail.targets.find(t => String(t.url || '').trim() === requestUrl);
    if (match) {
      return { id: match.id, name: match.name, url: match.url };
    }

    return {
      id: null,
      name: requestUrl.replace(/^https?:\/\//, ''),
      url: requestUrl,
    };
  }

  function renderEngagementTimeline(results) {
    const list = $('#eng-timeline-list');
    if (!results.length) {
      list.innerHTML = '<div class="eng-timeline-empty">Select a run to inspect the timeline.</div>';
      return;
    }

    const sorted = [...results].sort((a, b) => Number(a.seq || 0) - Number(b.seq || 0));
    list.innerHTML = '';
    sorted.forEach((r) => {
      const statusClass = r.error_message ? 'status-error' : 'status-ok';
      const statusText = r.error_message ? 'error' : `${r.status_code || '?'}`;
      const row = document.createElement('div');
      row.className = 'eng-timeline-row';
      row.innerHTML = `
        <div class="eng-timeline-ts">${esc(formatEngagementDateTime(r.received_at || r.sent_at || ''))}</div>
        <div class="eng-timeline-session">${esc(r.session_label || '-')}</div>
        <div class="eng-timeline-body">
          <div class="eng-timeline-top">
            <span class="run-status-badge ${statusClass === 'status-ok' ? 'completed' : 'error'}">${esc(statusText)}</span>
            <span class="eng-timeline-prompt">${esc(shortText(r.prompt_text, 120))}</span>
          </div>
          <div class="eng-timeline-response">${esc(shortText(r.response_text || r.error_message || ''))}</div>
        </div>
        <div class="eng-timeline-latency">${r.latency_ms != null ? `${r.latency_ms}ms` : '—'}</div>
      `;
      row.addEventListener('click', () => showResultDetail(r));
      list.appendChild(row);
    });
  }

  function renderEngagementReport(results, run) {
    const summary = $('#eng-report-summary');
    const coverage = $('#eng-report-coverage');
    const preview = $('#eng-report-preview');

    if (!results.length) {
      summary.textContent = 'Select a run to build the report snapshot.';
      coverage.innerHTML = '';
      preview.textContent = 'No report data yet.';
      return;
    }

    const total = results.length;
    const failed = results.filter(r => !!r.error_message || Number(r.status_code || 0) === 0).length;
    const successful = total - failed;
    const judged = results.filter(r => String(r.judge_verdict || '').trim()).length;
    const avgLatency = results
      .map(r => Number(r.latency_ms))
      .filter(v => Number.isFinite(v))
      .reduce((acc, v, _, arr) => acc + (v / arr.length), 0);

    summary.textContent = `Run ${run?.id || '—'} · ${successful}/${total} successful · ${failed} failed · ${judged} judged`;

    const owaspByPrompt = new Map((wb.allPrompts || []).map(p => [p.id, p.owasp_ref || null]));
    const counts = {};
    results.forEach((r) => {
      const ref = owaspByPrompt.get(r.prompt_id);
      if (!ref) return;
      counts[ref] = (counts[ref] || 0) + 1;
    });

    const refs = ['A01', 'A02', 'A03', 'A04', 'A05', 'A06', 'A07', 'A08', 'A09', 'A10'];
    coverage.innerHTML = refs
      .map(ref => `<span class="eng-report-chip">${ref}: ${counts[ref] || 0}</span>`)
      .join('');

    preview.textContent = [
      `engagement: ${engagementDetail.name}`,
      `slug: ${engagementDetail.slug}`,
      `run: ${run?.id || '—'}`,
      `status: ${run?.status || '—'}`,
      `started_at: ${run?.started_at || '—'}`,
      `results_total: ${total}`,
      `results_successful: ${successful}`,
      `results_failed: ${failed}`,
      `avg_latency_ms: ${Number.isFinite(avgLatency) ? avgLatency.toFixed(1) : 'n/a'}`,
    ].join('\n');
  }

  function updateEngagementHeader(run, results) {
    const target = resolveTargetFromResults(results);
    engagementDetail.targetMatch = target;
    engagementDetail.scenarioName = inferScenarioNameFromResults(results, target.id);

    const endCandidates = [...results]
      .map(r => r.received_at || r.sent_at || '')
      .filter(Boolean)
      .sort();
    const endAt = endCandidates[endCandidates.length - 1] || '';

    $('#eng-detail-target').textContent = target.name || '—';
    $('#eng-detail-scenario').textContent = engagementDetail.scenarioName || '—';
    $('#eng-detail-status').textContent = run?.status || '—';
    $('#eng-detail-start').textContent = formatEngagementDateTime(run?.started_at || '');
    $('#eng-detail-end').textContent = formatEngagementDateTime(endAt);
  }

  function highlightActiveEngagementCard(slug) {
    $$('#engagement-cards .target-card-row').forEach((c) => {
      c.classList.toggle('active', c.dataset.slug === slug);
    });
  }

  function renderRunsViewEmptyState(message) {
    $('#runs-empty').textContent = message;
    $('#runs-empty').style.display = '';
    $('#eng-detail').style.display = 'none';
  }

  async function openEngagementDetail(eng, { syncRoute = true, selectRunId = null } = {}) {
    const result = await API.call('open_db', { path: eng.slug });
    dbOpen = true;
    onDbOpen(result.name || eng.name, result.slug);

    engagementDetail.slug = eng.slug;
    engagementDetail.name = result.name || eng.name;
    engagementDetail.activeRunId = null;
    engagementDetail.resultsByRunId.clear();

    $('#eng-detail-title').textContent = engagementDetail.name;
    $('#eng-detail-slug').textContent = `/engagements/${eng.slug}`;
    $('#runs-empty').style.display = 'none';
    $('#eng-detail').style.display = '';
    $('#run-results-section').style.display = 'none';
    setEngagementDetailTab('results');
    highlightActiveEngagementCard(eng.slug);

    await hydrateEngagementDetailCatalogs();
    await loadRuns({ engagementSlug: eng.slug, autoSelectFirst: true, preferredRunId: selectRunId });

    if (syncRoute) setEngagementRoute(eng.slug);
  }

  async function loadEngagementList({ preferredSlug = null, autoOpen = true, syncRoute = true } = {}) {
    const container = $('#engagement-cards');
    container.innerHTML = '<div style="padding:12px 14px;font-family:var(--mono);font-size:11px;color:var(--text-3);">loading…</div>';
    try {
      const routeSlug = getEngagementSlugFromRoute();
      const desiredSlug = preferredSlug || routeSlug || activeEngagementSlug || null;
      const engagements = (await API.call('list_engagements', {}))
        .filter(eng => !isEngagementArchived(eng.slug));

      container.innerHTML = '';
      if (engagements.length === 0) {
        container.innerHTML = '<div style="padding:12px 14px;font-family:var(--mono);font-size:11px;color:var(--text-3);">no engagements yet</div>';
        renderRunsViewEmptyState('Select an engagement to view its runs.');
        return;
      }

      engagements.forEach((eng) => {
        const card = document.createElement('div');
        card.className = 'target-card-row';
        card.dataset.slug = eng.slug;
        if (eng.slug === activeEngagementSlug) card.classList.add('active');
        const date = eng.created_at ? eng.created_at.substring(0, 10) : '';
        const findings = eng.finding_count || 0;
        const runs = eng.run_count || 0;
        card.innerHTML = `
          <div class="target-card-name">${esc(eng.name)}</div>
          <div class="target-card-url" style="display:flex;gap:10px;">
            <span>${esc(date)}</span>
            <span>${runs} run${runs !== 1 ? 's' : ''}</span>
            ${findings ? `<span style="color:var(--warn)">${findings} finding${findings !== 1 ? 's' : ''}</span>` : ''}
          </div>`;
        card.addEventListener('click', () => {
          openEngagementDetail(eng, { syncRoute: true }).catch(err => toast(err.message, 'error'));
        });
        container.appendChild(card);
      });

      if (autoOpen && desiredSlug) {
        const selected = engagements.find(eng => eng.slug === desiredSlug);
        if (selected) {
          await openEngagementDetail(selected, { syncRoute });
          return;
        }
      }

      if (!engagementDetail.slug) {
        renderRunsViewEmptyState('Select an engagement to view its runs.');
      }
    } catch (err) {
      toast(err.message, 'error');
      renderRunsViewEmptyState('Could not load engagements.');
    }
  }

  $('#btn-runs-new-engagement').addEventListener('click', openEngagementWizard);

  // ── Runs view ──────────────────────────────────────────────────────
  function markActiveRunRow(runId) {
    $$('#runs-tbody tr').forEach((tr) => {
      tr.classList.toggle('active', tr.dataset.runId === runId);
    });
  }

  async function loadRuns({ engagementSlug = activeEngagementSlug, autoSelectFirst = false, preferredRunId = null } = {}) {
    if (!engagementSlug) return;
    try {
      const runs = await API.call('list_runs', { engagement_slug: engagementSlug });
      engagementDetail.runs = runs;
      const tbody = $('#runs-tbody');
      tbody.innerHTML = '';

      if (runs.length === 0) {
        tbody.innerHTML = '<tr><td colspan="6" style="font-family:var(--mono);font-size:11px;color:var(--text-3);text-align:center;padding:20px;">no runs yet — fire a prompt from the workbench</td></tr>';
        $('#run-results-section').style.display = 'none';
        renderEngagementTimeline([]);
        renderEngagementReport([], null);
        updateEngagementHeader(null, []);
        return;
      }

      runs.forEach((r) => {
        const tr = document.createElement('tr');
        tr.className = 'clickable';
        tr.dataset.runId = r.id;
        tr.innerHTML = `
          <td style="font-family:var(--mono);font-size:11px;">${esc(r.id.substring(0, 8))}</td>
          <td><span class="run-status-badge ${esc(r.status)}">${esc(r.status)}</span></td>
          <td style="font-family:var(--mono);font-size:11px;">${r.completed}/${r.total_prompts || '?'}</td>
          <td style="font-family:var(--mono);font-size:11px;color:${r.errors > 0 ? 'var(--critical)' : 'var(--text-2)'};">${r.errors}</td>
          <td style="font-family:var(--mono);font-size:11px;">${esc(formatRunStarted(r.started_at))}</td>
          <td>
            <button class="btn-small btn-view-results">Results</button>
          </td>`;
        tr.addEventListener('click', (e) => {
          if (e.target.closest('button')) return;
          loadRunResults(r.id, { engagementSlug, switchToResultsTab: false }).catch(err => toast(err.message, 'error'));
        });
        tr.querySelector('.btn-view-results').addEventListener('click', (e) => {
          e.stopPropagation();
          loadRunResults(r.id, { engagementSlug, switchToResultsTab: true }).catch(err => toast(err.message, 'error'));
        });
        tbody.appendChild(tr);
      });

      const fallbackRunId = preferredRunId || engagementDetail.activeRunId;
      const chosen = runs.find(r => r.id === fallbackRunId) || (autoSelectFirst ? runs[0] : null);
      if (chosen) {
        await loadRunResults(chosen.id, { engagementSlug, switchToResultsTab: false });
      }
    } catch (err) {
      toast(err.message, 'error');
    }
  }

  async function loadRunResults(runId, { engagementSlug = activeEngagementSlug, switchToResultsTab = false } = {}) {
    try {
      const results = await API.call('get_results', { engagement_slug: engagementSlug, run_id: runId });
      engagementDetail.activeRunId = runId;
      engagementDetail.resultsByRunId.set(runId, results);
      markActiveRunRow(runId);

      $('#run-results-section').style.display = '';
      const tbody = $('#results-tbody');
      tbody.innerHTML = '';
      results.forEach((r) => {
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

      const runSummary = engagementDetail.runs.find(r => r.id === runId) || null;
      updateEngagementHeader(runSummary, results);
      renderEngagementTimeline(results);
      renderEngagementReport(results, runSummary);

      if (switchToResultsTab) setEngagementDetailTab('results');
    } catch (err) {
      toast(err.message, 'error');
    }
  }

  function buildMarkdownReport(results, run) {
    const target = engagementDetail.targetMatch?.name || '—';
    const scenario = engagementDetail.scenarioName || '—';
    const status = run?.status || '—';
    const total = results.length;
    const failed = results.filter(r => !!r.error_message || Number(r.status_code || 0) === 0).length;
    const successful = total - failed;

    const lines = [
      `# Engagement Report`,
      ``,
      `- Engagement: ${engagementDetail.name}`,
      `- Slug: ${engagementDetail.slug}`,
      `- Run: ${run?.id || '—'}`,
      `- Target: ${target}`,
      `- Scenario: ${scenario}`,
      `- Status: ${status}`,
      `- Started: ${formatEngagementDateTime(run?.started_at || '')}`,
      `- Total results: ${total}`,
      `- Successful: ${successful}`,
      `- Failed: ${failed}`,
      ``,
      `## Attempts`,
      ``,
      `| Seq | Prompt ID | Status | Latency | Session |`,
      `| --- | --- | --- | --- | --- |`,
    ];

    results
      .sort((a, b) => Number(a.seq || 0) - Number(b.seq || 0))
      .forEach((r) => {
        const statusText = r.error_message ? 'ERROR' : String(r.status_code || '?');
        lines.push(`| ${r.step_order || '-'} | ${r.prompt_id || '-'} | ${statusText} | ${r.latency_ms != null ? `${r.latency_ms}ms` : '-'} | ${r.session_label || '-'} |`);
      });

    return lines.join('\n');
  }

  function downloadTextFile(filename, text) {
    const blob = new Blob([text], { type: 'text/markdown;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);
  }

  $('#btn-eng-rerun')?.addEventListener('click', async () => {
    const runId = engagementDetail.activeRunId;
    if (!engagementDetail.slug || !runId) {
      toast('Select a run first', 'info');
      return;
    }
    const targetId = engagementDetail.targetMatch?.id;
    if (!targetId) {
      toast('Re-run requires a known target mapping. Select a run tied to an existing target URL.', 'error');
      return;
    }

    const source = engagementDetail.resultsByRunId.get(runId) || [];
    const payloads = source
      .filter(r => String(r.prompt_text || '').trim())
      .map((r, idx) => ({
        prompt_id: r.prompt_id || `rerun-${idx + 1}`,
        payload_id: `rerun-${String(idx + 1).padStart(3, '0')}`,
        text: r.prompt_text,
      }));

    if (!payloads.length) {
      toast('No prompt payloads available for re-run', 'error');
      return;
    }

    const btn = $('#btn-eng-rerun');
    btn.disabled = true;
    try {
      const newRunId = await API.call('start_run', {
        engagement_slug: engagementDetail.slug,
        request_id: targetId,
        payloads,
        parallelism: 4,
      });
      toast(`Re-run started: ${newRunId}`, 'success');
      await loadRuns({
        engagementSlug: engagementDetail.slug,
        autoSelectFirst: true,
        preferredRunId: newRunId,
      });
      setEngagementDetailTab('results');
    } catch (err) {
      toast(err.message, 'error');
    } finally {
      btn.disabled = false;
    }
  });

  $('#btn-eng-export-md')?.addEventListener('click', () => {
    const runId = engagementDetail.activeRunId;
    if (!runId) {
      toast('Select a run first', 'info');
      return;
    }
    const results = engagementDetail.resultsByRunId.get(runId) || [];
    const run = engagementDetail.runs.find(r => r.id === runId) || null;
    const markdown = buildMarkdownReport(results, run);
    const filename = `${engagementDetail.slug || 'engagement'}-${runId}.md`;
    downloadTextFile(filename, markdown);
    toast('Markdown report exported', 'success');
  });

  $('#btn-eng-export-pdf')?.addEventListener('click', async () => {
    const runId = engagementDetail.activeRunId;
    if (!runId) {
      toast('Select a run first', 'info');
      return;
    }
    try {
      const report = await API.call('export_findings_pdf', { run_id: runId });
      toast(`Report generated: ${report.path}`, 'success');
    } catch (err) {
      toast(err.message, 'error');
    }
  });

  $('#btn-eng-archive')?.addEventListener('click', async () => {
    const slug = engagementDetail.slug;
    if (!slug) {
      toast('Open an engagement first', 'info');
      return;
    }
    if (!confirm(`Archive engagement "${engagementDetail.name}" in the UI list?`)) return;

    archiveEngagementSlug(slug);
    clearEngagementRoute({ replace: true });
    engagementDetail.slug = null;
    engagementDetail.activeRunId = null;
    renderRunsViewEmptyState('Select an engagement to view its runs.');
    await loadEngagementList({ autoOpen: false });
    loadHomeRecentEngagements();
    toast('Engagement archived from visible lists', 'success');
  });

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

  async function applyEngagementRouteFromLocation() {
    const slug = getEngagementSlugFromRoute();
    if (!slug) return false;

    if (!$('#view-runs').classList.contains('active')) {
      showView('view-runs');
    } else {
      await loadEngagementList({ preferredSlug: slug, autoOpen: true, syncRoute: false });
    }
    return true;
  }

  window.addEventListener('popstate', () => {
    applyEngagementRouteFromLocation().catch(err => toast(err.message, 'error'));
  });

  setTimeout(() => {
    applyEngagementRouteFromLocation().catch(() => {
      // ignore invalid route on cold start
    });
  }, 0);

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
      } catch (err) { /* progress poll — transient errors are expected */ }
    }, 1000);
  }

  function stopProgressPoll() {
    if (progressPollTimer) {
      clearInterval(progressPollTimer);
      progressPollTimer = null;
    }
  }

  // ── Keyboard shortcuts ─────────────────────────────────────────────
  document.addEventListener('keydown', (e) => {
    const tag = document.activeElement?.tagName;
    const inInput = tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT';

    // / → focus picker search (when workbench active and not in an input)
    if (e.key === '/' && !inInput && $('#view-workbench').classList.contains('active')) {
      e.preventDefault();
      $('#picker-search').focus();
    }

    // Escape → close any open modal, blur picker search
    if (e.key === 'Escape') {
      document.querySelectorAll('.modal').forEach(m => { m.style.display = 'none'; });
      if (document.activeElement === $('#picker-search')) {
        $('#picker-search').blur();
      }
    }

    // Cmd/Ctrl+Enter → fire prompt
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
      if ($('#view-workbench').classList.contains('active')) {
        e.preventDefault();
        $('#btn-wb-fire').click();
      }
    }
  });

  // ── Close modals on backdrop click ─────────────────────────────────
  document.querySelectorAll('.modal').forEach(modal => {
    modal.addEventListener('click', (e) => {
      if (e.target === modal) modal.style.display = 'none';
    });
  });

  // Mark the initial view so the sidebar reflects the active state on load.
  showView('view-home');

  // ── Analyzer activation ────────────────────────────────────────────
  initAnalyzerUI();
});

// ============================================================
// Analyzer activation flow
// ============================================================
(function initAnalyzerUI() {
  // Hardware class label lookup
  const HW_LABELS = {
    apple_silicon: 'Apple Silicon (Metal)',
    x86_64_avx2:  'x86-64 AVX2 (CPU)',
    generic:       'Generic CPU (slow)',
  };

  let selectedVariantId = null;
  let downloadUnlisten = null;
  let analyzerStatus = null;

  function openAnalyzerModal() {
    $('#analyzer-modal').style.display = 'flex';
    refreshAnalyzerModal();
  }

  async function refreshAnalyzerModal() {
    // Reset state
    selectedVariantId = null;
    $('#btn-analyzer-install').disabled = true;
    $('#analyzer-download-section').style.display = 'none';
    $('#analyzer-variants').innerHTML = '<div class="analyzer-loading">loading…</div>';

    try {
      analyzerStatus = await API.call('get_analyzer_status');
      const hw = analyzerStatus.hardware;
      $('#analyzer-hw-badge').textContent = HW_LABELS[hw] || hw;
      $('#analyzer-hw-badge').dataset.hw = hw;

      if (analyzerStatus.installed) {
        $('#analyzer-install-badge').textContent =
          `installed: ${analyzerStatus.model_file}`;
        $('#analyzer-install-badge').dataset.state = 'installed';
        $('#btn-analyzer-uninstall').style.display = '';
        $('#btn-analyzer-install').textContent = 'Re-download';
      } else {
        $('#analyzer-install-badge').textContent = 'not installed';
        $('#analyzer-install-badge').dataset.state = 'none';
        $('#btn-analyzer-uninstall').style.display = 'none';
        $('#btn-analyzer-install').textContent = 'Download & Install';
      }

      const manifest = await API.call('fetch_analyzer_manifest');
      renderVariants(manifest.variants, hw);
    } catch (err) {
      $('#analyzer-variants').innerHTML =
        `<div class="analyzer-loading" style="color:var(--red)">Failed to load: ${esc(err.message)}</div>`;
    }
  }

  function renderVariants(variants, hw) {
    if (!variants || variants.length === 0) {
      $('#analyzer-variants').innerHTML =
        '<div class="analyzer-loading">No variants available.</div>';
      return;
    }

    // Sort: recommended match first, then recommended others, then rest
    const sorted = [...variants].sort((a, b) => {
      const aMatch = a.hardware === hw && a.recommended;
      const bMatch = b.hardware === hw && b.recommended;
      if (aMatch && !bMatch) return -1;
      if (!aMatch && bMatch) return 1;
      return b.recommended - a.recommended;
    });

    $('#analyzer-variants').innerHTML = sorted.map(v => {
      const isMatch = v.hardware === hw;
      const sizeGb = (v.model.size_bytes / 1e9).toFixed(1);
      const hwLabel = HW_LABELS[v.hardware] || v.hardware;
      return `
        <div class="analyzer-variant${isMatch ? ' analyzer-variant-match' : ''}"
             data-variant-id="${esc(v.id)}">
          <div class="analyzer-variant-header">
            <span class="analyzer-variant-label">${esc(v.label)}</span>
            ${v.recommended ? '<span class="chip" style="margin-left:6px;font-size:10px;">recommended</span>' : ''}
          </div>
          <div class="analyzer-variant-meta">
            <span>${esc(hwLabel)}</span>
            <span>${sizeGb} GB</span>
          </div>
        </div>`;
    }).join('');

    // Auto-select the first recommended match
    const autoSelect = sorted.find(v => v.hardware === hw && v.recommended) || sorted[0];
    if (autoSelect) selectVariant(autoSelect.id);

    $('#analyzer-variants').querySelectorAll('.analyzer-variant').forEach(card => {
      card.addEventListener('click', () => selectVariant(card.dataset.variantId));
    });
  }

  function selectVariant(id) {
    selectedVariantId = id;
    $('#analyzer-variants').querySelectorAll('.analyzer-variant').forEach(card => {
      card.classList.toggle('analyzer-variant-selected', card.dataset.variantId === id);
    });
    $('#btn-analyzer-install').disabled = false;
  }

  // ── Install button ──────────────────────────────────────────────────
  $('#btn-analyzer-install').addEventListener('click', async () => {
    if (!selectedVariantId) return;

    // Show progress section
    $('#analyzer-download-section').style.display = '';
    $('#btn-analyzer-install').disabled = true;
    $('#analyzer-progress-bar').style.width = '0%';
    $('#analyzer-progress-text').textContent = 'Starting download…';
    $('#analyzer-progress-pct').textContent = '0%';
    $('#analyzer-progress-bytes').textContent = '';

    // Subscribe to progress events before firing the command
    if (downloadUnlisten) { downloadUnlisten(); downloadUnlisten = null; }
    downloadUnlisten = await window.__TAURI__.event.listen(
      'analyzer-download-progress',
      (ev) => onDownloadProgress(ev.payload)
    );

    try {
      await API.call('download_and_install_analyzer', { variant_id: selectedVariantId });
    } catch (err) {
      showToast(`Download failed: ${err.message}`, 'error');
      $('#analyzer-download-section').style.display = 'none';
      $('#btn-analyzer-install').disabled = false;
    }
  });

  function onDownloadProgress(p) {
    const pct = Math.round(p.percent);
    $('#analyzer-progress-bar').style.width = `${pct}%`;
    $('#analyzer-progress-pct').textContent = `${pct}%`;

    if (p.bytes_total > 0) {
      const dl = (p.bytes_downloaded / 1e6).toFixed(0);
      const tot = (p.bytes_total / 1e6).toFixed(0);
      $('#analyzer-progress-bytes').textContent = `${dl} MB / ${tot} MB`;
    }

    if (p.error) {
      $('#analyzer-progress-text').textContent = `Error: ${p.error}`;
      $('#btn-analyzer-install').disabled = false;
      if (downloadUnlisten) { downloadUnlisten(); downloadUnlisten = null; }
      return;
    }

    if (p.finished) {
      $('#analyzer-progress-text').textContent = 'Installed!';
      $('#analyzer-progress-pct').textContent = '100%';
      if (downloadUnlisten) { downloadUnlisten(); downloadUnlisten = null; }
      // Refresh status
      refreshAnalyzerModal();
      // Refresh home CTA
      checkAnalyzerCta();
      showToast('Analyz0r activated. Judgments will use the local LLM on next analysis run.', 'ok');
    } else {
      $('#analyzer-progress-text').textContent = 'Downloading…';
    }
  }

  // ── Uninstall button ────────────────────────────────────────────────
  $('#btn-analyzer-uninstall').addEventListener('click', async () => {
    try {
      await API.call('uninstall_analyzer');
      showToast('Analyzer model removed.', 'ok');
      refreshAnalyzerModal();
      checkAnalyzerCta();
    } catch (err) {
      showToast(`Uninstall failed: ${err.message}`, 'error');
    }
  });

  // ── Close handlers ──────────────────────────────────────────────────
  $('#analyzer-modal-close').addEventListener('click', () => {
    $('#analyzer-modal').style.display = 'none';
  });
  $('#analyzer-modal-cancel').addEventListener('click', () => {
    $('#analyzer-modal').style.display = 'none';
  });

  // ── Settings sidebar entry-point ────────────────────────────────────
  document.querySelector('[data-nav="settings"]').addEventListener('click', openAnalyzerModal);

  // ── Home CTA ────────────────────────────────────────────────────────
  $('#btn-home-activate-analyzer').addEventListener('click', openAnalyzerModal);

  async function checkAnalyzerCta() {
    try {
      const status = await API.call('get_analyzer_status');
      $('#analyzer-cta-card').style.display = status.installed ? 'none' : '';
    } catch (_) {
      $('#analyzer-cta-card').style.display = '';
    }
  }

  // Show CTA on home view whenever it becomes active
  document.querySelector('[data-view="view-home"]').addEventListener('click', () => {
    checkAnalyzerCta();
  });

  // Check on first load
  if (window.__TAURI__) checkAnalyzerCta();
});
