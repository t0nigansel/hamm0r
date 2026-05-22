/**
 * app.js — Main UI logic for hamm0r.
 *
 * Sidebar + main panel layout with scenario-based testing.
 * All backend communication goes through API.call() (see api.js).
 */

// Module-scope DOM helpers so functions defined outside the
// DOMContentLoaded handler (like initAnalyzerUI) can reach them. The
// handler below redeclares them inside its own scope; that shadowing
// is intentional and harmless — both lookups still go to the same
// `document.querySelector`.
const $ = (sel) => document.querySelector(sel);
const $$ = (sel) => document.querySelectorAll(sel);

// Show/hide an element by toggling the .is-hidden utility class. Prefer
// this over `el.style.display = 'none' | 'flex'` so the natural display
// value stays in CSS and only one source of truth governs visibility.
const setHidden = (el, hidden) => {
  if (!el) return;
  el.classList.toggle('is-hidden', !!hidden);
};

// Top-level "always-on" handlers. Registered immediately on script
// load (not inside DOMContentLoaded) so they survive any synchronous
// failure that aborts DCL setup. Covers the Settings modal, the
// Requests view's primary buttons, and per-row request clicks.
// Functions that need closure access (startNewRequest, openRequestEditor)
// are exposed on window.__hamm0r once DCL has defined them.
window.__hamm0r = window.__hamm0r || {};

const THEME_CACHE_KEY = 'hamm0r.ui.theme.v1';

function normalizeTheme(theme) {
  const value = String(theme || '').trim().toLowerCase();
  if (
    value === 'light' ||
    value === 'spirit' ||
    value === 'spirit-testing' ||
    value === 'spirit_testing' ||
    value === 'testsolutions' ||
    value === 'test-solutions' ||
    value === 'test_solutions'
  ) {
    return 'light';
  }
  return 'default';
}

function applyTheme(theme) {
  const normalized = normalizeTheme(theme);
  document.documentElement.dataset.theme = normalized;
  try {
    localStorage.setItem(THEME_CACHE_KEY, normalized);
  } catch (_) {
    // ignore storage errors
  }
}

try {
  applyTheme(localStorage.getItem(THEME_CACHE_KEY) || 'default');
} catch (_) {
  applyTheme('default');
}

// One global error handler so silent throws inside DCL or any later
// async path become visible. Keeps the toast non-fatal.
window.addEventListener('error', (e) => {
  try {
    console.error('[hamm0r] uncaught error:', e.error || e.message, e.filename, e.lineno);
    if (typeof toast === 'function') {
      toast(`JS error: ${e.message || 'unknown'}`, 'error');
    }
  } catch (_) { /* ignore */ }
});
document.addEventListener('click', (e) => {
  const target = e.target;
  if (!target) return;

  // Open: clicking the sidebar Settings button.
  const settingsBtn = target.closest?.('[data-nav="settings"]');
  if (settingsBtn) {
    const modal = document.querySelector('#settings-modal');
    if (modal) {
      setHidden(modal, false);
      window.dispatchEvent(new CustomEvent('settings-modal-opened'));
    }
    return;
  }

  // Close: X button, Cancel button, or backdrop click.
  if (target.closest?.('#settings-modal-close, #settings-modal-cancel')) {
    setHidden(document.querySelector('#settings-modal'), true);
    return;
  }
  if (target.id === 'settings-modal') {
    setHidden(target, true);
    return;
  }

  // General/Logging/Analyz0r section switch.
  const navItem = target.closest?.('.settings-nav-item');
  if (navItem && document.querySelector('#settings-modal')?.contains(navItem)) {
    const view = navItem.dataset.settingsView;
    if (view) {
      document.querySelectorAll('.settings-nav-item').forEach((b) => {
        const isActive = b.dataset.settingsView === view;
        b.classList.toggle('active', isActive);
        b.setAttribute('aria-current', isActive ? 'page' : 'false');
      });
      document.querySelectorAll('.settings-view').forEach((p) => {
        p.classList.toggle('active', p.dataset.settingsView === view);
      });
    }
    return;
  }

  // Analyz0r subnav: Prompt / Local Judge / Hosted Judge.
  const subnavItem = target.closest?.('.settings-subnav-item');
  if (subnavItem) {
    const view = subnavItem.dataset.analyzerView;
    if (view) {
      document.querySelectorAll('.settings-subnav-item').forEach((b) => {
        b.classList.toggle('active', b.dataset.analyzerView === view);
      });
      document.querySelectorAll('.settings-subview').forEach((p) => {
        p.classList.toggle('active', p.dataset.analyzerView === view);
      });
    }
    return;
  }

  // Requests view: "+" button and "Create request" CTA.
  if (target.closest?.('#btn-new-request, #btn-request-get-started')) {
    if (typeof window.__hamm0r.startNewRequest === 'function') {
      window.__hamm0r.startNewRequest();
    } else {
      console.error('[hamm0r] startNewRequest not yet exposed; DCL setup likely failed');
    }
    return;
  }

  // Requests view: clicking a row in the list.
  const requestRow = target.closest?.('#request-list li[data-id]');
  if (requestRow) {
    const id = requestRow.dataset.id;
    if (id && typeof window.__hamm0r.openRequestEditor === 'function') {
      window.__hamm0r.openRequestEditor(id);
    } else if (id) {
      console.error('[hamm0r] openRequestEditor not yet exposed; DCL setup likely failed');
    }
    return;
  }
});

function esc(str) {
  if (str == null) return '';
  const div = document.createElement('div');
  div.textContent = String(str);
  return div.innerHTML;
}

function toast(message, type = 'info') {
  const container = $('#toast-container');
  if (!container) return;
  const el = document.createElement('div');
  el.className = `toast toast-${type}`;
  el.textContent = message;
  container.appendChild(el);
  setTimeout(() => el.remove(), 4000);
}

function toastAction(message, actionLabel, onAction, type = 'info') {
  const container = $('#toast-container');
  if (!container) return;
  const el = document.createElement('div');
  el.className = `toast toast-${type} toast-action`;

  const text = document.createElement('span');
  text.className = 'toast-message';
  text.textContent = message;
  el.appendChild(text);

  const btn = document.createElement('button');
  btn.type = 'button';
  btn.className = 'toast-link';
  btn.textContent = actionLabel;
  btn.addEventListener('click', async () => {
    try {
      await onAction();
    } catch (err) {
      toast(err.message || String(err), 'error');
    }
  });
  el.appendChild(btn);

  container.appendChild(el);
  setTimeout(() => el.remove(), 7000);
}

document.addEventListener('DOMContentLoaded', () => {
  // ── State ──────────────────────────────────────────────────────────
  let dbOpen = false;
  let activeEngagementSlug = null;
  let activeViewId = 'view-home';
  let editingPromptId = null;
  let currentScenarioId = null;
  let currentScenario = null;
  // Matrix-mode editor state. A Scenario fires every selected Request
  // against every prompt resolved from the library subset.
  let currentScenarioMatrix = {
    request_ids: [],            // selected Request ids (strings)
    request_repeats: {},        // {[req_id]: localRepeatCount} — only when > 1
    owasp_refs: [],             // e.g. ["A01", "A03"]
    categories: [],             // prompt-file stems
    shared_session: false,
  };
  let currentScenarioMatrixGlobalRequests = [];   // cache of list_requests
  let currentScenarioMatrixPromptIndex = null;    // cache of list_prompts
  let currentRunId = null;
  let progressPollTimer = null;
  let engagementProgressPollTimer = null;
  let engagementResultsPollTimer = null;
  const engagementRunActivity = new Map();
  const expectedRunTotals = new Map();
  let lastEngagementEventRefreshAt = 0;
  const ARCHIVED_ENGAGEMENTS_KEY = 'hamm0r.archivedEngagements.v1';

  // Per-row action glyphs.
  // Using unicode characters (always render, no namespace pitfalls). The
  // `.btn-icon-glyph` wrapper styles them to a consistent box.
  function glyph(ch, extraClass = '') {
    return `<span class="btn-icon-glyph ${extraClass}" aria-hidden="true">${ch}</span>`;
  }
  const ICONS = {
    rerun:     glyph('↻'),                    // U+21BB clockwise open circle arrow
    stop:      glyph('■'),                    // U+25A0 black square
    analyze:   glyph('🔍'),                   // U+1F50D magnifying glass
    exportMd:  glyph('MD', 'btn-icon-text'),  // textual badge
    exportPdf: glyph('PDF', 'btn-icon-text'), // textual badge
    copy:      glyph('<svg viewBox="0 0 16 16" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><rect x="5" y="5" width="9" height="10" rx="1"/><path d="M3 11V3a1 1 0 0 1 1-1h7"/></svg>'),
    cancel:    glyph('<svg viewBox="0 0 16 16" width="13" height="13" fill="none" stroke="#e5484d" stroke-width="2" stroke-linecap="round"><path d="M4 4l8 8M12 4l-8 8"/></svg>'),
    analyzed:  glyph('<svg viewBox="0 0 16 16" width="13" height="13" fill="none" stroke="#3dd68c" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 8l3.5 3.5L13 5"/></svg>'),
    archive:   glyph('🗑'),                   // U+1F5D1 wastebasket
  };

  const engagementDetail = {
    slug: null,
    name: '',
    activeRunId: null,
    runs: [],
    resultsByRunId: new Map(),
    triageByRunId: new Map(),
    scenarios: [],
    targetMatch: null,
    activeScenarioId: null,
    renderedReportSlug: null,
    renderedReportRunId: null,
    renderedReportHtml: null,
    scenarioName: '—',
  };

  // ── DOM refs ───────────────────────────────────────────────────────
  const $ = (sel) => document.querySelector(sel);
  const $$ = (sel) => document.querySelectorAll(sel);

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

  function parseJudgeIdentity(modelUsed) {
    const raw = String(modelUsed || '').trim();
    if (!raw) return { mode: '—', provider: '—', model: '—' };
    if (raw.startsWith('azure_openai:')) {
      return {
        mode: 'Hosted',
        provider: 'Azure OpenAI',
        model: raw.slice('azure_openai:'.length) || raw,
      };
    }
    if (raw.startsWith('ollama:')) {
      return {
        mode: 'Local',
        provider: 'Ollama',
        model: raw.slice('ollama:'.length) || raw,
      };
    }
    if (raw.startsWith('heuristic-')) {
      return {
        mode: 'Local',
        provider: 'Heuristic',
        model: raw,
      };
    }
    return {
      mode: 'Local',
      provider: 'Local Model',
      model: raw,
    };
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
      updateHomeCtas();
      loadHomeRecentEngagements();
    }
  }

  // Breadcrumb state — populated by showView (view label), onDbOpen
  // (engagement name) and loadRunResults (run id). updateBreadcrumb()
  // renders the three optional segments.
  const breadcrumbState = { viewLabel: 'home', engagementName: null, runId: null };

  function updateBreadcrumb() {
    const v = $('#breadcrumb-view');
    const eSep = $('#breadcrumb-sep-engagement');
    const e = $('#breadcrumb-engagement');
    const rSep = $('#breadcrumb-sep-run');
    const r = $('#breadcrumb-run');
    if (v) v.textContent = breadcrumbState.viewLabel;
    const hasEng = !!breadcrumbState.engagementName;
    const hasRun = hasEng && !!breadcrumbState.runId;
    if (eSep) eSep.hidden = !hasEng;
    if (e) {
      e.hidden = !hasEng;
      e.textContent = breadcrumbState.engagementName || '';
      e.classList.toggle('current', hasEng && !hasRun);
    }
    if (rSep) rSep.hidden = !hasRun;
    if (r) {
      r.hidden = !hasRun;
      r.textContent = breadcrumbState.runId || '';
    }
  }

  function onDbOpen(name, slug) {
    if (slug) activeEngagementSlug = slug;
    breadcrumbState.engagementName = name || null;
    breadcrumbState.runId = null;
    updateBreadcrumb();
    if ($('#view-home').classList.contains('active')) {
      loadHomeRecentEngagements();
      updateHomeCtas();
    }
    if ($('#view-engagements').classList.contains('active')) loadEngagementList({ autoOpen: false });
  }

  checkDatabaseStatus();

  // ── T-05 · Sidebar navigation ──────────────────────────────────────
  const VIEW_LABELS = {
    'view-home': 'home',
    'view-requests': 'requests',
    'view-prompts': 'library',
    'view-scenarios': 'scenarios',
    'view-engagements': 'engagements',
  };

  function showView(viewId) {
    activeViewId = viewId;
    $$('.nav-item[data-view]').forEach(b => b.classList.remove('active'));
    $$('.view').forEach(v => v.classList.remove('active'));

    const btn = $(`.nav-item[data-view="${viewId}"]`);
    if (btn) btn.classList.add('active');
    const viewEl = $(`#${viewId}`);
    if (viewEl) viewEl.classList.add('active');

    breadcrumbState.viewLabel = VIEW_LABELS[viewId] || viewId;
    // Leaving the engagement detail view drops the run segment.
    if (viewId !== 'view-engagements') breadcrumbState.runId = null;
    updateBreadcrumb();


    if (viewId === 'view-home') {
      updateHomeCtas();
      loadHomeRecentEngagements();
    }
    if (viewId === 'view-prompts') loadPrompts();
    if (viewId === 'view-requests') loadRequestList();
    if (viewId === 'view-engagements') loadEngagementList();
    if (viewId !== 'view-engagements') {
      stopEngagementProgressPoll();
      stopEngagementResultsPoll();
    }
    if (viewId !== 'view-engagements') clearEngagementRoute({ replace: true });
    if (dbOpen) {
      if (viewId === 'view-scenarios') loadScenarioList();
    }
    updateGlobalFireButton();
  }

  $$('.nav-item[data-view]').forEach(btn => {
    btn.addEventListener('click', () => showView(btn.dataset.view));
  });

  // Settings nav item (no view yet)
  // ── Engagement management ──────────────────────────────────────────

  async function openEngagementDialog() {
    setHidden($('#engagement-dialog'), false);
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
              unarchiveEngagementSlug(eng.slug);
              const result = await API.call('open_db', { path: eng.slug });
              dbOpen = true;
              setHidden($('#engagement-dialog'), true);
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

  function unarchiveEngagementSlug(slug) {
    if (!slug) return;
    if (!archivedEngagementSlugs.has(slug)) return;
    archivedEngagementSlugs.delete(slug);
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
    showView('view-engagements');
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

  function closeRunScenarioPicker() {
    setHidden($('#run-scenario-picker'), true);
  }

  async function runPickedScenario(scenario) {
    closeRunScenarioPicker();
    try {
      const eng = await API.call('create_engagement', {
        name: `${scenario.name || scenario.id} · run`,
      });
      await API.call('open_db', { path: eng.slug });
      const result = await API.call('start_scenario', { scenario_id: scenario.id });
      toast(`Run started (${result.run_id || result.id})`, 'success');
      showView('view-engagements');
    } catch (err) {
      toast(err.message, 'error');
    }
  }

  function describeScenarioForPicker(s) {
    const reqCount = (s.request_ids || []).length;
    const owasp = (s.library?.owasp_refs || []).join(', ');
    const cats = (s.library?.categories || []).join(', ');
    const libBits = [];
    if (owasp) libBits.push(`OWASP: ${owasp}`);
    if (cats) libBits.push(`categories: ${cats}`);
    const libSummary = libBits.length ? libBits.join(' · ') : 'no library subset';
    const reqSummary = `${reqCount} request${reqCount === 1 ? '' : 's'}`;
    const sharedBit = s.shared_session ? ' · shared session' : '';
    return `${reqSummary} · ${libSummary}${sharedBit}`;
  }

  function isScenarioRunnable(s) {
    const hasRequests = Array.isArray(s.request_ids) && s.request_ids.length > 0;
    const lib = s.library || {};
    const hasLibrary = (Array.isArray(lib.owasp_refs) && lib.owasp_refs.length > 0)
      || (Array.isArray(lib.categories) && lib.categories.length > 0);
    return hasRequests && hasLibrary;
  }

  async function openScenarioPicker() {
    const list = $('#run-scenario-picker-list');
    setHidden($('#run-scenario-picker'), false);
    list.innerHTML = '<div class="eng-list-empty">loading…</div>';
    try {
      const scenarios = await API.call('list_scenarios', {});
      if (scenarios.length === 0) {
        closeRunScenarioPicker();
        toast('No saved Scenarios yet. Build one in the Scenarios view.', 'info');
        showView('view-scenarios');
        return;
      }
      list.innerHTML = '';
      scenarios.forEach((s) => {
        const card = document.createElement('div');
        const runnable = isScenarioRunnable(s);
        card.className = 'engagement-card' + (runnable ? '' : ' disabled');
        card.innerHTML = `
          <span class="eng-name">${esc(s.name || s.id)}</span>
          <span class="eng-meta">${esc(describeScenarioForPicker(s))}</span>`;
        if (runnable) {
          card.addEventListener('click', () => runPickedScenario(s));
        } else {
          card.title = 'Scenario has no Requests or no library subset — open it in the Scenarios view to finish setup.';
        }
        list.appendChild(card);
      });
    } catch (err) {
      closeRunScenarioPicker();
      toast(err.message, 'error');
    }
  }

  // ── Quick Start (ToDo §10.1) ─────────────────────────────────────────
  // One-screen first-run flow: paste an OpenAI-compatible URL, optional
  // bearer token, pick OWASP categories, fire. The orchestration creates
  // a Request, a Scenario and an Engagement, then starts the run.
  // Auto-generated artifacts are tagged "quickstart" so the user can find
  // (and clean up) them later from the regular views.
  const QUICKSTART_OWASP_ALL = ['A01','A02','A03','A04','A05','A06','A07','A08','A09','A10'];

  function openQuickStart() {
    const modal = $('#quickstart-modal');
    if (!modal) return;
    // Reset transient state every open: clear token, clear status,
    // reset chip selection to "All", reset mode to builtin.
    $('#qs-token').value = '';
    setQuickstartStatus('');
    $('#qs-owasp-grid')?.querySelectorAll('.chip').forEach((c) => {
      c.classList.toggle('active', c.dataset.qsOwasp === 'ALL');
    });
    const modeSel = $('#qs-mode');
    if (modeSel) modeSel.value = 'builtin';
    syncQuickstartTokenRow();
    syncQuickstartMode();
    loadQuickstartRequests();
    setHidden(modal, false);
    $('#qs-url').focus();
  }

  async function loadQuickstartRequests() {
    const sel = $('#qs-existing');
    if (!sel) return;
    sel.innerHTML = '<option value="">loading…</option>';
    let list = [];
    try {
      list = await API.call('list_requests', {});
    } catch (err) {
      sel.innerHTML = '<option value="">(failed to load)</option>';
      return;
    }
    sel.innerHTML = '';
    if (list.length === 0) {
      sel.innerHTML = '<option value="">(none)</option>';
    } else {
      list.forEach((r) => {
        const opt = document.createElement('option');
        opt.value = r.id;
        const urlShort = (r.url || '').replace(/^https?:\/\//, '');
        opt.textContent = `${r.name || r.id} — ${r.method || 'POST'} ${urlShort}`;
        sel.appendChild(opt);
      });
    }
    syncQuickstartMode();
  }

  function syncQuickstartMode() {
    const mode = $('#qs-mode')?.value || 'builtin';
    const isExisting = mode === 'existing';
    $$('#quickstart-form .qs-builtin-row').forEach((row) => {
      row.style.display = isExisting ? 'none' : '';
    });
    if (isExisting) syncQuickstartTokenRow();
    const sel = $('#qs-existing');
    const hasRequests = !!sel && sel.options.length > 0 && sel.options[0].value !== '';
    const existingRow = $('#qs-existing-row');
    const emptyRow = $('#qs-existing-empty-row');
    if (existingRow) existingRow.style.display = isExisting && hasRequests ? '' : 'none';
    if (emptyRow) emptyRow.style.display = isExisting && !hasRequests ? '' : 'none';
  }

  function closeQuickStart() {
    setHidden($('#quickstart-modal'), true);
  }

  function setQuickstartStatus(text) {
    const el = $('#qs-status');
    if (el) el.textContent = text || '';
  }

  function syncQuickstartTokenRow() {
    const mode = $('#qs-mode')?.value || 'builtin';
    const auth = $('#qs-auth')?.value || 'bearer';
    const row = $('#qs-token-row');
    if (row) row.style.display = mode === 'builtin' && auth === 'bearer' ? '' : 'none';
  }

  function readQuickstartOwaspRefs() {
    const chips = $('#qs-owasp-grid')?.querySelectorAll('.chip.active') || [];
    const picks = [...chips].map((c) => c.dataset.qsOwasp);
    if (picks.includes('ALL')) return QUICKSTART_OWASP_ALL.slice();
    return picks;
  }

  async function runQuickStart() {
    const mode = $('#qs-mode')?.value || 'builtin';
    const owaspRefs = readQuickstartOwaspRefs();
    if (owaspRefs.length === 0) {
      toast('Pick at least one OWASP category', 'error'); return;
    }

    let requestId;
    let url, model, auth, token;
    if (mode === 'existing') {
      requestId = ($('#qs-existing')?.value || '').trim();
      if (!requestId) {
        toast('Pick a Request, or create one first', 'error'); return;
      }
    } else {
      url = ($('#qs-url')?.value || '').trim();
      model = ($('#qs-model')?.value || '').trim();
      auth = $('#qs-auth')?.value || 'bearer';
      token = ($('#qs-token')?.value || '').trim();
      if (!url) { toast('Endpoint URL is required', 'error'); return; }
      if (!model) { toast('Model is required', 'error'); return; }
      if (auth === 'bearer' && !token) {
        toast('Paste a bearer token, or switch auth to None', 'error'); return;
      }
    }

    const fireBtn = $('#btn-quickstart-fire');
    if (fireBtn) fireBtn.disabled = true;
    try {
      const ts = Date.now();
      if (mode === 'builtin') {
        const tokenEnv = `HAMM0R_QUICKSTART_${ts}`;
        if (auth === 'bearer') {
          setQuickstartStatus('Storing token in OS keychain…');
          await API.call('set_bearer_token', { var: tokenEnv, token });
        }

        setQuickstartStatus('Creating Request…');
        requestId = `quickstart-${ts}`;
        const request = {
          version: 1,
          id: requestId,
          name: `Quick Start · ${new Date().toISOString().slice(0, 10)}`,
          method: 'POST',
          url,
          auth: auth === 'bearer'
            ? { type: 'bearer', token_env: tokenEnv }
            : { type: 'none' },
          headers: { 'Content-Type': 'application/json' },
          body: {
            format: 'json',
            content: {
              model,
              messages: [{ role: 'user', content: '{{prompt}}' }],
            },
          },
          response: {
            extract: { type: 'jsonpath', path: '$.choices[0].message.content' },
          },
          timeout_seconds: 60,
          adapter: 'open-ai-compat',
          tag: 'quickstart',
        };
        await API.call('save_request_global', { request });
      }

      setQuickstartStatus('Creating Scenario…');
      const scenarioName = `Quick Start · ${new Date().toISOString().slice(0, 19).replace('T', ' ')}`;
      const scenario = await API.call('create_scenario', { name: scenarioName });
      await API.call('update_scenario', {
        id: scenario.id,
        name: scenarioName,
        request_ids: [requestId],
        library: { owasp_refs: owaspRefs, categories: [] },
      });

      setQuickstartStatus('Creating Engagement…');
      const eng = await API.call('create_engagement', { name: scenarioName });
      await API.call('open_db', { path: eng.slug });
      dbOpen = true;
      onDbOpen(eng.name || scenarioName, eng.slug);

      setQuickstartStatus('Firing…');
      await API.call('start_scenario_run', {
        engagement_slug: eng.slug,
        scenario_id: scenario.id,
      });

      closeQuickStart();
      showView('view-engagements');
      toast('Quick Start run fired', 'success');
    } catch (err) {
      setQuickstartStatus(`Error: ${err.message}`);
      toast(`Quick Start failed: ${err.message}`, 'error');
    } finally {
      if (fireBtn) fireBtn.disabled = false;
    }
  }

  $('#quickstart-close')?.addEventListener('click', closeQuickStart);
  $('#btn-quickstart-cancel')?.addEventListener('click', closeQuickStart);
  $('#qs-auth')?.addEventListener('change', syncQuickstartTokenRow);
  $('#qs-mode')?.addEventListener('change', syncQuickstartMode);
  $('#btn-qs-create-request')?.addEventListener('click', () => {
    closeQuickStart();
    showView('view-requests');
    if (typeof window.__hamm0r?.startNewRequest === 'function') {
      window.__hamm0r.startNewRequest();
    }
  });
  $('#qs-owasp-grid')?.addEventListener('click', (e) => {
    const chip = e.target.closest('.chip[data-qs-owasp]');
    if (!chip) return;
    const value = chip.dataset.qsOwasp;
    const grid = $('#qs-owasp-grid');
    if (value === 'ALL') {
      // Toggle "All" exclusively — selecting it clears the others.
      grid.querySelectorAll('.chip').forEach((c) => {
        c.classList.toggle('active', c.dataset.qsOwasp === 'ALL');
      });
    } else {
      grid.querySelector('.chip[data-qs-owasp="ALL"]')?.classList.remove('active');
      chip.classList.toggle('active');
      // Falling back to "All" if nothing else is selected.
      const anyActive = grid.querySelectorAll('.chip.active').length > 0;
      if (!anyActive) {
        grid.querySelector('.chip[data-qs-owasp="ALL"]')?.classList.add('active');
      }
    }
  });
  $('#quickstart-form')?.addEventListener('submit', (e) => {
    e.preventDefault();
    runQuickStart();
  });

  const HOME_CTA_ACTIONS = {
    open_engagement: openEngagementDialog,
    run_scenario: openScenarioPicker,
    open_scenarios: () => showView('view-scenarios'),
    open_requests: () => showView('view-requests'),
    open_prompts: () => showView('view-prompts'),
    quick_start: openQuickStart,
  };

  // Render the two Home tiles based on what the user has in their workspace.
  // Three states: no Requests yet, Requests but no runnable Scenarios, ready.
  // Tiles carry a data-cta-action attribute; one delegated handler dispatches.
  async function updateHomeCtas() {
    const grid = $('#home-cta-grid');
    if (!grid) return;

    let requestCount = 0;
    let runnableScenarioCount = 0;
    try {
      const [requests, scenarios] = await Promise.all([
        API.call('list_requests', {}),
        API.call('list_scenarios', {}),
      ]);
      requestCount = requests.length;
      runnableScenarioCount = scenarios.filter(isScenarioRunnable).length;
    } catch (_) {
      // On failure, fall through to the "ready" tiles — they degrade gracefully.
      runnableScenarioCount = 1;
    }

    let tiles;
    if (!dbOpen) {
      tiles = [
        { action: 'quick_start', primary: true, eyebrow: 'First run',
          title: 'Quick Start',
          desc: 'Paste an endpoint URL, pick OWASP categories, fire. One screen.' },
        { action: 'open_engagement', primary: false, eyebrow: 'Or',
          title: 'Open or create an Engagement',
          desc: 'Full control: build Requests and Scenarios manually.' },
      ];
    } else if (requestCount === 0) {
      tiles = [
        { action: 'open_requests', primary: true, eyebrow: 'Start', title: 'Create your first Request',
          desc: 'Define an HTTP endpoint to attack. An Ollama starter is bundled.' },
        { action: 'open_prompts', primary: false, eyebrow: 'Browse', title: 'Explore the Prompt library',
          desc: 'Look at the bundled OWASP attacks before you build a Request.' },
      ];
    } else if (runnableScenarioCount === 0) {
      tiles = [
        { action: 'open_scenarios', primary: true, eyebrow: 'Build', title: 'Create your first Scenario',
          desc: 'Compose a Request × prompts matrix — that’s what gets fired.' },
        { action: 'open_requests', primary: false, eyebrow: 'Manage', title: 'Edit Requests',
          desc: 'Adjust the Requests you have saved so far.' },
      ];
    } else {
      tiles = [
        { action: 'run_scenario', primary: true, eyebrow: 'Run', title: 'Run a Scenario',
          desc: 'Pick a saved Scenario; fires into a fresh engagement.' },
        { action: 'open_scenarios', primary: false, eyebrow: 'Build', title: 'Open Scenarios',
          desc: 'Compose a new Scenario — Requests × prompts.' },
      ];
    }

    grid.innerHTML = tiles.map(t => `
      <button class="landing-card${t.primary ? ' landing-card-primary' : ''}" data-cta-action="${t.action}">
        <span class="landing-eyebrow">${t.eyebrow}</span>
        <strong>${t.title}</strong>
        <span>${t.desc}</span>
      </button>
    `).join('');
  }

  $('#home-cta-grid')?.addEventListener('click', (e) => {
    const btn = e.target.closest('[data-cta-action]');
    if (!btn) return;
    const fn = HOME_CTA_ACTIONS[btn.dataset.ctaAction];
    if (fn) fn();
  });

  $('#run-scenario-picker-close')?.addEventListener('click', closeRunScenarioPicker);
  $('#run-scenario-picker-cancel')?.addEventListener('click', closeRunScenarioPicker);
  $('#btn-home-refresh-recents')?.addEventListener('click', () => {
    loadHomeRecentEngagements();
  });
  $('#engagement-dialog-close').addEventListener('click', () => {
    setHidden($('#engagement-dialog'), true);
  });
  $('#engagement-dialog-cancel').addEventListener('click', () => {
    setHidden($('#engagement-dialog'), true);
  });
  $('#engagement-form').addEventListener('submit', async (e) => {
    e.preventDefault();
    const name = $('#eng-name').value.trim();
    const seed = $('#eng-seed').checked;
    if (!name) return;
    try {
      const result = await API.call('create_engagement', { name });
      unarchiveEngagementSlug(result.slug);
      await API.call('open_db', { path: result.slug });
      dbOpen = true;
      setHidden($('#engagement-dialog'), true);
      $('#eng-name').value = '';
      if (seed) {
        await API.call('seed_library', { update: false });
        toast('Prompt library seeded', 'success');
      }
      toast(`Engagement created: ${name}`, 'success');
      onDbOpen(result.name || name, result.slug);
    } catch (err) { toast(err.message, 'error'); }
  });




  // ── Requests view (top-level) ──────────────────────────────────────
  // Independent of any Target. Backed by the *_global Tauri commands.
  // Editing a Request here updates the same files the Target editor reads,
  // so a Request created here is available in any Target's request list.

  const requestEditor = {
    /** id of the request currently being edited; '' for a new draft. */
    currentId: '',
    /** Track the editor's body mode so Save serialises the right shape. */
    bodyMode: 'structured', // 'structured' | 'raw'
    /** Pending delete id while the references dialog is open. */
    pendingDeleteId: '',
  };

  function resolveGlobalFireAction() {
    if (activeViewId === 'view-requests') {
      return requestEditor.currentId
        ? { enabled: true, mode: 'request', title: 'Fire selected request' }
        : { enabled: false, mode: 'request', title: 'Select a request to fire' };
    }

    if (activeViewId === 'view-scenarios') {
      return currentScenarioId
        ? { enabled: true, mode: 'scenario', title: 'Fire selected scenario' }
        : { enabled: false, mode: 'scenario', title: 'Select a scenario to fire' };
    }

    if (activeViewId === 'view-engagements') {
      if (!engagementDetail.slug) {
        return { enabled: false, mode: 'engagement', title: 'Select an engagement to fire' };
      }
      if (!engagementDetail.activeScenarioId) {
        return { enabled: false, mode: 'engagement', title: 'Selected engagement has no scenario run to fire' };
      }
      return { enabled: true, mode: 'engagement', title: 'Fire selected engagement scenario' };
    }

    return { enabled: false, mode: '', title: 'Select a request, scenario, or engagement to fire' };
  }

  function updateGlobalFireButton() {
    const btn = $('#btn-global-fire');
    if (!btn) return;
    const busy = btn.dataset.busy === 'true';
    const action = resolveGlobalFireAction();
    btn.disabled = busy || !action.enabled;
    btn.title = busy ? 'Fire is running' : action.title;
    btn.setAttribute('aria-label', btn.title);
    btn.dataset.fireMode = action.mode || '';
  }

  function setTopbarProgress(completed = 0, total = 0, state = 'running') {
    const root = $('#topbar-progress');
    const bar = $('#topbar-progress-bar');
    const count = $('#topbar-progress-count');
    if (!root || !bar || !count) return;

    const safeCompleted = Math.max(0, Number(completed) || 0);
    const safeTotal = Math.max(0, Number(total) || 0);
    const width = 42;
    const filled = safeTotal > 0
      ? Math.max(0, Math.min(width, Math.round((safeCompleted / safeTotal) * width)))
      : 0;
    bar.textContent = `[${'='.repeat(filled)}${' '.repeat(width - filled)}]`;
    count.textContent = `${safeCompleted}/${safeTotal || '?'}`;
    root.dataset.state = state;
    root.style.visibility = 'visible';
  }

  function clearTopbarProgress() {
    const root = $('#topbar-progress');
    if (!root) return;
    root.style.visibility = 'hidden';
    root.dataset.state = '';
    // Also close the detail panel if it was open.
    setTopbarDetailOpen(false);
    resetTopbarDetail();
  }

  // ── Expand panel: opens on click of the compact bar, shows live run info.
  const topbarDetail = {
    runId: null,
    startedAtMs: null,
    elapsedTimer: null,
    seq: 0,
    total: 0,
    errors: 0,
    requestId: null,
    promptId: null,
  };

  function setTopbarDetailOpen(open) {
    const panel = $('#topbar-progress-detail');
    const toggle = $('#topbar-progress-toggle');
    if (!panel || !toggle) return;
    panel.hidden = !open;
    toggle.setAttribute('aria-expanded', open ? 'true' : 'false');
    toggle.title = open ? 'Hide run details' : 'Show run details';
  }

  function renderTopbarDetail() {
    const set = (id, v) => { const el = $('#' + id); if (el) el.textContent = v || '—'; };
    set('tpd-run', topbarDetail.runId);
    const elapsed = topbarDetail.startedAtMs
      ? formatElapsed(Date.now() - topbarDetail.startedAtMs)
      : '—';
    set('tpd-elapsed', elapsed);
    set('tpd-attempts', topbarDetail.total > 0
      ? `${topbarDetail.seq}/${topbarDetail.total}`
      : `${topbarDetail.seq}`);
    set('tpd-errors', String(topbarDetail.errors || 0));
    set('tpd-request', topbarDetail.requestId);
    set('tpd-prompt', topbarDetail.promptId);
  }

  function formatElapsed(ms) {
    const s = Math.max(0, Math.floor(ms / 1000));
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    const sec = s % 60;
    if (h > 0) return `${h}h ${String(m).padStart(2, '0')}m ${String(sec).padStart(2, '0')}s`;
    if (m > 0) return `${m}m ${String(sec).padStart(2, '0')}s`;
    return `${sec}s`;
  }

  function resetTopbarDetail() {
    topbarDetail.runId = null;
    topbarDetail.startedAtMs = null;
    topbarDetail.seq = 0;
    topbarDetail.total = 0;
    topbarDetail.errors = 0;
    topbarDetail.requestId = null;
    topbarDetail.promptId = null;
    if (topbarDetail.elapsedTimer) {
      clearInterval(topbarDetail.elapsedTimer);
      topbarDetail.elapsedTimer = null;
    }
    renderTopbarDetail();
  }

  function updateTopbarDetailFromProgress(ev) {
    if (!ev || !ev.run_id) return;
    // First sighting of this run? Capture start timestamp and start ticking.
    if (topbarDetail.runId !== ev.run_id) {
      topbarDetail.runId = ev.run_id;
      topbarDetail.startedAtMs = Date.now();
      topbarDetail.errors = 0;
      if (topbarDetail.elapsedTimer) clearInterval(topbarDetail.elapsedTimer);
      topbarDetail.elapsedTimer = setInterval(renderTopbarDetail, 1000);
    }
    if (Number.isFinite(Number(ev.seq))) topbarDetail.seq = Number(ev.seq);
    if (Number.isFinite(Number(ev.total))) topbarDetail.total = Number(ev.total);
    if (ev.error) topbarDetail.errors += 1;
    if (ev.request_id) topbarDetail.requestId = ev.request_id;
    if (ev.prompt_id) topbarDetail.promptId = ev.prompt_id;
    if (ev.finished) {
      if (topbarDetail.elapsedTimer) {
        clearInterval(topbarDetail.elapsedTimer);
        topbarDetail.elapsedTimer = null;
      }
    }
    renderTopbarDetail();
  }

  $('#topbar-progress-toggle')?.addEventListener('click', () => {
    // Only respond when there's actually a run in flight (or finished without dismissal).
    const root = $('#topbar-progress');
    if (!root || root.style.visibility === 'hidden') return;
    const panel = $('#topbar-progress-detail');
    setTopbarDetailOpen(panel?.hidden !== false ? true : false);
  });

  async function fireSelectedRequest() {
    if (!requestEditor.currentId) {
      toast('Select a request first', 'error');
      return;
    }
    setTopbarProgress(0, 1, 'running');
    try {
      const request = buildRequestFromForm();
      const result = await API.call('test_request', {
        request,
        session_strategy: 'none',
        session_field: null,
        prompt_text: $('#req-test-prompt').value,
      });
      renderRequestTestResult(result);
      toast(
        `Request fired: ${result.status}`,
        result.status >= 200 && result.status < 400 ? 'success' : 'info',
      );
      setTopbarProgress(1, 1, result.status >= 200 && result.status < 400 ? 'done' : 'error');
    } catch (err) {
      clearRequestTestResult();
      setTopbarProgress(0, 1, 'error');
      toast(err.message, 'error');
    }
  }

  function renderRequestHeaders(headers) {
    const root = $('#req-headers');
    root.innerHTML = '';
    Object.entries(headers || {}).forEach(([k, v]) => {
      addRequestHeaderRow(k, v);
    });
    if (Object.keys(headers || {}).length === 0) {
      addRequestHeaderRow('Content-Type', 'application/json');
    }
  }

  function addRequestHeaderRow(name = '', value = '') {
    const row = document.createElement('div');
    row.className = 'header-row';
    row.style.cssText = 'display:flex;gap:6px;margin-bottom:4px;';
    row.innerHTML = `
      <input type="text" class="req-header-name"  placeholder="Header"   value="${esc(name)}"  style="flex:0 0 35%;">
      <input type="text" class="req-header-value" placeholder="value"    value="${esc(value)}" style="flex:1;">
      <button type="button" class="btn btn-ghost req-header-remove" style="font-size:11px;padding:4px 10px;">×</button>
    `;
    row.querySelector('.req-header-remove').addEventListener('click', () => row.remove());
    $('#req-headers').appendChild(row);
  }

  function readRequestHeadersFromForm() {
    const out = {};
    $$('#req-headers .header-row').forEach(row => {
      const name = row.querySelector('.req-header-name').value.trim();
      const value = row.querySelector('.req-header-value').value;
      if (name) out[name] = value;
    });
    return out;
  }

  function blankRequest() {
    return {
      version: 1,
      id: '',
      name: '',
      method: 'POST',
      url: '',
      auth: { type: 'none' },
      headers: { 'Content-Type': 'application/json' },
      body: {
        format: 'json',
        content: { model: 'gpt-4', messages: [{ role: 'user', content: '{{prompt}}' }] },
      },
      response: { extract: { type: 'raw' } },
      timeout_seconds: 30,
      adapter: 'custom-rest',
    };
  }

  function setRequestBodyMode(mode) {
    requestEditor.bodyMode = mode;
    $$('#req-body-tabs .tab').forEach(t => {
      t.classList.toggle('tab-active', t.dataset.bodyTab === mode);
    });
    $('#req-body-structured').style.display = mode === 'structured' ? '' : 'none';
    $('#req-body-raw').style.display = mode === 'raw' ? '' : 'none';
    updatePromptDetector();
  }

  function updatePromptDetector() {
    const text = requestEditor.bodyMode === 'raw'
      ? $('#req-body-raw-text').value
      : $('#req-body-json').value;
    const has = /\{\{\s*prompt\s*\}\}/.test(text);
    const el = $('#req-prompt-detector');
    el.innerHTML = has
      ? '<span style="color:var(--ok,#3a3)">✓ {{prompt}} placeholder detected</span>'
      : '<span style="color:var(--warn,#c80)">⚠ no {{prompt}} placeholder — payload will not be substituted</span>';
  }

  function populateRequestEditor(req) {
    $('#req-name').value = req.name || '';
    $('#req-id').value = req.id || '';
    $('#req-id').disabled = !!req.id; // id is the filename; immutable after creation
    if ($('#req-tag')) $('#req-tag').value = req.tag || '';
    $('#req-method').value = (req.method || 'POST').toUpperCase();
    $('#req-url').value = req.url || '';
    renderRequestHeaders(req.headers || {});

    // Auth
    const auth = req.auth || { type: 'none' };
    $('#req-auth-type').value = auth.type || 'none';
    $('#req-auth-bearer').style.display = auth.type === 'bearer' ? '' : 'none';
    $('#req-auth-custom').style.display = auth.type === 'custom-header' ? '' : 'none';
    $('#req-auth-token-env').value = auth.token_env || '';
    $('#req-auth-header-name').value = auth.header_name || '';
    $('#req-auth-value-env').value = auth.value_env || '';
    refreshRequestAuthTokenStatus().catch((err) => {
      console.error('refreshRequestAuthTokenStatus', err);
    });

    // Body
    const fmt = req.body && req.body.format;
    if (fmt === 'raw') {
      const raw = typeof req.body.content === 'string' ? req.body.content : JSON.stringify(req.body.content || '');
      $('#req-body-raw-text').value = raw;
      $('#req-body-json').value = '';
      setRequestBodyMode('raw');
    } else {
      const content = (req.body && req.body.content) ?? {};
      $('#req-body-json').value = typeof content === 'string'
        ? content
        : JSON.stringify(content, null, 2);
      $('#req-body-raw-text').value = '';
      setRequestBodyMode('structured');
    }

    // Response extract
    const ext = (req.response && req.response.extract) || { type: 'raw' };
    $('#req-extract-type').value = ext.type || 'raw';
    $('#req-extract-path').value = ext.path || ext.pattern || '';
    $('#req-extract-path').style.display = ext.type === 'raw' ? 'none' : '';
    if ($('#req-bind')) $('#req-bind').value = (req.response && req.response.bind) || '';
    if ($('#req-result-columns')) {
      $('#req-result-columns').value = (req.response?.result_columns || [])
        .map((col) => `${col.label || col.id}: ${col.path}`)
        .join('\n');
    }

    $('#req-timeout').value = Number(req.timeout_seconds || 30);
    $('#req-test-prompt').value = req.test_payload || '';
    $('#btn-req-delete').style.display = req.id ? '' : 'none';
    clearRequestTestResult();
  }

  function renderRequestTestResult(result) {
    $('#req-test-result').style.display = '';
    $('#req-test-status').textContent = String(result.status ?? '—');
    $('#req-test-duration').textContent = `${result.duration_ms ?? 0} ms`;
    $('#req-test-request-line').value =
      [String(result.request_method || '').toUpperCase(), String(result.request_url || '').trim()]
        .filter(Boolean)
        .join(' ');
    $('#req-test-request-headers').value = Object.entries(result.request_headers || {})
      .map(([key, value]) => `${key}: ${value}`)
      .join('\n');
    $('#req-test-request-body').value = result.request_body || '';
    $('#req-test-response-headers').value = Object.entries(result.response_headers || {})
      .map(([key, value]) => `${key}: ${value}`)
      .join('\n');
    $('#req-test-response-extracted').value = result.extracted_response_body || '';
    $('#req-test-response-raw').value = result.raw_response_body || '';
  }

  function clearRequestTestResult() {
    $('#req-test-result').style.display = 'none';
    $('#req-test-status').textContent = '—';
    $('#req-test-duration').textContent = '—';
    $('#req-test-request-line').value = '';
    $('#req-test-request-headers').value = '';
    $('#req-test-request-body').value = '';
    $('#req-test-response-headers').value = '';
    $('#req-test-response-extracted').value = '';
    $('#req-test-response-raw').value = '';
  }

  function buildRequestFromForm() {
    const headers = readRequestHeadersFromForm();
    const authType = $('#req-auth-type').value;
    let auth = { type: 'none' };
    if (authType === 'bearer') {
      auth = { type: 'bearer', token_env: $('#req-auth-token-env').value.trim() };
    } else if (authType === 'custom-header') {
      auth = {
        type: 'custom-header',
        header_name: $('#req-auth-header-name').value.trim(),
        value_env: $('#req-auth-value-env').value.trim(),
      };
    }

    let body;
    if (requestEditor.bodyMode === 'raw') {
      body = { format: 'raw', content: $('#req-body-raw-text').value };
    } else {
      const text = $('#req-body-json').value.trim();
      let parsed;
      try {
        parsed = text === '' ? {} : JSON.parse(text);
      } catch (e) {
        throw new Error(`Body is not valid JSON: ${e.message}`);
      }
      body = { format: 'json', content: parsed };
    }

    const extType = $('#req-extract-type').value;
    let extract;
    if (extType === 'jsonpath') {
      extract = { type: 'jsonpath', path: $('#req-extract-path').value.trim() };
    } else if (extType === 'regex') {
      extract = { type: 'regex', pattern: $('#req-extract-path').value.trim() };
    } else {
      extract = { type: 'raw' };
    }

    const bindRaw = ($('#req-bind')?.value || '').trim();
    const bind = bindRaw === '' ? null : bindRaw;
    const resultColumns = parseResultColumnsFromForm();
    const tagRaw = ($('#req-tag')?.value || '').trim();
    const tag = tagRaw === '' ? null : tagRaw;

    const out = {
      version: 1,
      id: $('#req-id').value.trim(),
      name: $('#req-name').value.trim(),
      method: $('#req-method').value.toUpperCase(),
      url: $('#req-url').value.trim(),
      auth,
      headers,
      body,
      response: {
        extract,
        ...(bind ? { bind } : {}),
        ...(resultColumns.length ? { result_columns: resultColumns } : {}),
      },
      timeout_seconds: Math.max(1, Number($('#req-timeout').value || 30)),
      adapter: body.format === 'raw' ? 'raw-http' : 'custom-rest',
    };
    if (tag) out.tag = tag;
    const testPayload = $('#req-test-prompt')?.value || '';
    if (testPayload !== '') out.test_payload = testPayload;
    return out;
  }

  function metricIdFromLabel(label) {
    const id = String(label || '')
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-+|-+$/g, '');
    return id || 'metric';
  }

  function parseResultColumnsFromForm() {
    const raw = $('#req-result-columns')?.value || '';
    return raw
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter(Boolean)
      .map((line) => {
        const splitAt = line.indexOf(':');
        if (splitAt < 0) {
          const path = line.trim();
          return { id: metricIdFromLabel(path), label: path, path };
        }
        const label = line.slice(0, splitAt).trim();
        const path = line.slice(splitAt + 1).trim();
        return { id: metricIdFromLabel(label), label, path };
      })
      .filter((col) => col.label && col.path);
  }

  function uniqueRequestCopyId(baseId, requests) {
    const existing = new Set(requests.map((r) => r.id));
    const base = `${String(baseId || 'request').replace(/-copy-\d+$/, '')}-copy`;
    if (!existing.has(base)) return base;
    for (let n = 2; n < 10000; n += 1) {
      const candidate = `${base}-${n}`;
      if (!existing.has(candidate)) return candidate;
    }
    return `${base}-${Date.now()}`;
  }

  async function copyRequestFromList(id) {
    try {
      const [source, requests] = await Promise.all([
        API.call('get_request', { id }),
        API.call('list_requests', {}),
      ]);
      if (!source) {
        toast(`Request '${id}' not found`, 'error');
        return;
      }

      const copy = JSON.parse(JSON.stringify(source));
      copy.id = uniqueRequestCopyId(source.id, requests);
      copy.name = `${source.name || source.id} Copy`;

      await API.call('save_request_global', { request: copy });
      currentScenarioMatrixGlobalRequests = [];
      toast(`Copied request '${source.name || source.id}'`, 'success');
      await loadRequestList(copy.id);
    } catch (err) {
      toast(err.message, 'error');
    }
  }

  // Currently-active tag filter for the Requests list. '' = ALL.
  let requestTagFilter = '';

  function renderRequestTagFilter(allRequests) {
    const bar = $('#request-tag-filter');
    if (!bar) return;

    const tags = Array.from(new Set(
      allRequests.map(r => (r.tag || '').trim()).filter(Boolean)
    )).sort();

    // Only show the bar if there's something to filter — single-tag or
    // all-untagged lists don't benefit from a one-option chip bar.
    if (tags.length === 0) {
      setHidden(bar, true);
      bar.innerHTML = '';
      requestTagFilter = '';
      return;
    }

    setHidden(bar, false);
    const untaggedCount = allRequests.filter(r => !(r.tag || '').trim()).length;
    const chips = [
      `<button class="chip${requestTagFilter === '' ? ' active' : ''}" data-tag="">ALL</button>`,
      ...tags.map(t => `<button class="chip${requestTagFilter === t ? ' active' : ''}" data-tag="${esc(t)}">${esc(t)}</button>`),
    ];
    if (untaggedCount > 0) {
      chips.push(`<button class="chip${requestTagFilter === '__untagged__' ? ' active' : ''}" data-tag="__untagged__">UNTAGGED</button>`);
    }
    bar.innerHTML = chips.join('');
  }

  $('#request-tag-filter')?.addEventListener('click', (e) => {
    const chip = e.target.closest('.chip[data-tag]');
    if (!chip) return;
    requestTagFilter = chip.dataset.tag;
    loadRequestList();
  });

  async function loadRequestList(selectAfter = '') {
    const list = $('#request-list');
    list.innerHTML = '';
    let requests;
    try {
      requests = await API.call('list_requests', {});
    } catch (err) {
      toast(err.message, 'error');
      return;
    }

    renderRequestTagFilter(requests);

    const filtered = requests.filter(r => {
      if (requestTagFilter === '') return true;
      const tag = (r.tag || '').trim();
      if (requestTagFilter === '__untagged__') return tag === '';
      return tag === requestTagFilter;
    });

    $('#request-empty').style.display = requests.length === 0 ? '' : 'none';

    filtered.forEach(r => {
      const li = document.createElement('li');
      li.className = 'target-card-row';
      if (r.id === requestEditor.currentId) li.classList.add('active');
      li.dataset.id = r.id;
      const urlShort = (r.url || '').replace(/^https?:\/\//, '');
      const tagBadge = (r.tag || '').trim()
        ? `<span class="tag" style="margin-left:6px;font-size:10px;opacity:0.75;">${esc(r.tag)}</span>`
        : '';
      li.innerHTML = `
        <div class="target-card-name">${esc(r.name || r.id)}${tagBadge}</div>
        <div class="target-card-url">${esc(r.method || 'POST')} · ${esc(urlShort)}</div>`;
      li.addEventListener('click', () => openRequestEditor(r.id));
      const copyBtn = document.createElement('button');
      copyBtn.type = 'button';
      copyBtn.className = 'btn-icon btn-row-copy';
      copyBtn.title = 'Copy request';
      copyBtn.setAttribute('aria-label', 'Copy request');
      copyBtn.innerHTML = ICONS.copy;
      copyBtn.addEventListener('click', (e) => {
        e.stopPropagation();
        copyRequestFromList(r.id);
      });
      const deleteBtn = document.createElement('button');
      deleteBtn.type = 'button';
      deleteBtn.className = 'btn-icon btn-row-delete';
      deleteBtn.title = 'Delete request';
      deleteBtn.setAttribute('aria-label', 'Delete request');
      deleteBtn.innerHTML = ICONS.archive;
      deleteBtn.addEventListener('click', (e) => {
        e.stopPropagation();
        attemptDeleteRequest(r.id);
      });
      li.appendChild(copyBtn);
      li.appendChild(deleteBtn);
      list.appendChild(li);
    });

    if (selectAfter) {
      openRequestEditor(selectAfter);
    } else {
      // Show welcome when nothing is selected (form is hidden).
      if ($('#request-form').style.display === 'none' || !$('#request-form').style.display) {
        $('#request-welcome').style.display = '';
      }
    }
  }

  function startNewRequest() {
    requestEditor.currentId = '';
    populateRequestEditor(blankRequest());
    $('#req-id').disabled = false;
    $('#request-welcome').style.display = 'none';
    $('#request-form').style.display = '';
    $('#btn-req-delete').style.display = 'none';
    $$('#request-list .target-card-row').forEach((row) => row.classList.remove('active'));
    updateGlobalFireButton();
    setTimeout(() => $('#req-name').focus(), 0);
  }

  async function openRequestEditor(id) {
    let req;
    try {
      req = await API.call('get_request', { id });
    } catch (err) {
      toast(err.message, 'error');
      return;
    }
    if (!req) {
      toast(`Request '${id}' not found`, 'error');
      return;
    }
    requestEditor.currentId = req.id;
    populateRequestEditor(req);
    $('#request-welcome').style.display = 'none';
    $('#request-form').style.display = '';
    $$('#request-list .target-card-row').forEach((row) => {
      row.classList.toggle('active', row.dataset.id === req.id);
    });
    updateGlobalFireButton();
  }

  // Expose for the top-level click delegation so the Requests buttons
  // and per-row clicks work even if a later DCL setup step throws.
  window.__hamm0r.startNewRequest = startNewRequest;
  window.__hamm0r.openRequestEditor = openRequestEditor;

  // Body tab switching
  document.addEventListener('click', (e) => {
    const tab = e.target.closest('#req-body-tabs .tab');
    if (!tab) return;
    setRequestBodyMode(tab.dataset.bodyTab);
  });

  // Live prompt-placeholder detection on either textarea
  ['req-body-json', 'req-body-raw-text'].forEach(id => {
    document.addEventListener('input', (e) => {
      if (e.target && e.target.id === id) updatePromptDetector();
    });
  });

  // Auth-type dependent field toggling
  document.addEventListener('change', (e) => {
    if (e.target && e.target.id === 'req-auth-type') {
      $('#req-auth-bearer').style.display = e.target.value === 'bearer' ? '' : 'none';
      $('#req-auth-custom').style.display = e.target.value === 'custom-header' ? '' : 'none';
      if (e.target.value === 'bearer') {
        refreshRequestAuthTokenStatus().catch((err) => {
          console.error('refreshRequestAuthTokenStatus', err);
        });
      }
    }
    if (e.target && e.target.id === 'req-extract-type') {
      $('#req-extract-path').style.display = e.target.value === 'raw' ? 'none' : '';
    }
  });

  // Wire the static buttons (these elements only ever exist once).
  if ($('#btn-new-request')) {
    $('#btn-new-request').addEventListener('click', startNewRequest);
  }
  if ($('#btn-request-get-started')) {
    $('#btn-request-get-started').addEventListener('click', startNewRequest);
  }
  if ($('#btn-req-add-header')) {
    $('#btn-req-add-header').addEventListener('click', () => addRequestHeaderRow('', ''));
  }
  if ($('#btn-req-cancel')) {
    $('#btn-req-cancel').addEventListener('click', () => {
      requestEditor.currentId = '';
      $('#request-form').style.display = 'none';
      $('#request-welcome').style.display = '';
      $$('#request-list .target-card-row').forEach((row) => row.classList.remove('active'));
      clearRequestTestResult();
      updateGlobalFireButton();
    });
  }
  $('#btn-req-fire')?.addEventListener('click', () => {
    fireSelectedRequest().catch((err) => toast(err.message, 'error'));
  });
  $('#btn-eng-run')?.addEventListener('click', () => {
    fireSelectedEngagementScenario().catch((err) => toast(err.message, 'error'));
  });
  // ── Bearer-token keychain UI ───────────────────────────────────────
  // Stores a token in the OS credential vault under the env-var name
  // shown in #req-auth-token-env. The runner resolves the env var
  // first and falls back to the keychain — see secrets.rs::resolve_token.
  // Plaintext crosses the JS↔Rust bridge exactly once on save and is
  // never read back into the UI.

  let authTokenStatusTimer = null;
  function scheduleAuthTokenStatusRefresh() {
    clearTimeout(authTokenStatusTimer);
    authTokenStatusTimer = setTimeout(() => {
      refreshRequestAuthTokenStatus().catch((err) => {
        console.error('refreshRequestAuthTokenStatus', err);
      });
    }, 200);
  }

  async function refreshRequestAuthTokenStatus() {
    const pill = $('#req-auth-token-status');
    const setBtn = $('#btn-req-auth-token-set');
    const forgetBtn = $('#btn-req-auth-token-forget');
    if (!pill || !setBtn || !forgetBtn) return;

    const varName = ($('#req-auth-token-env').value || '').trim();
    if (!varName) {
      pill.dataset.state = 'empty';
      pill.textContent = 'Enter an env var name above';
      setBtn.disabled = true;
      forgetBtn.style.display = 'none';
      return;
    }
    setBtn.disabled = false;

    try {
      const status = await API.call('bearer_token_status', { var: varName });
      if (!status.keychain_available) {
        pill.dataset.state = 'unavailable';
        pill.textContent = 'OS keychain unavailable — env var only';
        setBtn.disabled = true;
        forgetBtn.style.display = 'none';
        return;
      }
      forgetBtn.style.display = status.stored_in_keychain ? '' : 'none';
      if (status.env_var_set && status.stored_in_keychain) {
        pill.dataset.state = 'env';
        pill.textContent = 'env var set (wins) · keychain entry exists';
      } else if (status.env_var_set) {
        pill.dataset.state = 'env';
        pill.textContent = 'env var set';
      } else if (status.stored_in_keychain) {
        pill.dataset.state = 'keychain';
        pill.textContent = 'stored in keychain';
      } else {
        pill.dataset.state = 'missing';
        pill.textContent = 'not set';
      }
    } catch (err) {
      pill.dataset.state = 'error';
      pill.textContent = `status error: ${err.message || err}`;
    }
  }

  function openRequestAuthTokenModal() {
    const varName = ($('#req-auth-token-env').value || '').trim();
    if (!varName) {
      toast('Enter an env var name first.', 'error');
      return;
    }
    $('#req-auth-token-env-display').value = varName;
    $('#req-auth-token-input').value = '';
    $('#req-auth-token-input').type = 'password';
    $('#req-auth-token-reveal').checked = false;
    setHidden($('#req-auth-token-modal'), false);
    setTimeout(() => $('#req-auth-token-input').focus(), 0);
  }

  function closeRequestAuthTokenModal() {
    $('#req-auth-token-input').value = '';
    setHidden($('#req-auth-token-modal'), true);
  }

  async function saveRequestAuthTokenFromModal() {
    const varName = ($('#req-auth-token-env-display').value || '').trim();
    const token = $('#req-auth-token-input').value;
    if (!varName) { toast('No env var name available.', 'error'); return; }
    if (!token) { toast('Token must not be empty.', 'error'); return; }
    try {
      await API.call('set_bearer_token', { var: varName, token });
      closeRequestAuthTokenModal();
      toast(`Token stored in keychain for ${varName}.`, 'success');
      await refreshRequestAuthTokenStatus();
    } catch (err) {
      toast(err.message || String(err), 'error');
    }
  }

  async function forgetRequestAuthToken() {
    const varName = ($('#req-auth-token-env').value || '').trim();
    if (!varName) return;
    if (!confirm(`Remove the keychain entry for ${varName}?`)) return;
    try {
      await API.call('forget_bearer_token', { var: varName });
      toast(`Token removed for ${varName}.`, 'success');
      await refreshRequestAuthTokenStatus();
    } catch (err) {
      toast(err.message || String(err), 'error');
    }
  }

  if ($('#btn-req-auth-token-set')) {
    $('#btn-req-auth-token-set').addEventListener('click', openRequestAuthTokenModal);
  }
  if ($('#btn-req-auth-token-forget')) {
    $('#btn-req-auth-token-forget').addEventListener('click', forgetRequestAuthToken);
  }
  if ($('#req-auth-token-env')) {
    $('#req-auth-token-env').addEventListener('input', scheduleAuthTokenStatusRefresh);
  }
  if ($('#req-auth-token-modal-close')) {
    $('#req-auth-token-modal-close').addEventListener('click', closeRequestAuthTokenModal);
  }
  if ($('#req-auth-token-cancel')) {
    $('#req-auth-token-cancel').addEventListener('click', closeRequestAuthTokenModal);
  }
  if ($('#req-auth-token-save')) {
    $('#req-auth-token-save').addEventListener('click', saveRequestAuthTokenFromModal);
  }
  if ($('#req-auth-token-reveal')) {
    $('#req-auth-token-reveal').addEventListener('change', (e) => {
      $('#req-auth-token-input').type = e.target.checked ? 'text' : 'password';
    });
  }
  if ($('#req-auth-token-input')) {
    $('#req-auth-token-input').addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        e.preventDefault();
        saveRequestAuthTokenFromModal();
      }
    });
  }
  if ($('#request-form')) {
    $('#request-form').addEventListener('submit', async (ev) => {
      ev.preventDefault();
      try {
        const req = buildRequestFromForm();
        if (!req.id) { toast('Id is required', 'error'); return; }
        if (!req.name) { toast('Name is required', 'error'); return; }
        if (!/^[a-z0-9][a-z0-9-]*$/.test(req.id)) {
          toast('Id must be kebab-case (lowercase letters, digits, hyphens)', 'error');
          return;
        }
        await API.call('save_request_global', { request: req });
        toast(`Saved request '${req.name}'`, 'success');
        requestEditor.currentId = req.id;
        updateGlobalFireButton();
        await loadRequestList(req.id);
      } catch (err) {
        toast(err.message, 'error');
      }
    });
  }

  // Delete flow with references confirmation.
  async function attemptDeleteRequest(id) {
    const deletingCurrent = requestEditor.currentId === id;
    let res;
    try {
      res = await API.call('delete_request_global', { id, force: false });
    } catch (err) {
      toast(err.message, 'error');
      return;
    }
    if (!res.blocked) {
      toast('Request deleted', 'success');
      if (deletingCurrent) {
        requestEditor.currentId = '';
        $('#request-form').style.display = 'none';
        $('#request-welcome').style.display = '';
        clearRequestTestResult();
        updateGlobalFireButton();
      }
      await loadRequestList();
      return;
    }
    // Show references dialog.
    requestEditor.pendingDeleteId = id;
    const summary = $('#req-delete-summary');
    const refs = $('#req-delete-refs');
    summary.textContent = `This request is referenced by ${res.references.length} item(s):`;
    refs.innerHTML = '';
    res.references.forEach(r => {
      const li = document.createElement('li');
      if (r.kind === 'target') {
        li.textContent = `Target: ${r.name} (${r.id})`;
      } else if (r.kind === 'scenario') {
        li.textContent = `Scenario step: ${r.name} (${r.id}) → ${r.step_id}`;
      } else {
        li.textContent = JSON.stringify(r);
      }
      refs.appendChild(li);
    });
    setHidden($('#req-delete-dialog'), false);
  }

  if ($('#btn-req-delete')) {
    $('#btn-req-delete').addEventListener('click', () => {
      if (requestEditor.currentId) attemptDeleteRequest(requestEditor.currentId);
    });
  }
  if ($('#btn-req-delete-cancel')) {
    $('#btn-req-delete-cancel').addEventListener('click', () => {
      setHidden($('#req-delete-dialog'), true);
      requestEditor.pendingDeleteId = '';
    });
  }
  if ($('#btn-req-delete-confirm')) {
    $('#btn-req-delete-confirm').addEventListener('click', async () => {
      const id = requestEditor.pendingDeleteId;
      if (!id) return;
      try {
        await API.call('delete_request_global', { id, force: true });
        toast('Request deleted (with references cleaned)', 'success');
        setHidden($('#req-delete-dialog'), true);
        requestEditor.pendingDeleteId = '';
        if (requestEditor.currentId === id) {
          requestEditor.currentId = '';
          $('#request-form').style.display = 'none';
          $('#request-welcome').style.display = '';
          clearRequestTestResult();
          updateGlobalFireButton();
        }
        await loadRequestList();
      } catch (err) {
        toast(err.message, 'error');
      }
    });
  }


  // ── Shared prompt filter (used by library + picker) ──────────────
  function applyPromptFilter(prompts, owaspFilter, searchText, categoryFilter = '') {
    let result = prompts;
    if (owaspFilter === 'baseline') {
      result = result.filter(p => (p.tags || []).includes('baseline'));
    } else if (owaspFilter) {
      result = result.filter(p => p.owasp_ref === owaspFilter);
    }
    if (categoryFilter) {
      result = result.filter(p => String(p.category || '') === categoryFilter);
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
  let libraryFilter = { owasp: '', category: '', search: '' };
  let libraryDebounce = null;

  // Last-known full prompt list. Populated by loadPrompts so report
  // helpers like renderEngagementReport can resolve `prompt_id` →
  // `owasp_ref` without an extra round-trip.
  let cachedPrompts = [];

  async function loadPrompts() {
    try {
      const all = await API.call('list_prompts', {});
      cachedPrompts = all;
      renderLibraryCategoryChips(all);
      const prompts = applyPromptFilter(all, libraryFilter.owasp, libraryFilter.search, libraryFilter.category);
      renderPrompts(prompts);
      $('#prompt-count').textContent = `${prompts.length} prompts`;
    } catch (err) { toast(err.message, 'error'); }
  }

  function renderLibraryCategoryChips(prompts) {
    const root = $('#library-category-chips');
    if (!root) return;
    const categories = [...new Set(prompts.map(p => String(p.category || '').trim()))]
      .filter(Boolean)
      .sort((a, b) => a.localeCompare(b));
    if (libraryFilter.category && !categories.includes(libraryFilter.category)) {
      libraryFilter.category = '';
    }
    root.innerHTML = `
      <button class="chip ${libraryFilter.category ? '' : 'active'}" data-category="">ALL</button>
      ${categories.map(cat => `
        <button class="chip ${libraryFilter.category === cat ? 'active' : ''}" data-category="${esc(cat)}">${esc(cat)}</button>
      `).join('')}
    `;
    $$('#library-category-chips .chip').forEach(chip => {
      chip.addEventListener('click', () => {
        $$('#library-category-chips .chip').forEach(c => c.classList.remove('active'));
        chip.classList.add('active');
        libraryFilter.category = chip.dataset.category || '';
        loadPrompts();
      });
    });
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
      row.classList.toggle('selected', editingPromptId === p.id);
      row.dataset.id = p.id;
      const displayName = p.name || p.id;
      row.innerHTML = `
        <div class="meta">
          <span class="prompt-name">${esc(displayName)}</span>
          ${p.owasp_ref ? `<span class="owasp">${esc(p.owasp_ref)}</span>` : ''}
          <span class="owasp">${esc(p.category || '')}</span>
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
      row.addEventListener('click', () => openPromptEditor(p.id));
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
    const idHint = $('#pe-id-hint');
    if (promptId) {
      $$('#library-prompt-list .prompt-row').forEach(row => {
        row.classList.toggle('selected', row.dataset.id === promptId);
      });
      $('#editor-title').textContent = 'Edit Prompt';
      try {
        const found = await API.call('get_prompt', { id: promptId });
        if (!found) { toast('Prompt not found', 'error'); return; }
        const p = found.prompt;
        $('#pe-name').value = p.name || p.id;
        $('#pe-text').value = p.text || '';
        $('#pe-category').value = found.category || '';
        $('#pe-owasp').value = p.owasp_ref || '';
        $('#pe-severity').value = (p.severity || 'LOW').toUpperCase();
        $('#pe-tags').value = (p.tags || []).join(', ');
        if (idHint) idHint.textContent = `id: ${p.id} (unchanged on save)`;
      } catch (err) { toast(err.message, 'error'); }
    } else {
      $('#editor-title').textContent = 'Add Prompt';
      $('#prompt-form').reset();
      if (idHint) idHint.textContent = 'id is auto-derived from the name when you save.';
    }
  }

  $('#pe-cancel').addEventListener('click', () => {
    $('#prompt-editor').style.display = 'none';
    editingPromptId = null;
  });

  $('#prompt-form').addEventListener('submit', async (e) => {
    e.preventDefault();
    const name = $('#pe-name').value.trim();
    if (!name) { toast('Name is required', 'error'); return; }
    const category = $('#pe-category').value.trim();
    if (!category) { toast('Category is required', 'error'); return; }

    const dto = {
      id: editingPromptId || '',
      name,
      category,
      text: $('#pe-text').value,
      severity: $('#pe-severity').value,
      mode: 'single',
      tags: $('#pe-tags').value.split(',').map(t => t.trim()).filter(Boolean),
      owasp_ref: $('#pe-owasp').value || null,
    };
    try {
      if (editingPromptId) {
        await API.call('update_prompt', dto);
        toast('Prompt updated', 'success');
      } else {
        const created = await API.call('create_prompt', dto);
        toast(`Prompt created (id: ${created.id})`, 'success');
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
      const empty = $('#scenario-list-empty');
      ul.innerHTML = '';
      scenarios.forEach(s => {
        const li = document.createElement('li');
        li.className = 'target-card-row';
        li.dataset.id = s.id;
        if (s.id === currentScenarioId) li.classList.add('active');
        li.innerHTML = `
          <div class="target-card-name">${esc(s.name || s.id)}</div>
          <div class="target-card-url">${esc(s.id)}</div>`;
        li.addEventListener('click', () => openScenario(s.id));
        const copyBtn = document.createElement('button');
        copyBtn.type = 'button';
        copyBtn.className = 'btn-icon btn-row-copy';
        copyBtn.title = 'Copy scenario';
        copyBtn.setAttribute('aria-label', 'Copy scenario');
        copyBtn.innerHTML = ICONS.copy;
        copyBtn.addEventListener('click', (e) => {
          e.stopPropagation();
          copyScenarioFromList(s.id);
        });
        const deleteBtn = document.createElement('button');
        deleteBtn.type = 'button';
        deleteBtn.className = 'btn-icon btn-row-delete';
        deleteBtn.title = 'Delete scenario';
        deleteBtn.setAttribute('aria-label', 'Delete scenario');
        deleteBtn.innerHTML = ICONS.archive;
        deleteBtn.addEventListener('click', (e) => {
          e.stopPropagation();
          deleteScenarioFromUi(s);
        });
        li.appendChild(copyBtn);
        li.appendChild(deleteBtn);
        ul.appendChild(li);
      });
      if (empty) empty.style.display = scenarios.length === 0 ? '' : 'none';
    } catch (err) { toast(err.message, 'error'); }
  }

  async function copyScenarioFromList(id) {
    try {
      const copy = await API.call('copy_scenario', { id });
      currentScenarioId = copy.id;
      toast(`Copied scenario '${copy.name}'`, 'success');
      await loadScenarioList();
      openScenario(copy.id);
    } catch (err) {
      toast(err.message, 'error');
    }
  }

  async function createNewScenario() {
    if (!dbOpen) { toast('Open an engagement first', 'error'); return; }
    try {
      const s = await API.call('create_scenario', { name: 'New Scenario' });
      currentScenarioId = s.id;
      await loadScenarioList();
      openScenario(s.id);
    } catch (err) { toast(err.message, 'error'); }
  }

  $('#btn-new-scenario').addEventListener('click', createNewScenario);
  $('#btn-scenario-get-started')?.addEventListener('click', createNewScenario);
  $('#btn-scenario-go-to-requests')?.addEventListener('click', () => showView('view-requests'));

  async function openScenario(scenarioId) {
    currentScenarioId = scenarioId;
    try {
      const s = await API.call('get_scenario', { id: scenarioId });
      if (!s) return;
      currentScenario = s;

      $('#scenario-empty').style.display = 'none';
      $('#scenario-builder').style.display = '';
      $('#scenario-results').style.display = 'none';

      // Fill header fields
      $('#sc-name').value = s.name;
      $('#sc-repeat').value = s.repeat_count || s.repeat || 1;

      // Matrix-mode state. Read from the scenario YAML's matrix fields.
      currentScenarioMatrix = {
        request_ids: Array.isArray(s.request_ids) ? [...s.request_ids] : [],
        request_repeats: s.request_repeats && typeof s.request_repeats === 'object'
          ? { ...s.request_repeats }
          : {},
        owasp_refs: Array.isArray(s.library?.owasp_refs) ? [...s.library.owasp_refs] : [],
        categories: Array.isArray(s.library?.categories) ? [...s.library.categories] : [],
        shared_session: !!s.shared_session,
        enabled_mutators: Array.isArray(s.mutations?.enabled_mutators)
          ? [...s.mutations.enabled_mutators]
          : [],
        max_variants_per_seed: Number.isFinite(s.mutations?.max_variants_per_seed)
          ? s.mutations.max_variants_per_seed
          : null,
        // Section 1 of docs/ToDo.md: multi-session config.
        session_count: Number.isFinite(s.session_count) && s.session_count > 1
          ? s.session_count
          : 1,
        session_identity_kind: s.session_identity?.kind?.type
          || (typeof s.session_identity?.kind === 'string' ? s.session_identity.kind : null)
          || 'cookie_jar',
        session_header_name:
          s.session_identity?.kind?.header_name || '',
      };
      await renderScenarioMatrixUi();

      // Highlight in sidebar
      $$('#scenario-list li').forEach(li =>
        li.classList.toggle('active', li.dataset.id === scenarioId));
      updateGlobalFireButton();
    } catch (err) { toast(err.message, 'error'); }
  }

  // ── Phase 2 matrix-mode editor helpers ────────────────────────────
  async function renderScenarioMatrixUi() {
    // Lazy-load global Requests + prompt index once per session.
    if (currentScenarioMatrixGlobalRequests.length === 0) {
      try {
        currentScenarioMatrixGlobalRequests = await API.call('list_requests', {});
      } catch (err) {
        console.error('[matrix:list_requests]', err);
        currentScenarioMatrixGlobalRequests = [];
      }
    }
    if (currentScenarioMatrixPromptIndex === null) {
      try {
        currentScenarioMatrixPromptIndex = await API.call('list_prompts', {});
      } catch (err) {
        console.error('[matrix:list_prompts]', err);
        currentScenarioMatrixPromptIndex = [];
      }
    }
    renderScenarioMatrixRequests();
    renderScenarioMatrixOwasp();
    renderScenarioMatrixCategories();
    await renderScenarioMatrixMutations();
    $('#sc-matrix-shared-session').checked = !!currentScenarioMatrix.shared_session;
    renderScenarioMatrixMultiSession();
    updateMatrixPromptCounter();
  }

  function buildSessionIdentityPayload(kind, headerName) {
    if (kind === 'cookie_jar') {
      return { kind: { type: 'cookie_jar' } };
    }
    const name = (headerName || '').trim();
    if (kind === 'conversation_header') {
      return {
        kind: {
          type: 'conversation_header',
          header_name: name || 'X-Conversation-Id',
        },
      };
    }
    if (kind === 'custom_header') {
      return {
        kind: {
          type: 'custom_header',
          header_name: name || 'X-Session',
        },
      };
    }
    return { kind: { type: 'cookie_jar' } };
  }

  // ── Section 1 (multi-session) Scenario editor controls ──────────────
  function renderScenarioMatrixMultiSession() {
    const countInput = $('#sc-matrix-session-count');
    const kindSelect = $('#sc-matrix-session-identity-kind');
    const headerInput = $('#sc-matrix-session-header-name');
    const hint = $('#sc-matrix-session-hint');
    if (!countInput) return;
    countInput.value = currentScenarioMatrix.session_count || 1;
    kindSelect.value = currentScenarioMatrix.session_identity_kind || 'cookie_jar';
    headerInput.value = currentScenarioMatrix.session_header_name || '';
    updateMultiSessionVisibility();
    updateMultiSessionHint();
  }

  function updateMultiSessionVisibility() {
    const count = Math.max(1, parseInt($('#sc-matrix-session-count')?.value, 10) || 1);
    const kindSelect = $('#sc-matrix-session-identity-kind');
    const headerInput = $('#sc-matrix-session-header-name');
    if (!kindSelect || !headerInput) return;
    if (count > 1) {
      kindSelect.style.display = '';
      const kind = kindSelect.value;
      headerInput.style.display =
        kind === 'conversation_header' || kind === 'custom_header' ? '' : 'none';
    } else {
      kindSelect.style.display = 'none';
      headerInput.style.display = 'none';
    }
  }

  function updateMultiSessionHint() {
    const out = $('#sc-matrix-session-hint');
    if (!out) return;
    const count = Math.max(1, parseInt($('#sc-matrix-session-count')?.value, 10) || 1);
    if (count <= 1) {
      out.textContent =
        'Single-session run. Set >1 to fire plant/probe phases across parallel ' +
        'sessions and scan for cross-session canary leaks (OWASP LLM02).';
      return;
    }
    out.textContent =
      `${count} sessions will fire in parallel. Plants run first (every session in ` +
      'parallel), then probes. The post-run scanner flags any canary that surfaces ' +
      'in a different session than the one that planted it.';
  }

  // ── 2.10 Mutation panel ─────────────────────────────────────────────
  let mutatorRegistryCache = null;

  async function loadMutatorRegistry() {
    if (mutatorRegistryCache) return mutatorRegistryCache;
    try {
      mutatorRegistryCache = await API.call('list_mutators');
    } catch (err) {
      console.error('[matrix:list_mutators]', err);
      mutatorRegistryCache = [];
    }
    return mutatorRegistryCache;
  }

  async function renderScenarioMatrixMutations() {
    const root = $('#sc-matrix-mutations');
    if (!root) return;
    const registry = await loadMutatorRegistry();
    const enabled = new Set(currentScenarioMatrix.enabled_mutators || []);

    const families = new Map();
    registry.forEach((m) => {
      if (!families.has(m.family)) families.set(m.family, []);
      families.get(m.family).push(m);
    });

    root.innerHTML = '';
    families.forEach((mutators, family) => {
      const section = document.createElement('div');
      section.className = 'mutation-family';
      const heading = document.createElement('div');
      heading.className = 'mutation-family-heading';
      heading.textContent = family;
      section.appendChild(heading);
      mutators.forEach((m) => {
        const label = document.createElement('label');
        label.className = 'mutation-toggle';
        label.innerHTML = `
          <input type="checkbox" data-mutator-id="${esc(m.id)}" ${enabled.has(m.id) ? 'checked' : ''}>
          <span>${esc(m.id)}</span>
        `;
        const cb = label.querySelector('input');
        cb.addEventListener('change', (e) => {
          if (e.target.checked) {
            if (!currentScenarioMatrix.enabled_mutators.includes(m.id)) {
              currentScenarioMatrix.enabled_mutators.push(m.id);
            }
          } else {
            currentScenarioMatrix.enabled_mutators =
              currentScenarioMatrix.enabled_mutators.filter((x) => x !== m.id);
          }
          updateMutationCounter();
        });
        section.appendChild(label);
      });
      root.appendChild(section);
    });

    const cap = $('#sc-matrix-mutations-cap');
    if (cap) {
      cap.value = currentScenarioMatrix.max_variants_per_seed != null
        ? currentScenarioMatrix.max_variants_per_seed
        : '';
    }
    updateMutationCounter();
  }

  function updateMutationCounter() {
    const out = $('#sc-matrix-mutations-counter');
    if (!out) return;
    const count = (currentScenarioMatrix.enabled_mutators || []).length;
    if (count === 0) {
      out.textContent = 'Mutations disabled. Each seed prompt fires once.';
      return;
    }
    const cap = currentScenarioMatrix.max_variants_per_seed;
    const variantsPerSeed = cap != null ? Math.min(count, cap) : count;
    out.textContent =
      `${count} mutator(s) enabled · ` +
      `up to ${variantsPerSeed} extra variant(s) per seed ` +
      `(seed itself always fires).`;
    updateMatrixPromptCounter();
  }

  function renderScenarioMatrixRequests() {
    const root = $('#sc-matrix-requests');
    if (!root) return;
    root.innerHTML = '';
    const all = currentScenarioMatrixGlobalRequests || [];
    if (all.length === 0) {
      root.innerHTML =
        '<div class="muted" style="padding:14px;font-size:12px;">' +
        'No Requests defined yet. Build them in the Requests view.' +
        '</div>';
      return;
    }
    const checked = new Set(currentScenarioMatrix.request_ids);
    all.forEach((req) => {
      const isChecked = checked.has(req.id);
      const localRepeat = currentScenarioMatrix.request_repeats[req.id] || 1;
      const row = document.createElement('div');
      row.className = 'target-request-pick-row' + (isChecked ? ' active' : '');
      row.innerHTML = `
        <label class="pick-checkbox">
          <input type="checkbox" data-req-id="${esc(req.id)}" ${isChecked ? 'checked' : ''}>
          <span class="pick-name">${esc(req.name || req.id)}</span>
          <span class="pick-meta">${esc((req.method || 'POST') + ' · ' + (req.url || '').replace(/^https?:\/\//, ''))}</span>
        </label>
        <span class="pick-repeat" ${isChecked ? '' : 'style="display:none;"'}>
          <label class="pick-repeat-label" for="repeat-${esc(req.id)}">×</label>
          <input type="number" id="repeat-${esc(req.id)}" class="pick-repeat-input"
            min="1" value="${localRepeat}" data-req-id="${esc(req.id)}"
            title="Per-request repeat — multiplies the global Repeat count">
        </span>
      `;
      const checkbox = row.querySelector('input[type=checkbox]');
      const repeatSpan = row.querySelector('.pick-repeat');
      const repeatInput = row.querySelector('.pick-repeat-input');

      checkbox.addEventListener('change', (e) => {
        if (e.target.checked) {
          if (!currentScenarioMatrix.request_ids.includes(req.id)) {
            currentScenarioMatrix.request_ids.push(req.id);
          }
          repeatSpan.style.display = '';
        } else {
          currentScenarioMatrix.request_ids = currentScenarioMatrix.request_ids
            .filter((id) => id !== req.id);
          delete currentScenarioMatrix.request_repeats[req.id];
          repeatInput.value = 1;
          repeatSpan.style.display = 'none';
        }
        row.classList.toggle('active', e.target.checked);
        updateMatrixPromptCounter();
      });

      repeatInput.addEventListener('input', () => {
        const v = Math.max(1, parseInt(repeatInput.value, 10) || 1);
        if (v <= 1) {
          delete currentScenarioMatrix.request_repeats[req.id];
        } else {
          currentScenarioMatrix.request_repeats[req.id] = v;
        }
        updateMatrixPromptCounter();
      });

      root.appendChild(row);
    });
  }

  function renderScenarioMatrixOwasp() {
    const root = $('#sc-matrix-owasp');
    if (!root) return;
    root.innerHTML = '';
    const refs = ['A01', 'A02', 'A03', 'A04', 'A05', 'A06', 'A07', 'A08', 'A09', 'A10'];
    refs.forEach((ref) => {
      const chip = document.createElement('button');
      chip.type = 'button';
      const active = currentScenarioMatrix.owasp_refs.includes(ref);
      chip.className = 'chip' + (active ? ' active' : '');
      chip.dataset.owasp = ref;
      chip.textContent = ref;
      chip.addEventListener('click', () => {
        if (currentScenarioMatrix.owasp_refs.includes(ref)) {
          currentScenarioMatrix.owasp_refs = currentScenarioMatrix.owasp_refs
            .filter((r) => r !== ref);
        } else {
          currentScenarioMatrix.owasp_refs.push(ref);
        }
        chip.classList.toggle('active');
        updateMatrixPromptCounter();
      });
      root.appendChild(chip);
    });
  }

  function renderScenarioMatrixCategories() {
    const root = $('#sc-matrix-categories');
    if (!root) return;
    root.innerHTML = '';
    const prompts = currentScenarioMatrixPromptIndex || [];
    const categories = [...new Set(prompts.map((p) => String(p.category || '')))].filter(Boolean).sort();
    if (categories.length === 0) {
      root.innerHTML = '<span class="muted" style="font-size:11px;">No prompt categories on disk.</span>';
      return;
    }
    categories.forEach((cat) => {
      const chip = document.createElement('button');
      chip.type = 'button';
      const active = currentScenarioMatrix.categories.includes(cat);
      chip.className = 'chip' + (active ? ' active' : '');
      chip.dataset.category = cat;
      chip.textContent = cat;
      chip.addEventListener('click', () => {
        if (currentScenarioMatrix.categories.includes(cat)) {
          currentScenarioMatrix.categories = currentScenarioMatrix.categories
            .filter((c) => c !== cat);
        } else {
          currentScenarioMatrix.categories.push(cat);
        }
        chip.classList.toggle('active');
        updateMatrixPromptCounter();
      });
      root.appendChild(chip);
    });
  }

  function updateMatrixPromptCounter() {
    const out = $('#sc-matrix-prompt-counter');
    if (!out) return;
    const prompts = currentScenarioMatrixPromptIndex || [];
    const matched = prompts.filter((p) => {
      const owasp = String(p.owasp_ref || '');
      const cat = String(p.category || '');
      return (
        currentScenarioMatrix.owasp_refs.includes(owasp) ||
        currentScenarioMatrix.categories.includes(cat)
      );
    }).length;
    // Sum per-request repeats so "chat×3" is counted as 3 firings.
    const requestRepeatSum = currentScenarioMatrix.request_ids
      .reduce((sum, id) => sum + (currentScenarioMatrix.request_repeats[id] || 1), 0);
    const repeat = Math.max(1, parseInt($('#sc-repeat')?.value, 10) || 1);
    const mutatorCount = (currentScenarioMatrix.enabled_mutators || []).length;
    const cap = currentScenarioMatrix.max_variants_per_seed;
    const extraVariants = mutatorCount === 0
      ? 0
      : (cap != null ? Math.min(mutatorCount, cap) : mutatorCount);
    const variantsPerSeed = 1 + extraVariants;
    const total = matched * variantsPerSeed * Math.max(1, requestRepeatSum) * repeat;
    const mutationNote = extraVariants > 0
      ? ` × (1 seed + ${extraVariants} mutation(s))`
      : '';
    out.textContent = `${matched} prompts${mutationNote} × ${requestRepeatSum} request firing(s) × ${repeat} repeat = ${total} attempts (plus auth-chain prereqs).`;
  }

  function estimateCurrentScenarioTotal() {
    const prompts = currentScenarioMatrixPromptIndex || [];
    const matched = prompts.filter((p) => {
      const owasp = String(p.owasp_ref || '');
      const cat = String(p.category || '');
      return (
        currentScenarioMatrix.owasp_refs.includes(owasp) ||
        currentScenarioMatrix.categories.includes(cat)
      );
    }).length;
    const requestRepeatSum = currentScenarioMatrix.request_ids
      .reduce((sum, id) => sum + (currentScenarioMatrix.request_repeats[id] || 1), 0);
    const repeat = Math.max(1, parseInt($('#sc-repeat')?.value, 10) || 1);
    return matched * Math.max(1, requestRepeatSum) * repeat;
  }

  // Listen on the shared-session checkbox.
  document.addEventListener('change', (e) => {
    if (e.target && e.target.id === 'sc-matrix-shared-session') {
      currentScenarioMatrix.shared_session = !!e.target.checked;
    }
  });

  // Mutation cap input.
  document.addEventListener('input', (e) => {
    if (e.target && e.target.id === 'sc-matrix-mutations-cap') {
      const v = Number(e.target.value);
      currentScenarioMatrix.max_variants_per_seed =
        Number.isFinite(v) && v > 0 ? Math.floor(v) : null;
      updateMutationCounter();
    }
    if (e.target && e.target.id === 'sc-matrix-session-count') {
      const v = Math.max(1, parseInt(e.target.value, 10) || 1);
      currentScenarioMatrix.session_count = v;
      updateMultiSessionVisibility();
      updateMultiSessionHint();
    }
    if (e.target && e.target.id === 'sc-matrix-session-header-name') {
      currentScenarioMatrix.session_header_name = e.target.value.trim();
    }
  });

  document.addEventListener('change', (e) => {
    if (e.target && e.target.id === 'sc-matrix-session-identity-kind') {
      currentScenarioMatrix.session_identity_kind = e.target.value;
      updateMultiSessionVisibility();
    }
  });

  $('#sc-repeat')?.addEventListener('input', updateMatrixPromptCounter);

  // ── Save scenario header ───────────────────────────────────────────
  $('#btn-save-scenario').addEventListener('click', async () => {
    if (!currentScenarioId) return;

    const data = {
      id: currentScenarioId,
      name: $('#sc-name').value.trim() || 'Untitled',
      repeat_count: parseInt($('#sc-repeat').value) || 1,
      // Matrix fields fed straight to the Scenario YAML.
      request_ids: [...currentScenarioMatrix.request_ids],
      request_repeats: { ...currentScenarioMatrix.request_repeats },
      library: {
        owasp_refs: [...currentScenarioMatrix.owasp_refs],
        categories: [...currentScenarioMatrix.categories],
      },
      shared_session: !!currentScenarioMatrix.shared_session,
      mutations: (currentScenarioMatrix.enabled_mutators || []).length === 0
        ? null
        : {
            enabled_mutators: [...currentScenarioMatrix.enabled_mutators],
            max_variants_per_seed: currentScenarioMatrix.max_variants_per_seed,
          },
      session_count: (currentScenarioMatrix.session_count || 1) > 1
        ? currentScenarioMatrix.session_count
        : null,
      session_identity: (currentScenarioMatrix.session_count || 1) > 1
        ? buildSessionIdentityPayload(
            currentScenarioMatrix.session_identity_kind || 'cookie_jar',
            currentScenarioMatrix.session_header_name || ''
          )
        : null,
    };
    try {
      await API.call('update_scenario', data);
      toast('Scenario saved', 'success');
      loadScenarioList();
    } catch (err) { toast(err.message, 'error'); }
  });

  // ── Delete scenario ────────────────────────────────────────────────
  async function deleteScenarioFromUi(scenario) {
    const id = typeof scenario === 'string' ? scenario : scenario?.id;
    if (!id) return;
    const label = typeof scenario === 'object' ? (scenario.name || scenario.id) : id;
    if (!confirm(`Delete scenario "${label}"?`)) return;
    try {
      await API.call('delete_scenario', { id });
      if (currentScenarioId === id) {
        currentScenarioId = null;
        currentScenario = null;
        $('#scenario-builder').style.display = 'none';
        $('#scenario-empty').style.display = '';
        updateGlobalFireButton();
      }
      toast('Scenario deleted', 'success');
      loadScenarioList();
    } catch (err) { toast(err.message, 'error'); }
  }

  $('#btn-delete-scenario').addEventListener('click', async (e) => {
    e.preventDefault();
    if (!currentScenarioId) return;
    await deleteScenarioFromUi(currentScenario || currentScenarioId);
  });

  // ── Run scenario ───────────────────────────────────────────────────
  async function fireSelectedScenario() {
    if (!currentScenarioId) return;
    if (currentScenarioMatrix.request_ids.length === 0) {
      toast('Pick at least one Request before running.', 'error');
      return;
    }
    if (currentScenarioMatrix.owasp_refs.length === 0
        && currentScenarioMatrix.categories.length === 0) {
      toast('Pick at least one OWASP ref or category before running.', 'error');
      return;
    }

    // Save scenario first
    $('#btn-save-scenario').click();
    await new Promise(r => setTimeout(r, 300)); // brief wait for save

    setTopbarProgress(0, estimateCurrentScenarioTotal(), 'running');

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
      stopProgressPoll();
    }
  }

  $('#btn-topbar-stop')?.addEventListener('click', async () => {
    try {
      if (!currentRunId) {
        toast('No run is active right now', 'info');
        return;
      }
      const result = await API.call('stop_run', { run_id: currentRunId });
      toast(result?.stopped ? 'Stop requested' : 'Run is no longer active', 'info');
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
      const active = btn.dataset.engTab === tab;
      btn.classList.toggle('active', active);
      btn.setAttribute('aria-selected', active ? 'true' : 'false');
    });
    $$('.eng-detail-panel').forEach((panel) => {
      panel.classList.toggle('active', panel.id === `eng-panel-${tab}`);
    });
    const frame = document.getElementById('eng-report-frame');
    if (frame) frame.style.display = tab === 'report' ? '' : 'none';
    if (tab === 'report' && engagementDetail.activeRunId) {
      const results = engagementDetail.resultsByRunId.get(engagementDetail.activeRunId) || [];
      const run = engagementDetail.runs.find((item) => item.id === engagementDetail.activeRunId) || null;
      renderEngagementReport(results, run);
    }
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

  // Look up the Scenario name a run came from by reading the
  // `scenario_id` recorded in the run header (set by matrix dispatcher).
  // Returns '—' for ad-hoc runs (rerun path) or when the source
  // scenario has been deleted.
  function lookupScenarioNameForRun(run) {
    const scenarioId = run?.scenario_id;
    if (!scenarioId) return '—';
    const found = (engagementDetail.scenarios || []).find(s => s.id === scenarioId);
    return found?.name || `(deleted scenario: ${scenarioId})`;
  }

  async function hydrateEngagementDetailCatalogs() {
    if (!engagementDetail.scenarios.length) {
      try { engagementDetail.scenarios = await API.call('list_scenarios', {}); } catch (_err) { engagementDetail.scenarios = []; }
    }
  }

  // Populate the scenario combobox in the engagement detail header.
  // The chosen scenario is held in engagementDetail.activeScenarioId; the
  // binding is persisted to engagement.yaml by start_scenario_run when the
  // user fires the engagement (no separate save command needed).
  function populateEngagementScenarioSelect() {
    const sel = $('#eng-detail-scenario-select');
    if (!sel) return;
    const scenarios = engagementDetail.scenarios || [];
    const currentId = engagementDetail.activeScenarioId || '';
    const opts = ['<option value="">—</option>'];
    let foundCurrent = false;
    scenarios.forEach((s) => {
      if (s.id === currentId) foundCurrent = true;
      opts.push(`<option value="${esc(s.id)}">${esc(s.name || s.id)}</option>`);
    });
    if (currentId && !foundCurrent) {
      opts.push(`<option value="${esc(currentId)}">(deleted scenario: ${esc(currentId)})</option>`);
    }
    sel.innerHTML = opts.join('');
    sel.value = currentId;
  }

  $('#eng-detail-scenario-select')?.addEventListener('change', (ev) => {
    engagementDetail.activeScenarioId = ev.target.value || null;
    updateEngagementActionButtons();
  });

  $('#btn-eng-detail-copy-endpoint')?.addEventListener('click', async () => {
    const url = $('#eng-detail-target')?.dataset.fullUrl || '';
    if (!url) return;
    try {
      await navigator.clipboard.writeText(url);
      toast('Endpoint URL copied', 'success');
    } catch (_err) {
      toast('Could not copy to clipboard', 'error');
    }
  });

  function resolveTargetFromResults(results) {
    const requestUrl = String(results.find(r => r.request_url)?.request_url || '').trim();
    if (!requestUrl) return { id: null, name: '—', url: '' };

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
      preview.style.display = '';
      engagementDetail.renderedReportSlug = null;
      engagementDetail.renderedReportRunId = null;
      engagementDetail.renderedReportHtml = null;
      const frame = document.getElementById('eng-report-frame');
      if (frame) frame.style.display = 'none';
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

    const owaspByPrompt = new Map(cachedPrompts.map(p => [p.id, p.owasp_ref || null]));
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

    const frame = document.getElementById('eng-report-frame');
    if (!$('#eng-panel-report')?.classList.contains('active')) {
      if (frame) frame.style.display = 'none';
      preview.style.display = '';
      return;
    }

    // If the analyzer has produced an HTML report for this run, replace
    // the textual snapshot with the rendered report. Reports live next
    // to verdicts in the engagement folder; absence is normal.
    if (run?.id && engagementDetail.slug) {
      tryRenderRunReportHtml(engagementDetail.slug, run.id, preview);
    }
  }

  function engagementVerdictBadgeHtml(result) {
    const verdict = String(result?.judge_verdict || '').toUpperCase();
    if (verdict === 'SUCCESS') {
      return '<span class="verdict-badge verdict-success">success</span>';
    }
    if (verdict === 'FAIL') {
      return '<span class="verdict-badge verdict-fail">fail</span>';
    }
    if (verdict === 'PARTIAL') {
      return '<span class="verdict-badge verdict-partial">partial</span>';
    }
    if (verdict === 'UNCLEAR') {
      return '<span class="verdict-badge verdict-pending">unclear</span>';
    }
    return '<span style="color:var(--text-3);">—</span>';
  }

  function resultColumnsForResults(results) {
    const byId = new Map();
    results.forEach((result) => {
      (result.result_columns || result.result_metrics || []).forEach((col) => {
        const id = col.id || metricIdFromLabel(col.label || col.path);
        if (!id || byId.has(id)) return;
        byId.set(id, {
          id,
          label: col.label || id,
          path: col.path || '',
        });
      });
    });
    return [...byId.values()];
  }

  function resultMetricValue(result, column) {
    const metric = (result.result_metrics || []).find((item) => {
      const id = item.id || metricIdFromLabel(item.label || item.path);
      return id === column.id || item.path === column.path;
    });
    return metric?.value ?? '';
  }

  function renderResultsTableHead(columns = []) {
    const metricHeaders = columns
      .map((col) => `<th>${esc(col.label || col.id)}</th>`)
      .join('');
    $('#results-head-row').innerHTML = `
      <th>Step</th><th>Session</th><th>Prompt ID</th><th>Status</th>
      <th>Verdict</th><th>Triage</th>
      ${metricHeaders}
      <th class="col-wide">Request</th>
      <th class="col-wide">Prompt</th><th class="col-wide">Response</th>
      <th>Latency</th>`;
  }

  const TRIAGE_LABELS = {
    unreviewed:     'Unreviewed',
    confirmed:      'Confirmed',
    false_positive: 'False Positive',
    needs_review:   'Needs Review',
  };

  function triageBadgeHtml(status) {
    const label = TRIAGE_LABELS[status] || 'Unreviewed';
    return `<span class="triage-badge triage-${status || 'unreviewed'}">${esc(label)}</span>`;
  }

  let _triageActiveFilter = 'all';

  function applyTriageFilter() {
    const filter = _triageActiveFilter;
    const rows = document.querySelectorAll('#results-tbody tr[data-triage-status]');
    rows.forEach((row) => {
      const status = row.dataset.triageStatus || 'unreviewed';
      const visible = filter === 'all' || status === filter;
      row.style.display = visible ? '' : 'none';
    });
  }

  // Wire filter-bar chip clicks once (delegated from #triage-filter-bar).
  document.getElementById('triage-filter-bar')?.addEventListener('click', (e) => {
    const chip = e.target.closest('[data-triage-filter]');
    if (!chip) return;
    _triageActiveFilter = chip.dataset.triageFilter;
    document.querySelectorAll('.triage-filter-chip').forEach(c =>
      c.classList.toggle('active', c === chip)
    );
    applyTriageFilter();
  });

  async function tryRenderRunReportHtml(engagementSlug, runId, preview) {
    try {
      const html = await API.call('read_report_html', { engagement_slug: engagementSlug, run_id: runId });
      // Race guard: if the user switched runs while the read was in
      // flight, this resolution is for a stale run — drop it rather
      // than overwriting the now-current run's report.
      if (engagementDetail.activeRunId !== runId) return;
      if (engagementDetail.slug !== engagementSlug) return;

      let frame = document.getElementById('eng-report-frame');
      if (!html) {
        engagementDetail.renderedReportSlug = null;
        engagementDetail.renderedReportRunId = null;
        engagementDetail.renderedReportHtml = null;
        // No generated report — keep the textual snapshot, hide any old frame.
        if (frame) frame.style.display = 'none';
        preview.style.display = '';
        return;
      }
      // Render the generated report inside a sandboxed iframe alongside
      // the textual preview, hiding the latter while the report is shown.
      if (!frame) {
        frame = document.createElement('iframe');
        frame.id = 'eng-report-frame';
        frame.className = 'eng-report-preview';
        frame.setAttribute('sandbox', '');
        preview.parentNode.insertBefore(frame, preview);
      }
      const reportChanged = (
        engagementDetail.renderedReportSlug !== engagementSlug
        || engagementDetail.renderedReportRunId !== runId
        || engagementDetail.renderedReportHtml !== html
      );
      if (reportChanged) {
        frame.srcdoc = html;
        engagementDetail.renderedReportSlug = engagementSlug;
        engagementDetail.renderedReportRunId = runId;
        engagementDetail.renderedReportHtml = html;
      }
      frame.style.display = '';
      preview.style.display = 'none';
    } catch (_) {
      // Silently leave the textual preview in place on read errors.
    }
  }

  function updateEngagementHeader(run, results) {
    const target = resolveTargetFromResults(results);
    engagementDetail.targetMatch = target;
    // Prefer the run's recorded scenario_id; fall back to the engagement's
    // persisted target binding so fresh engagements (no runs yet) still
    // expose a default scenario for the ▶ Run button.
    if (run?.scenario_id) {
      engagementDetail.activeScenarioId = run.scenario_id;
    }
    engagementDetail.scenarioName = lookupScenarioNameForRun(run);

    const endCandidates = [...results]
      .map(r => r.received_at || r.sent_at || '')
      .filter(Boolean)
      .sort();
    const endAt = endCandidates[endCandidates.length - 1] || '';

    const endpointEl = $('#eng-detail-target');
    const fullEndpoint = target.url || target.name || '';
    const displayEndpoint = target.name || '—';
    endpointEl.textContent = displayEndpoint;
    endpointEl.title = fullEndpoint || '—';
    endpointEl.dataset.fullUrl = fullEndpoint;
    const copyBtn = $('#btn-eng-detail-copy-endpoint');
    if (copyBtn) copyBtn.style.display = fullEndpoint ? '' : 'none';
    populateEngagementScenarioSelect();
    $('#eng-detail-status').textContent = run?.status || '—';
    $('#eng-detail-start').textContent = formatEngagementDateTime(run?.started_at || '');
    $('#eng-detail-end').textContent = formatEngagementDateTime(endAt);
    updateEngagementActionButtons(run);
  }

  function updateEngagementActionButtons(run = null) {
    const rerunBtn = $('#btn-eng-rerun');
    const stopBtn = $('#btn-eng-stop');
    const selectedRun = run || engagementDetail.runs.find((item) => item.id === engagementDetail.activeRunId) || null;
    const runIsRunning = String(selectedRun?.status || '').toLowerCase() === 'running';

    if (rerunBtn) rerunBtn.disabled = !engagementDetail.slug || !engagementDetail.activeRunId || runIsRunning;
    if (stopBtn) stopBtn.disabled = !engagementDetail.slug || !engagementDetail.activeRunId || !runIsRunning;
    updateGlobalFireButton();
  }

  async function fireSelectedEngagementScenario() {
    if (!engagementDetail.slug) {
      toast('Select an engagement first', 'error');
      return;
    }
    if (!engagementDetail.activeScenarioId) {
      toast('Selected engagement has no scenario run to fire.', 'error');
      return;
    }
    try {
      const runId = await API.call('start_scenario_run', {
        engagement_slug: engagementDetail.slug,
        scenario_id: engagementDetail.activeScenarioId,
      });
      setTopbarProgress(0, 0, 'running');
      toast(`Engagement run started: ${runId}`, 'success');
      await loadRuns({
        engagementSlug: engagementDetail.slug,
        autoSelectFirst: true,
        preferredRunId: runId,
      });
      setEngagementDetailTab('results');
    } catch (err) {
      toast(err.message, 'error');
    }
  }

  $('#btn-global-fire')?.addEventListener('click', async () => {
    const btn = $('#btn-global-fire');
    const action = resolveGlobalFireAction();
    if (!action.enabled) {
      toast(action.title, 'info');
      updateGlobalFireButton();
      return;
    }

    btn.dataset.busy = 'true';
    updateGlobalFireButton();
    try {
      if (action.mode === 'request') {
        await fireSelectedRequest();
      } else if (action.mode === 'scenario') {
        await fireSelectedScenario();
      } else if (action.mode === 'engagement') {
        await fireSelectedEngagementScenario();
      }
    } finally {
      btn.dataset.busy = 'false';
      updateGlobalFireButton();
    }
  });

  function highlightActiveEngagementCard(slug) {
    $$('#engagement-cards .target-card-row').forEach((c) => {
      c.classList.toggle('active', c.dataset.slug === slug);
    });
  }

  function renderRunsViewEmptyState(message) {
    $('#runs-empty').textContent = message;
    $('#runs-empty').style.display = '';
    $('#eng-detail').style.display = 'none';
    updateEngagementActionButtons(null);
  }

  async function openEngagementDetail(eng, { syncRoute = true, selectRunId = null } = {}) {
    unarchiveEngagementSlug(eng.slug);
    const result = await API.call('open_db', { path: eng.slug });
    dbOpen = true;
    onDbOpen(result.name || eng.name, result.slug);

    engagementRunActivity.clear();
    engagementDetail.slug = eng.slug;
    engagementDetail.name = result.name || eng.name;
    engagementDetail.activeRunId = null;
    breadcrumbState.runId = null;
    updateBreadcrumb();
    engagementDetail.resultsByRunId.clear();
    // Seed activeScenarioId from the engagement's persisted target binding
    // (engagement.yaml). loadRunResults later overwrites this from the run
    // header, but for a fresh engagement with no runs yet this is the only
    // way the ▶ Run button knows what to fire.
    try {
      const meta = await API.call('get_engagement', { slug: eng.slug });
      engagementDetail.activeScenarioId = meta?.target?.scenario_id || null;
    } catch (_) {
      engagementDetail.activeScenarioId = null;
    }

    $('#eng-detail-title').textContent = engagementDetail.name;
    $('#eng-detail-slug').textContent = `/engagements/${eng.slug}`;
    setHidden($('#runs-empty'), true);
    setHidden($('#engagements-welcome'), true);
    $('#eng-detail').style.display = '';
    $('#run-results-section').style.display = 'none';
    setEngagementDetailTab('results');
    highlightActiveEngagementCard(eng.slug);
    updateGlobalFireButton();

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
        setHidden($('#runs-empty'), true);
        setHidden($('#eng-detail'), true);
        setHidden($('#engagements-welcome'), false);
        updateEngagementActionButtons(null);
        return;
      }

      // At least one engagement exists — hide the cold-start welcome.
      setHidden($('#engagements-welcome'), true);

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
          </div>
          <button type="button"
                  class="btn-icon btn-eng-delete"
                  title="Delete engagement"
                  aria-label="Delete engagement">${ICONS.archive}</button>`;
        card.addEventListener('click', () => {
          openEngagementDetail(eng, { syncRoute: true }).catch(err => toast(err.message, 'error'));
        });
        const deleteBtn = card.querySelector('.btn-eng-delete');
        if (deleteBtn) {
          deleteBtn.addEventListener('click', (e) => {
            e.stopPropagation();
            deleteEngagementFromUi(eng);
          });
        }
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

  $('#btn-runs-new-engagement').addEventListener('click', openEngagementDialog);
  $('#btn-engagement-get-started')?.addEventListener('click', openEngagementDialog);

  async function deleteEngagementFromUi(eng) {
    if (!eng || !eng.slug) return;
    const label = eng.name || eng.slug;
    const msg =
      `Permanently delete engagement "${label}"?\n\n` +
      `This removes the engagement folder on disk: every run JSONL, every ` +
      `verdict log, every response file, and every generated report. ` +
      `The action cannot be undone.`;
    if (!confirm(msg)) return;

    try {
      const res = await API.call('delete_engagement', { slug: eng.slug });
      if (!res?.deleted) {
        toast(`Engagement "${label}" was already gone on disk.`, 'info');
      } else {
        toast(`Engagement "${label}" deleted.`, 'success');
      }

      // Clear in-memory state for the engagement we just nuked.
      if (engagementDetail.slug === eng.slug) {
        engagementDetail.slug = null;
        engagementDetail.name = '';
        engagementDetail.activeRunId = null;
        engagementDetail.runs = [];
        engagementDetail.resultsByRunId = new Map();
        $('#eng-detail').style.display = 'none';
        clearEngagementRoute({ replace: true });
      }
      if (activeEngagementSlug === eng.slug) {
        activeEngagementSlug = null;
        dbOpen = false;
        breadcrumbState.engagementName = null;
        breadcrumbState.runId = null;
        updateBreadcrumb();
      }
      // Also drop any archive shadow we kept for the row.
      unarchiveEngagementSlug(eng.slug);

      await loadEngagementList({ autoOpen: false });
      await loadHomeRecentEngagements();
    } catch (err) {
      toast(err.message || String(err), 'error');
    }
  }

  API.onProgress((ev) => {
    if (!ev || !ev.run_id) return;
    const runId = ev.run_id;
    const statusText = ev.error ? 'error' : (ev.finished ? 'completed' : 'running');
    const eventTotal = Number(ev.total);
    if (Number.isFinite(eventTotal) && eventTotal > 0) {
      expectedRunTotals.set(runId, eventTotal);
    }
    const displayTotal = expectedRunTotals.get(runId) || (Number.isFinite(eventTotal) ? eventTotal : null);
    setTopbarProgress(
      Number.isFinite(Number(ev.seq)) ? Number(ev.seq) : 0,
      displayTotal || 0,
      ev.error ? 'error' : (ev.finished ? 'done' : 'running'),
    );
    updateTopbarDetailFromProgress({ ...ev, total: displayTotal || ev.total });

    setLiveActivityState(runId, {
      seq: Number.isFinite(Number(ev.seq)) ? Number(ev.seq) : null,
      total: displayTotal,
      status: statusText,
      response: ev.status != null ? String(ev.status) : null,
      error: ev.error || null,
    });

    const row = [...$$('#runs-tbody tr')].find((tr) => tr.dataset.runId === runId);
    if (row) {
      const statusEl = row.querySelector('.run-status-badge');
      const progressEl = row.querySelector('.run-progress-value');
      const errorsEl = row.querySelector('.run-errors-value');
      if (statusEl) {
        const cssState = ev.error ? 'error' : (ev.finished ? 'completed' : 'running');
        statusEl.className = `run-status-badge ${cssState}`;
        statusEl.textContent = cssState;
      }
      if (progressEl) {
        progressEl.textContent = `${ev.seq || 0}/${displayTotal || '?'}`;
      }
      if (errorsEl && ev.error) {
        const nextErr = Number(errorsEl.textContent || '0') + 1;
        errorsEl.textContent = String(nextErr);
        errorsEl.style.color = 'var(--critical)';
      }
    }

    const isRunsViewActive = $('#view-engagements').classList.contains('active');
    updateEngagementActionButtons();
    if (!isRunsViewActive) return;
    if (!engagementDetail.activeRunId || engagementDetail.activeRunId !== runId) return;
    if (!engagementDetail.slug) return;

    const now = Date.now();
    if (now - lastEngagementEventRefreshAt < 700 && !ev.finished && !ev.error) return;
    lastEngagementEventRefreshAt = now;
    loadRunResults(runId, {
      engagementSlug: engagementDetail.slug,
      switchToResultsTab: false,
      suppressErrors: true,
    }).catch(() => {});
  });

  API.onUserRelevantError((ev) => {
    if (!ev || !ev.message) return;
    toast(ev.message, 'error');
  });

  // ── Runs view ──────────────────────────────────────────────────────
  function markActiveRunRow(runId) {
    $$('#runs-tbody tr').forEach((tr) => {
      tr.classList.toggle('active', tr.dataset.runId === runId);
    });
  }

  function setLiveActivityState(runId, patch = {}) {
    if (!runId) return;
    const prev = engagementRunActivity.get(runId) || {};
    engagementRunActivity.set(runId, {
      ...prev,
      ...patch,
      updatedAt: Date.now(),
    });
    renderLiveActivity();
  }

  function renderLiveActivity() {
    const box = $('#eng-live-activity');
    if (!box) return;

    const runId = engagementDetail.activeRunId || (engagementDetail.runs[0]?.id || null);
    if (!runId) {
      box.className = 'eng-live-activity';
      box.textContent = 'No live run activity yet.';
      return;
    }

    const state = engagementRunActivity.get(runId);
    if (!state) {
      box.className = 'eng-live-activity';
      box.textContent = `Run ${runId}: waiting for activity…`;
      return;
    }

    const status = String(state.status || '').toLowerCase();
    const isRunning = status === 'running' || status === 'starting';
    const mode = isRunning ? 'running' : (state.error ? 'error' : 'done');
    box.className = `eng-live-activity ${mode}`;

    const parts = [
      `run ${runId}`,
      state.seq != null ? `attempt ${state.seq}/${state.total || '?'}` : null,
      state.request ? `request ${state.request}` : null,
      state.response ? `response ${state.response}` : null,
      state.error ? `error ${state.error}` : null,
      state.status ? `status ${state.status}` : null,
      state.latency ? `latency ${state.latency}` : null,
      state.updatedAt ? `updated ${new Date(state.updatedAt).toLocaleTimeString()}` : null,
    ].filter(Boolean);
    box.textContent = parts.join('  |  ');
  }

  function stopEngagementProgressPoll() {
    if (engagementProgressPollTimer) {
      clearInterval(engagementProgressPollTimer);
      engagementProgressPollTimer = null;
    }
  }

  function stopEngagementResultsPoll() {
    if (engagementResultsPollTimer) {
      clearInterval(engagementResultsPollTimer);
      engagementResultsPollTimer = null;
    }
  }

  function startEngagementResultsPoll(engagementSlug, runId) {
    stopEngagementResultsPoll();
    if (!engagementSlug || !runId) return;

    const run = (engagementDetail.runs || []).find((r) => r.id === runId);
    if (!run || String(run.status || '').toLowerCase() !== 'running') return;

    engagementResultsPollTimer = setInterval(async () => {
      if (!$('#view-engagements').classList.contains('active')) return;
      if (!engagementDetail.slug || engagementDetail.slug !== engagementSlug) {
        stopEngagementResultsPoll();
        return;
      }
      if (engagementDetail.activeRunId !== runId) {
        stopEngagementResultsPoll();
        return;
      }

      try {
        await loadRunResults(runId, {
          engagementSlug,
          switchToResultsTab: false,
          suppressErrors: true,
          silentRefresh: true,
        });

        const updated = (engagementDetail.runs || []).find((r) => r.id === runId);
        if (!updated || String(updated.status || '').toLowerCase() !== 'running') {
          stopEngagementResultsPoll();
        }
      } catch (_err) {
        // Ignore transient refresh failures.
      }
    }, 1000);
  }

  function hasRunningRuns() {
    return (engagementDetail.runs || []).some((r) => String(r.status || '').toLowerCase() === 'running');
  }

  function startEngagementProgressPoll(engagementSlug) {
    stopEngagementProgressPoll();
    if (!engagementSlug) return;
    if (!hasRunningRuns()) return;

    engagementProgressPollTimer = setInterval(async () => {
      if (!engagementDetail.slug || engagementDetail.slug !== engagementSlug) {
        stopEngagementProgressPoll();
        return;
      }
      if (!$('#view-engagements').classList.contains('active')) return;

      try {
        const runningRuns = (engagementDetail.runs || []).filter((r) => String(r.status || '').toLowerCase() === 'running');
        if (runningRuns.length === 0) {
          stopEngagementProgressPoll();
          return;
        }

        let terminalReached = false;
        for (const run of runningRuns) {
          const p = await API.call('get_run_progress', { engagement_slug: engagementSlug, run_id: run.id });
          const knownTotal = expectedRunTotals.get(run.id);
          const pollTotal = Number(p?.total_prompts);
          const displayTotal = knownTotal || (Number.isFinite(pollTotal) && pollTotal > 0 ? pollTotal : null);

          const idx = engagementDetail.runs.findIndex((r) => r.id === run.id);
          if (idx >= 0 && p) {
            engagementDetail.runs[idx] = {
              ...engagementDetail.runs[idx],
              ...p,
              total_prompts: displayTotal || p.total_prompts,
            };
          }

          const row = [...$$('#runs-tbody tr')].find((tr) => tr.dataset.runId === run.id);
          if (row && p) {
            const statusEl = row.querySelector('.run-status-badge');
            const progressEl = row.querySelector('.run-progress-value');
            const errorsEl = row.querySelector('.run-errors-value');

            if (statusEl) {
              const nextStatus = String(p.status || run.status || 'running').toLowerCase();
              statusEl.className = `run-status-badge ${nextStatus}`;
              statusEl.textContent = nextStatus;
              setLiveActivityState(run.id, { status: nextStatus });
            }
            if (progressEl) {
              progressEl.textContent = `${p.completed}/${displayTotal || '?'}`;
            }
            setTopbarProgress(
              Number.isFinite(Number(p.completed)) ? Number(p.completed) : 0,
              displayTotal || 0,
              String(p.status || run.status || '').toLowerCase() === 'running' ? 'running' : 'done',
            );
            if (errorsEl) {
              errorsEl.textContent = String(p.errors ?? 0);
              errorsEl.style.color = Number(p.errors || 0) > 0 ? 'var(--critical)' : 'var(--text-2)';
            }
          }

          if (p && String(p.status || '').toLowerCase() !== 'running') {
            terminalReached = true;
          }
        }

        if (terminalReached) {
          await loadRuns({
            engagementSlug,
            autoSelectFirst: false,
            preferredRunId: engagementDetail.activeRunId || null,
          });
        }
      } catch (_err) {
        // transient polling errors are expected
      }
    }, 1000);
  }

  // ── Analyzer availability for run-level Analyze button ──────────────
  // Cached per loadRuns call so we don't refetch per row. Refreshed on
  // each loadRuns invocation, which happens whenever the runs view is
  // re-rendered (incl. after install completes via checkAnalyzerCta).
  let analyzerAvailability = { state: 'not_installed', installed: false, judge_mode: 'local' };

  async function refreshAnalyzerAvailability() {
    try {
      const [status, settings] = await Promise.all([
        API.call('get_analyzer_status'),
        API.call('get_app_settings'),
      ]);
      analyzerAvailability = {
        ...status,
        judge_mode: settings?.analyzer?.judge_mode || 'local',
        hosted_judge: settings?.analyzer?.hosted_judge || null,
      };
    } catch (_) {
      analyzerAvailability = { state: 'not_installed', installed: false, judge_mode: 'local' };
    }
  }

  function analyzerUnavailableReason(status) {
    if (status?.judge_mode === 'hosted') {
      if (!status?.hosted_judge?.endpoint) return 'Hosted Judge is selected, but no endpoint is configured in Settings.';
      if (!status?.hosted_judge?.deployment) return 'Hosted Judge is selected, but no deployment/model is configured in Settings.';
      if (!status?.hosted_judge?.secret_stored) return 'Hosted Judge is selected, but no API key is stored in Settings.';
      if (status?.hosted_judge?.keychain_available === false) return 'Hosted Judge requires an available OS keychain to store the API key.';
      return null;
    }
    switch (status?.state) {
      case 'installed': return null;
      case 'downloading': return 'Analyzer is downloading — try again after it finishes.';
      case 'broken_install': return 'Analyzer install is broken — repair it in Settings.';
      case 'incompatible_version': return 'Installed analyzer is incompatible — reinstall in Settings.';
      case 'not_installed':
      default: return 'Analyzer not installed — install it in Settings → Analyz0r.';
    }
  }

  // Apply current analyzer availability to every Analyze button already
  // rendered in the runs table. Called when the install/uninstall flow
  // dispatches `analyzer-state-changed`, so the user does not need to
  // leave and re-enter the runs view to see the buttons enable.
  function applyAnalyzerAvailabilityToRows() {
    const reason = analyzerUnavailableReason(analyzerAvailability);
    $$('#runs-tbody .btn-analyze-run').forEach((btn) => {
      // Don't trample a row that is currently mid-analysis; its label
      // and disabled state belong to analyzeRun() until it finishes.
      if (btn.dataset.analyzing === 'true') return;
      btn.disabled = !!reason;
      btn.title = reason || (analyzerAvailability.judge_mode === 'hosted'
        ? 'Run Hosted Judge on this run'
        : 'Run local analyzer on this run');
    });
  }

  window.addEventListener('analyzer-state-changed', async () => {
    await refreshAnalyzerAvailability();
    applyAnalyzerAvailabilityToRows();
  });

  function findAnalyzeBtn(runId) {
    const tr = [...$$('#runs-tbody tr')].find((row) => row.dataset.runId === runId);
    return tr?.querySelector('.btn-analyze-run') || null;
  }

  async function analyzeRun({ engagementSlug, runId, force = false }) {
    const reason = analyzerUnavailableReason(analyzerAvailability);
    if (reason) { toast(reason, 'error'); return; }

    const btn = findAnalyzeBtn(runId);
    const restoreBtn = () => {
      if (!btn) return;
      btn.dataset.analyzing = 'false';
      btn.disabled = false;
      btn.innerHTML = ICONS.analyze;
      btn.setAttribute('aria-label', 'Analyze');
      btn.title = analyzerAvailability.judge_mode === 'hosted'
        ? 'Run Hosted Judge on this run'
        : 'Run local analyzer on this run';
    };
    if (btn) {
      // While analyzing, the button doubles as a Cancel button: enabled
      // (so the user can click it), labelled "Cancel", and tagged so
      // applyAnalyzerAvailabilityToRows() leaves it alone.
      btn.dataset.analyzing = 'true';
      btn.disabled = false;
      btn.innerHTML = ICONS.cancel;
      btn.setAttribute('aria-label', 'Cancel analysis');
      btn.title = 'Cancel this analysis';
    }

    let unlisten = null;
    try {
      unlisten = await window.__TAURI__.event.listen('analysis-progress', async (ev) => {
        const p = ev.payload || {};
        if (p.run_id !== runId) return;
        // Inline progress on the row's button: "Cancel · 3/12".
        if (btn && !p.finished && Number.isFinite(p.total) && p.total > 0) {
          btn.innerHTML = ICONS.cancel;
          btn.title = `Cancel this analysis (${p.processed || 0}/${p.total})`;
          btn.setAttribute('aria-label', `Cancel analysis, ${p.processed || 0} of ${p.total}`);
        }
        if (p.error) {
          toast(`Analysis failed: ${p.error}`, 'error');
        }
        if (p.finished) {
          if (unlisten) { unlisten(); unlisten = null; }
          restoreBtn();
          if (!p.error) {
            try {
              await API.call('generate_report', { engagement_slug: engagementSlug, run_id: runId });
              toast('Analysis complete — report generated.', 'success');
            } catch (err) {
              toast(`Report generation failed: ${err.message}`, 'error');
            }
            // Refresh results + report tab so verdicts/HTML show up.
            await loadRunResults(runId, { engagementSlug, switchToResultsTab: false }).catch(() => {});
            // Refresh the runs list so the green "analyzed" check appears
            // on the row without requiring a manual reload.
            await loadRuns({ engagementSlug }).catch(() => {});
          }
        }
      });
      await API.call('start_analysis', { engagement_slug: engagementSlug, run_id: runId, force });
      toast('Analysis started.', 'info');
    } catch (err) {
      if (unlisten) { unlisten(); }
      restoreBtn();
      toast(`Could not start analysis: ${err.message}`, 'error');
    }
  }

  async function loadRuns({ engagementSlug = activeEngagementSlug, autoSelectFirst = false, preferredRunId = null } = {}) {
    if (!engagementSlug) return;
    try {
      await refreshAnalyzerAvailability();
      const runs = await API.call('list_runs', { engagement_slug: engagementSlug });
      engagementDetail.runs = runs;
      const tbody = $('#runs-tbody');
      tbody.innerHTML = '';

      if (runs.length === 0) {
        stopEngagementProgressPoll();
        stopEngagementResultsPoll();
        engagementRunActivity.clear();
        renderLiveActivity();
        tbody.innerHTML = '<tr><td colspan="6" style="font-family:var(--mono);font-size:11px;color:var(--text-3);text-align:center;padding:20px;">no runs yet — fire a prompt from the workbench</td></tr>';
        $('#run-results-section').style.display = 'none';
        renderEngagementTimeline([]);
        renderEngagementReport([], null);
        renderResultsTableHead([]);
        updateEngagementHeader(null, []);
        return;
      }

      runs.forEach((r) => {
        setLiveActivityState(r.id, {
          status: String(r.status || '').toLowerCase(),
          seq: Number.isFinite(Number(r.completed)) ? Number(r.completed) : null,
          total: expectedRunTotals.get(r.id) || (Number.isFinite(Number(r.total_prompts)) ? Number(r.total_prompts) : null),
        });
        const tr = document.createElement('tr');
        tr.className = 'clickable';
        tr.dataset.runId = r.id;
        const isRunning = String(r.status || '').toLowerCase() === 'running';
        // Icon-only action buttons. Tooltip via title attribute.
        const stopBtnHtml = isRunning
          ? `<button class="btn-icon btn-stop-run" title="Stop this run" aria-label="Stop">${ICONS.stop}</button>`
          : '';
        tr.innerHTML = `
          <td style="font-family:var(--mono);font-size:11px;">${esc(r.id.substring(0, 8))}</td>
          <td><span class="run-status-badge ${esc(r.status)}">${esc(r.status)}</span></td>
          <td class="run-progress-value" style="font-family:var(--mono);font-size:11px;">${r.completed}/${expectedRunTotals.get(r.id) || r.total_prompts || '?'}</td>
          <td class="run-errors-value" style="font-family:var(--mono);font-size:11px;color:${r.errors > 0 ? 'var(--critical)' : 'var(--text-2)'};">${r.errors}</td>
          <td style="font-family:var(--mono);font-size:11px;">${esc(formatRunStarted(r.started_at))}</td>
          <td class="run-actions-cell">
            ${stopBtnHtml}
            <button class="btn-icon btn-rerun-run" title="Re-run with the same payloads"  aria-label="Re-run">${ICONS.rerun}</button>
            ${r.analyzed ? `<span class="run-analyzed-mark" title="This run has been analyzed" aria-label="Analyzed">${ICONS.analyzed}</span>` : ''}
            <button class="btn-icon btn-analyze-run" title="${r.analyzed ? 'Re-analyze this run' : 'Analyze this run'}" aria-label="${r.analyzed ? 'Re-analyze' : 'Analyze'}">${ICONS.analyze}</button>
            <button class="btn-icon btn-export-md-run"  title="Export Markdown report"  aria-label="Export MD">${ICONS.exportMd}</button>
            <button class="btn-icon btn-export-pdf-run" title="Export PDF (via print)"  aria-label="Export PDF">${ICONS.exportPdf}</button>
            <button class="btn-icon btn-delete-run" title="Delete run permanently (removes all files)"  aria-label="Delete">${ICONS.archive}</button>
          </td>`;
        tr.addEventListener('click', (e) => {
          if (e.target.closest('button')) return;
          loadRunResults(r.id, { engagementSlug, switchToResultsTab: false }).catch(err => toast(err.message, 'error'));
        });

        const analyzeBtn = tr.querySelector('.btn-analyze-run');
        const reason = analyzerUnavailableReason(analyzerAvailability);
        if (reason) {
          analyzeBtn.disabled = true;
          analyzeBtn.title = reason;
        }
        analyzeBtn.addEventListener('click', (e) => {
          e.stopPropagation();
          // Mid-flight click is a cancel; otherwise start a new analysis.
          if (analyzeBtn.dataset.analyzing === 'true') {
            API.call('cancel_analysis', { run_id: r.id }).catch((err) => {
              toast(`Cancel failed: ${err.message}`, 'error');
            });
            return;
          }
          if (r.analyzed && !window.confirm('Re-analyze this run? Existing verdicts will be overwritten.')) {
            return;
          }
          analyzeRun({ engagementSlug, runId: r.id });
        });

        // Wire the rest of the row's action buttons.
        tr.querySelector('.btn-rerun-run')?.addEventListener('click', (e) => {
          e.stopPropagation();
          rerunRun(r.id);
        });
        tr.querySelector('.btn-stop-run')?.addEventListener('click', (e) => {
          e.stopPropagation();
          stopRun(r.id);
        });
        tr.querySelector('.btn-export-md-run')?.addEventListener('click', (e) => {
          e.stopPropagation();
          exportRunMd(r.id);
        });
        tr.querySelector('.btn-export-pdf-run')?.addEventListener('click', (e) => {
          e.stopPropagation();
          exportRunPdf(r.id);
        });
        tr.querySelector('.btn-delete-run')?.addEventListener('click', (e) => {
          e.stopPropagation();
          deleteRunFromUi(r.id);
        });

        tbody.appendChild(tr);
      });

      const fallbackRunId = preferredRunId || engagementDetail.activeRunId;
      const chosen = runs.find(r => r.id === fallbackRunId) || (autoSelectFirst ? runs[0] : null);
      if (chosen) {
        await loadRunResults(chosen.id, { engagementSlug, switchToResultsTab: false });
      }
      renderLiveActivity();
      updateEngagementActionButtons();
      startEngagementProgressPoll(engagementSlug);
    } catch (err) {
      toast(err.message, 'error');
    }
  }

  async function loadRunResults(runId, { engagementSlug = activeEngagementSlug, switchToResultsTab = false, suppressErrors = false, silentRefresh = false } = {}) {
    try {
      engagementDetail.activeRunId = runId;
      breadcrumbState.runId = runId;
      updateBreadcrumb();
      markActiveRunRow(runId);
      $('#run-results-section').style.display = '';
      $('#run-results-title').textContent = `Results · ${runId}`;
      const tbody = $('#results-tbody');
      if (!silentRefresh) {
        tbody.innerHTML = '<tr><td colspan="9" style="font-family:var(--mono);font-size:11px;color:var(--text-3);text-align:center;padding:20px;">loading results…</td></tr>';
      }
      if (switchToResultsTab) {
        setEngagementDetailTab('results');
        $('#run-results-section')?.scrollIntoView({ block: 'start', behavior: 'smooth' });
      }

      const [results, diagnostics, triageEntries] = await Promise.all([
        API.call('get_results', { engagement_slug: engagementSlug, run_id: runId }),
        API.call('get_run_diagnostics', { engagement_slug: engagementSlug, run_id: runId }),
        API.call('get_triage', { engagement_slug: engagementSlug, run_id: runId }).catch(() => []),
      ]);
      const triageBySeq = new Map((triageEntries || []).map(e => [Number(e.seq), e]));
      engagementDetail.resultsByRunId.set(runId, results);
      engagementDetail.triageByRunId.set(runId, triageBySeq);
      const runSummary = engagementDetail.runs.find(r => r.id === runId) || null;
      const runIsRunning = String(runSummary?.status || '').toLowerCase() === 'running';
      const resultColumns = resultColumnsForResults(results);
      renderResultsTableHead(resultColumns);
      tbody.innerHTML = '';
      results.forEach((r) => {
        const pending = !r.error_message && (r.status_code == null || Number(r.status_code) === 0);
        const running = runIsRunning && Number(r.status_code || 0) === 0;
        const statusClass = r.error_message && !running ? 'status-error' : 'status-ok';
        const statusText = running ? 'RUNNING' : (r.error_message ? 'ERROR' : (pending ? 'PENDING' : `${r.status_code || '?'}`));
        const reqMethod = String(r.request_method || '').toUpperCase();
        const reqUrl = String(r.request_url || '').trim();
        const requestText = [reqMethod, reqUrl].filter(Boolean).join(' ');
        const metricCells = resultColumns
          .map((col) => `<td>${esc(resultMetricValue(r, col) || '-')}</td>`)
          .join('');
        const triage = triageBySeq.get(Number(r.seq));
        const triageStatus = triage?.status || 'unreviewed';

        const tr = document.createElement('tr');
        tr.className = 'clickable';
        tr.dataset.triageStatus = triageStatus;
        const leakBadge = Array.isArray(r.leaks) && r.leaks.length > 0
          ? `<span class="leak-badge" title="Cross-session canary from ${esc(
              r.leaks.map(l => l.planted_session).join(', ')
            )} surfaced here">leak</span>`
          : '';
        const phaseBadge = r.phase && r.phase !== 'any'
          ? `<span class="phase-badge phase-${esc(r.phase)}" title="phase: ${esc(r.phase)}">${esc(r.phase)}</span>`
          : '';
        tr.innerHTML = `
          <td>${r.step_order || '-'}</td>
          <td>${esc(r.session_label || '-')} ${phaseBadge} ${leakBadge}</td>
          <td><code>${esc(r.prompt_id)}</code></td>
          <td><span class="${statusClass}">${statusText}</span></td>
          <td>${engagementVerdictBadgeHtml(r)}</td>
          <td class="triage-cell"></td>
          ${metricCells}
          <td><div class="cell-text">${esc(requestText || '-')}</div></td>
          <td><div class="cell-text">${esc(r.prompt_text)}</div></td>
          <td><div class="cell-text">${esc(r.response_text || (running ? '(running)' : (pending ? '(pending)' : '')))}</div></td>
          <td>${r.latency_ms != null ? r.latency_ms + 'ms' : '-'}</td>`;

        // Build triage cell: status select + optional note field.
        const triageCell = tr.querySelector('.triage-cell');
        const select = document.createElement('select');
        select.className = `triage-select triage-select-${triageStatus}`;
        [
          ['unreviewed',     'Unreviewed'],
          ['confirmed',      'Confirmed'],
          ['needs_review',   'Needs Review'],
          ['false_positive', 'False Positive'],
        ].forEach(([val, label]) => {
          const opt = document.createElement('option');
          opt.value = val;
          opt.textContent = label;
          if (val === triageStatus) opt.selected = true;
          select.appendChild(opt);
        });

        const noteBtn = document.createElement('button');
        noteBtn.className = 'triage-note-btn';
        noteBtn.title = triage?.note ? `Note: ${triage.note}` : 'Add note';
        noteBtn.textContent = triage?.note ? '📝' : '○';
        noteBtn.setAttribute('aria-label', 'Toggle note');

        const noteField = document.createElement('input');
        noteField.type = 'text';
        noteField.className = 'triage-note-input is-hidden';
        noteField.placeholder = 'Add a note…';
        noteField.value = triage?.note || '';
        noteField.maxLength = 500;

        const saveTriageChange = async (newStatus, newNote) => {
          try {
            const saved = await API.call('set_triage_status', {
              engagement_slug: engagementSlug,
              run_id: runId,
              seq: Number(r.seq),
              status: newStatus,
              note: newNote || null,
            });
            tr.dataset.triageStatus = saved.status;
            select.className = `triage-select triage-select-${saved.status}`;
            noteBtn.title = saved.note ? `Note: ${saved.note}` : 'Add note';
            noteBtn.textContent = saved.note ? '📝' : '○';
            triageBySeq.set(Number(r.seq), saved);
            applyTriageFilter();
          } catch (err) {
            toast(`Triage save failed: ${err.message}`, 'error');
          }
        };

        select.addEventListener('change', (e) => {
          e.stopPropagation();
          saveTriageChange(select.value, noteField.value);
        });
        select.addEventListener('click', (e) => e.stopPropagation());

        noteBtn.addEventListener('click', (e) => {
          e.stopPropagation();
          const hidden = noteField.classList.toggle('is-hidden');
          if (!hidden) noteField.focus();
        });

        noteField.addEventListener('click', (e) => e.stopPropagation());
        noteField.addEventListener('keydown', (e) => {
          if (e.key === 'Enter') {
            e.preventDefault();
            saveTriageChange(select.value, noteField.value);
            noteField.classList.add('is-hidden');
          } else if (e.key === 'Escape') {
            noteField.classList.add('is-hidden');
          }
        });

        triageCell.appendChild(select);
        triageCell.appendChild(noteBtn);
        triageCell.appendChild(noteField);

        tr.addEventListener('click', () => showResultDetail(r));
        tbody.appendChild(tr);
      });
      applyTriageFilter();

      const latest = [...results].sort((a, b) => Number(a.seq || 0) - Number(b.seq || 0)).at(-1);
      if (latest) {
        const reqMethod = String(latest.request_method || '').toUpperCase();
        const reqUrl = String(latest.request_url || '').trim();
        const requestText = [reqMethod, reqUrl].filter(Boolean).join(' ') || null;
        setLiveActivityState(runId, {
          seq: Number.isFinite(Number(latest.seq)) ? Number(latest.seq) : null,
          request: requestText,
          response: latest.error_message ? null : (latest.status_code ? String(latest.status_code) : null),
          error: latest.error_message || null,
          latency: latest.latency_ms != null ? `${latest.latency_ms}ms` : null,
        });
      } else if (diagnostics && diagnostics.request_url) {
        setLiveActivityState(runId, {
          request: diagnostics.request_url,
          status: diagnostics.status || null,
          error: (diagnostics.notes || []).length ? diagnostics.notes[0] : null,
        });
      }

      const live = engagementRunActivity.get(runId) || null;

      if (runIsRunning && results.length === 0) {
        const diagMessage = diagnostics?.notes?.length ? diagnostics.notes.join(' | ') : null;
        const pendingRow = document.createElement('tr');
        pendingRow.innerHTML = `
          <td>${live?.seq != null ? live.seq : '-'}</td>
          <td>-</td>
          <td><code>-</code></td>
          <td><span class="${live?.error ? 'status-error' : 'status-ok'}">${live?.error ? 'ERROR' : 'PENDING'}</span></td>
          <td><span style="color:var(--text-3);">—</span></td>
          <td><span style="color:var(--text-3);">—</span></td>
          <td><div class="cell-text">${esc(live?.request || diagnostics?.request_url || '-')}</div></td>
          <td><div class="cell-text">(attempt in progress)</div></td>
          <td><div class="cell-text">${esc(live?.error || diagMessage || '(waiting for response)')}</div></td>
          <td>${esc(live?.latency || '-')}</td>`;
        tbody.appendChild(pendingRow);
      }

      updateEngagementHeader(runSummary, results);
      renderEngagementTimeline(results);
      renderEngagementReport(results, runSummary);
      updateEngagementActionButtons(runSummary);
      startEngagementResultsPoll(engagementSlug, runId);
    } catch (err) {
      if (!suppressErrors) toast(err.message, 'error');
    }
  }

  function buildMarkdownReport(results, run) {
    const target = engagementDetail.targetMatch?.name || '—';
    const scenario = engagementDetail.scenarioName || '—';
    const status = run?.status || '—';
    const start = formatEngagementDateTime(run?.started_at || '');
    const endCandidates = [...results]
      .map(r => r.received_at || r.sent_at || '')
      .filter(Boolean)
      .sort();
    const end = formatEngagementDateTime(endCandidates[endCandidates.length - 1] || '');
    const progress = `${run?.completed ?? results.length}/${run?.total_prompts ?? results.length}`;
    const errors = run?.errors ?? results.filter(r => !!r.error_message || Number(r.status_code || 0) === 0).length;

    const lines = [
      `# Run Export`,
      ``,
      `- Target: ${target}`,
      `- Scenario: ${scenario}`,
      `- Status: ${status}`,
      `- Start: ${start || '—'}`,
      `- End: ${end || '—'}`,
      ``,
      `## Run Summary`,
      ``,
      `| Run ID | Status | Progress | Errors | Started | Actions |`,
      `| --- | --- | --- | --- | --- | --- |`,
      `| ${run?.id || '—'} | ${status} | ${progress} | ${errors} | ${formatRunStarted(run?.started_at || '') || '—'} | Results |`,
      ``,
      `## Results`,
      ``,
      `| Step | Session | Prompt ID | Status | Triage | Note | Request | Prompt | Response | Latency |`,
      `| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |`,
    ];

    const triageBySeq = engagementDetail.triageByRunId.get(run?.id || '') || new Map();

    results
      .sort((a, b) => Number(a.seq || 0) - Number(b.seq || 0))
      .forEach((r) => {
        const statusText = r.error_message ? 'ERROR' : String(r.status_code || '?');
        const requestText = [String(r.request_method || '').toUpperCase(), String(r.request_url || '').trim()]
          .filter(Boolean)
          .join(' ') || '-';
        const promptText = String(r.prompt_text || '').replace(/\r?\n/g, ' ').replace(/\|/g, '\\|');
        const responseText = String(r.response_text || r.error_message || '').replace(/\r?\n/g, ' ').replace(/\|/g, '\\|');
        const triage = triageBySeq.get(Number(r.seq));
        const triageLabel = TRIAGE_LABELS[triage?.status] || TRIAGE_LABELS['unreviewed'];
        const triageNote = String(triage?.note || '').replace(/\|/g, '\\|');
        lines.push(`| ${r.step_order || '-'} | ${r.session_label || '-'} | ${r.prompt_id || '-'} | ${statusText} | ${triageLabel} | ${triageNote || '-'} | ${requestText.replace(/\|/g, '\\|')} | ${promptText || '-'} | ${responseText || '-'} | ${r.latency_ms != null ? `${r.latency_ms}ms` : '-'} |`);
      });

    return lines.join('\n');
  }

  function buildRunExportHtml(results, run) {
    const target = engagementDetail.targetMatch?.name || '—';
    const scenario = engagementDetail.scenarioName || '—';
    const status = run?.status || '—';
    const start = formatEngagementDateTime(run?.started_at || '') || '—';
    const endCandidates = [...results]
      .map(r => r.received_at || r.sent_at || '')
      .filter(Boolean)
      .sort();
    const end = formatEngagementDateTime(endCandidates[endCandidates.length - 1] || '') || '—';
    const progress = `${run?.completed ?? results.length}/${run?.total_prompts ?? results.length}`;
    const errors = run?.errors ?? results.filter(r => !!r.error_message || Number(r.status_code || 0) === 0).length;

    const summaryRows = `
      <tr>
        <td>${esc(run?.id || '—')}</td>
        <td>${esc(status)}</td>
        <td>${esc(progress)}</td>
        <td>${esc(String(errors))}</td>
        <td>${esc(formatRunStarted(run?.started_at || '') || '—')}</td>
        <td>Results</td>
      </tr>
    `;

    const resultRows = [...results]
      .sort((a, b) => Number(a.seq || 0) - Number(b.seq || 0))
      .map((r) => {
        const statusText = r.error_message ? 'ERROR' : String(r.status_code || '?');
        const requestText = [String(r.request_method || '').toUpperCase(), String(r.request_url || '').trim()]
          .filter(Boolean)
          .join(' ') || '-';
        return `
          <tr>
            <td>${esc(String(r.step_order || '-'))}</td>
            <td>${esc(r.session_label || '-')}</td>
            <td>${esc(r.prompt_id || '-')}</td>
            <td>${esc(statusText)}</td>
            <td>${esc(requestText)}</td>
            <td>${esc(r.prompt_text || '')}</td>
            <td>${esc(r.response_text || r.error_message || '')}</td>
            <td>${esc(r.latency_ms != null ? `${r.latency_ms}ms` : '-')}</td>
          </tr>
        `;
      }).join('');

    return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>${esc(`${engagementDetail.name || 'Engagement'} · ${run?.id || 'run'}`)}</title>
  <style>
    body {
      font-family: "Segoe UI", Arial, sans-serif;
      color: #111827;
      margin: 32px;
      line-height: 1.4;
    }
    h1, h2 {
      margin: 0 0 12px;
    }
    .meta {
      margin: 0 0 24px;
    }
    .meta-row {
      margin: 4px 0;
    }
    table {
      width: 100%;
      border-collapse: collapse;
      margin: 12px 0 24px;
      table-layout: fixed;
    }
    th, td {
      border: 1px solid #d1d5db;
      padding: 8px;
      text-align: left;
      vertical-align: top;
      font-size: 12px;
      word-break: break-word;
    }
    th {
      background: #f3f4f6;
    }
    .mono {
      font-family: "Consolas", "Courier New", monospace;
    }
    @media print {
      body {
        margin: 16px;
      }
    }
  </style>
</head>
<body>
  <h1>Run Export</h1>
  <div class="meta">
    <div class="meta-row"><strong>Target:</strong> ${esc(target)}</div>
    <div class="meta-row"><strong>Scenario:</strong> ${esc(scenario)}</div>
    <div class="meta-row"><strong>Status:</strong> ${esc(status)}</div>
    <div class="meta-row"><strong>Start:</strong> ${esc(start)}</div>
    <div class="meta-row"><strong>End:</strong> ${esc(end)}</div>
  </div>

  <h2>Run Summary</h2>
  <table>
    <thead>
      <tr>
        <th>Run ID</th>
        <th>Status</th>
        <th>Progress</th>
        <th>Errors</th>
        <th>Started</th>
        <th>Actions</th>
      </tr>
    </thead>
    <tbody>
      ${summaryRows}
    </tbody>
  </table>

  <h2>Results</h2>
  <table>
    <thead>
      <tr>
        <th>Step</th>
        <th>Session</th>
        <th>Prompt ID</th>
        <th>Status</th>
        <th>Request</th>
        <th>Prompt</th>
        <th>Response</th>
        <th>Latency</th>
      </tr>
    </thead>
    <tbody>
      ${resultRows}
    </tbody>
  </table>
</body>
</html>`;
  }

  function printRunExport(results, run) {
    const html = buildRunExportHtml(results, run);
    const iframe = document.createElement('iframe');
    iframe.style.position = 'fixed';
    iframe.style.right = '0';
    iframe.style.bottom = '0';
    iframe.style.width = '0';
    iframe.style.height = '0';
    iframe.style.border = '0';
    iframe.setAttribute('aria-hidden', 'true');
    document.body.appendChild(iframe);

    const cleanup = () => {
      setTimeout(() => {
        iframe.remove();
      }, 250);
    };

    const frameWindow = iframe.contentWindow;
    if (!frameWindow) {
      iframe.remove();
      throw new Error('Could not open print preview frame.');
    }

    frameWindow.document.open();
    frameWindow.document.write(html);
    frameWindow.document.close();

    const triggerPrint = () => {
      try {
        frameWindow.focus();
        frameWindow.print();
      } finally {
        cleanup();
      }
    };

    if (frameWindow.document.readyState === 'complete') {
      setTimeout(triggerPrint, 50);
    } else {
      iframe.addEventListener('load', () => setTimeout(triggerPrint, 50), { once: true });
    }
  }

  // ── Per-run actions (used by row buttons) ─────────────────────────
  // Each function takes an explicit runId so it can fire on any row, not
  // just the currently selected one. Most need that run's results to be
  // loaded into engagementDetail.resultsByRunId — they auto-load if missing.

  async function ensureRunResultsLoaded(runId) {
    if (!engagementDetail.slug || !runId) return;
    if (engagementDetail.resultsByRunId.has(runId)) return;
    await loadRunResults(runId, {
      engagementSlug: engagementDetail.slug,
      switchToResultsTab: false,
    });
  }

  async function rerunRun(runId) {
    if (!engagementDetail.slug || !runId) return;
    await ensureRunResultsLoaded(runId);
    const source = engagementDetail.resultsByRunId.get(runId) || [];
    const diagnostics = await API.call('get_run_diagnostics', {
      engagement_slug: engagementDetail.slug,
      run_id: runId,
    }).catch(() => null);
    const targetId = source.find(r => r.request_id)?.request_id
      || diagnostics?.request_id
      || engagementDetail.targetMatch?.id;
    if (!targetId) {
      toast('Re-run requires a known request mapping. Open a run tied to an existing request.', 'error');
      return;
    }
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
    try {
      const newRunId = await API.call('start_run', {
        engagement_slug: engagementDetail.slug,
        request_id: targetId,
        payloads,
        parallelism: 4,
      });
      expectedRunTotals.set(newRunId, payloads.length);
      setTopbarProgress(0, payloads.length, 'running');
      toast(`Re-run started: ${newRunId}`, 'success');
      await loadRuns({
        engagementSlug: engagementDetail.slug,
        autoSelectFirst: true,
        preferredRunId: newRunId,
      });
      setEngagementDetailTab('results');
    } catch (err) {
      toast(err.message, 'error');
    }
  }

  async function stopRun(runId) {
    if (!engagementDetail.slug || !runId) return;
    const selectedRun = (engagementDetail.runs || []).find((run) => run.id === runId);
    if (String(selectedRun?.status || '').toLowerCase() !== 'running') {
      toast('That run is no longer running', 'info');
      return;
    }
    try {
      const result = await API.call('stop_run', {
        engagement_slug: engagementDetail.slug,
        run_id: runId,
      });
      toast(result?.stopped ? `Stop requested for ${runId}` : `${runId} is no longer active`, 'info');
      await loadRuns({
        engagementSlug: engagementDetail.slug,
        autoSelectFirst: false,
        preferredRunId: runId,
      });
    } catch (err) {
      toast(err.message, 'error');
    }
  }

  async function exportRunMd(runId) {
    if (!engagementDetail.slug || !runId) return;
    // Section 7.2: prefer the canonical analyzer-rendered Markdown when
    // the run has been analyzed. Falls back to the client-side raw-results
    // export so unanalyzed runs can still be shared.
    try {
      const analyzerMd = await API.call('read_report_md', {
        engagementSlug: engagementDetail.slug,
        runId,
      });
      if (analyzerMd) {
        const exported = await API.call('save_markdown_export', {
          engagement_slug: engagementDetail.slug,
          run_id: runId,
          markdown: analyzerMd,
        });
        toastAction('Markdown report exported', 'Export öffnen', () => API.call('open_export_path', {
          path: exported.path,
        }), 'success');
        return;
      }
    } catch (err) {
      console.warn('[export-md] canonical analyzer markdown unavailable, falling back:', err);
    }

    await ensureRunResultsLoaded(runId);
    const results = engagementDetail.resultsByRunId.get(runId) || [];
    const run = engagementDetail.runs.find(r => r.id === runId) || null;
    const markdown = buildMarkdownReport(results, run);
    try {
      const exported = await API.call('save_markdown_export', {
        engagement_slug: engagementDetail.slug,
        run_id: runId,
        markdown,
      });
      toastAction('Markdown report exported', 'Export öffnen', () => API.call('open_export_path', {
        path: exported.path,
      }), 'success');
    } catch (err) {
      toast(err.message, 'error');
    }
  }

  async function exportRunPdf(runId) {
    if (!engagementDetail.slug || !runId) return;
    await ensureRunResultsLoaded(runId);
    try {
      const results = engagementDetail.resultsByRunId.get(runId) || [];
      const run = engagementDetail.runs.find(r => r.id === runId) || null;
      printRunExport(results, run);
      toast('Print dialog opened. Choose "Save as PDF" to export.', 'success');
    } catch (err) {
      toast(err.message, 'error');
    }
  }

  async function deleteRunFromUi(runId) {
    if (!engagementDetail.slug || !runId) return;

    // Stronger confirm: this is irreversible.
    const shortId = runId.substring(0, 8);
    const msg = `Permanently delete run ${shortId}?\n\n` +
      `This removes the run JSONL, its verdicts, every response file, and any generated report. ` +
      `The action cannot be undone.`;
    if (!confirm(msg)) return;

    try {
      const res = await API.call('delete_run', {
        engagement_slug: engagementDetail.slug,
        run_id: runId,
      });
      if (engagementDetail.activeRunId === runId) {
        engagementDetail.activeRunId = null;
        engagementDetail.resultsByRunId.delete(runId);
        breadcrumbState.runId = null;
        updateBreadcrumb();
        $('#run-results-section').style.display = 'none';
      }
      await loadRuns({
        engagementSlug: engagementDetail.slug,
        autoSelectFirst: false,
      });
      toast(res?.removed
        ? `Run ${shortId} deleted (${res.removed} entries removed)`
        : `Run ${shortId} had no on-disk artifacts`, 'success');
    } catch (err) {
      toast(err.message, 'error');
    }
  }

  // ── Result detail modal ────────────────────────────────────────────
  const resultDetailSource = {
    prompt: '',
    request: '',
  };

  function renderPromptTemplateString(value, prompt) {
    return String(value == null ? '' : value).replace(/\{\{\s*prompt\s*\}\}/g, prompt);
  }

  function renderPromptTemplateValue(value, prompt) {
    if (typeof value === 'string') return renderPromptTemplateString(value, prompt);
    if (Array.isArray(value)) return value.map(item => renderPromptTemplateValue(item, prompt));
    if (value && typeof value === 'object') {
      return Object.fromEntries(
        Object.entries(value).map(([key, item]) => [key, renderPromptTemplateValue(item, prompt)])
      );
    }
    return value;
  }

  function formatRequestBodyForDetail(requestTemplate, prompt) {
    if (!requestTemplate || !requestTemplate.body) return '';
    const body = requestTemplate.body;
    const format = String(body.format || 'json').toLowerCase();
    const content = body.content;

    if (format === 'json') {
      const rendered = renderPromptTemplateValue(content ?? {}, prompt);
      return JSON.stringify(rendered, null, 2);
    }

    if (format === 'raw' || format === 'text') {
      if (typeof content === 'string') return renderPromptTemplateString(content, prompt);
      return renderPromptTemplateString(JSON.stringify(content ?? ''), prompt);
    }

    const rendered = renderPromptTemplateValue(content ?? {}, prompt);
    return typeof rendered === 'string' ? rendered : JSON.stringify(rendered, null, 2);
  }

  function formatResultRequestDetail(r) {
    const method = String(r.request_method || r.request_template?.method || '').toUpperCase();
    const url = String(r.request_url || r.request_template?.url || '').trim();
    const headers = r.request_headers || {};
    const body = formatRequestBodyForDetail(r.request_template, r.prompt_text || '');
    const lines = [[method, url].filter(Boolean).join(' ')];

    const headerLines = Object.entries(headers)
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([key, value]) => `${key}: ${value}`);
    if (headerLines.length) {
      lines.push('', ...headerLines);
    }

    if (body) {
      lines.push('', body);
    } else if (Number(r.request_body_size || 0) > 0) {
      lines.push('', `(body not available; recorded size ${r.request_body_size} bytes)`);
    }

    return lines.join('\n');
  }

  function setResultDetailSource(mode) {
    const selected = mode === 'request' ? 'request' : 'prompt';
    $$('#detail-source-tabs .tab').forEach(tab => {
      const active = tab.dataset.detailSource === selected;
      tab.classList.toggle('tab-active', active);
      tab.setAttribute('aria-selected', active ? 'true' : 'false');
    });
    $('#detail-source-text').textContent = resultDetailSource[selected] || '';
  }

  function showResultDetail(r) {
    const statusText = r.error_message ? 'ERROR' : (r.status_code || 'N/A');
    const verdictText = String(r.judge_verdict || '').toUpperCase() || '—';
    const confidenceText = r.judge_confidence != null
      ? `${Math.round(Number(r.judge_confidence) * 100)}%`
      : '—';
    const judgeIdentity = parseJudgeIdentity(r.judge_model_used);
    const summaryBits = [
      `step ${r.step_order || '-'}`,
      `session ${r.session_label || '-'}`,
      `status ${statusText}`,
      `verdict ${verdictText}`,
      r.latency_ms != null ? `${r.latency_ms}ms` : 'latency n/a',
    ];

    const metaItems = [
      ['Run ID', r.run_id || '—'],
      ['Result ID', r.result_id || '—'],
      ['Prompt ID', r.prompt_id || '—'],
      ['HTTP Method', r.request_method || '—'],
      ['Request URL', r.request_url || '—'],
      ['Status', String(statusText)],
      ['Judge Verdict', verdictText],
      ['Judge Confidence', confidenceText],
      ['Judge Mode', judgeIdentity.mode],
      ['Judge Provider', judgeIdentity.provider],
      ['Judge Model', judgeIdentity.model],
      ['Latency', r.latency_ms != null ? `${r.latency_ms}ms` : '—'],
      ['Sent At', formatEngagementDateTime(r.sent_at || '')],
      ['Received At', formatEngagementDateTime(r.received_at || '')],
      ['Session', r.session_label || '—'],
      ['Step', r.step_order || '—'],
      ['Error', r.error_message || '—'],
      ['Judge Reason', r.judge_reason || '—'],
    ];

    $('#detail-summary').textContent = summaryBits.join(' · ');
    resultDetailSource.prompt = r.prompt_text || '';
    resultDetailSource.request = formatResultRequestDetail(r);
    setResultDetailSource('prompt');
    $('#detail-response').textContent = r.response_text || '(no response)';
    const metrics = r.result_metrics || [];
    $('#detail-metrics-section').style.display = metrics.length ? '' : 'none';
    $('#detail-metrics').innerHTML = metrics.map((metric) => `
      <div class="detail-meta-item">
        <span class="detail-meta-label">${esc(metric.label || metric.id || metric.path)}</span>
        <div class="detail-meta-value">${esc(metric.value || 'â€”')}</div>
      </div>
    `).join('');
    $('#detail-meta').innerHTML = metaItems.map(([label, value]) => `
      <div class="detail-meta-item">
        <span class="detail-meta-label">${esc(label)}</span>
        <div class="detail-meta-value">${esc(value)}</div>
      </div>
    `).join('');
    // Replay panel: pre-fill textarea with the original prompt and remember
    // the source attempt so the Fire button can call replay_attempt.
    replayContext.runId = r.run_id || null;
    replayContext.seq = Number.isFinite(Number(r.seq)) ? Number(r.seq) : null;
    replayContext.originalPrompt = r.prompt_text || '';
    const ta = $('#replay-prompt');
    if (ta) ta.value = replayContext.originalPrompt;
    const status = $('#replay-status');
    if (status) status.textContent = '—';
    const out = $('#replay-result');
    if (out) { out.hidden = true; out.innerHTML = ''; }
    setHidden($('#result-detail'), false);
  }

  // Replay panel state for the active result-detail modal.
  const replayContext = {
    runId: null,
    seq: null,
    originalPrompt: '',
    activeReplayRunId: null,
  };

  $('#btn-replay-reset')?.addEventListener('click', () => {
    const ta = $('#replay-prompt');
    if (ta) ta.value = replayContext.originalPrompt;
  });

  $('#btn-replay-fire')?.addEventListener('click', async () => {
    if (!replayContext.runId || replayContext.seq == null) {
      toast('No attempt selected to replay', 'error');
      return;
    }
    const ta = $('#replay-prompt');
    const edited = ta ? ta.value : '';
    const override = edited !== replayContext.originalPrompt ? edited : null;
    const btn = $('#btn-replay-fire');
    const status = $('#replay-status');
    const out = $('#replay-result');
    if (btn) btn.disabled = true;
    if (status) status.textContent = 'firing…';
    if (out) { out.hidden = true; out.innerHTML = ''; }
    try {
      const replayRunId = await API.call('replay_attempt', {
        engagement_slug: activeEngagementSlug,
        run_id: replayContext.runId,
        seq: replayContext.seq,
        prompt_override: override,
      });
      replayContext.activeReplayRunId = replayRunId;
      if (status) status.textContent = `running · ${replayRunId}`;
      // Wait for the replay run to finish, then load its attempt + response.
      await pollReplayCompletion(replayRunId);
    } catch (err) {
      if (status) status.textContent = `error: ${err.message}`;
      toast(`Replay failed: ${err.message}`, 'error');
    } finally {
      if (btn) btn.disabled = false;
    }
  });

  async function pollReplayCompletion(replayRunId, attempts = 60) {
    const status = $('#replay-status');
    const out = $('#replay-result');
    for (let i = 0; i < attempts; i += 1) {
      await new Promise((res) => setTimeout(res, 500));
      try {
        const records = await API.call('read_run_attempts', {
          engagement_slug: activeEngagementSlug,
          run_id: replayRunId,
        });
        const attempt = (records || []).find((a) => a && a.seq);
        if (!attempt) continue;
        // We have an attempt; fetch its response body and render.
        const body = await API.call('read_response_body', {
          engagement_slug: activeEngagementSlug,
          run_id: replayRunId,
          seq: attempt.seq,
        }).catch(() => '');
        if (status) status.textContent = `done · ${replayRunId} · status ${attempt.response?.status ?? '—'}`;
        if (out) {
          out.hidden = false;
          out.innerHTML = `
            <div><strong>Status:</strong> ${esc(String(attempt.response?.status ?? '—'))}</div>
            <div><strong>Latency:</strong> ${esc(String(attempt.timing?.duration_ms ?? '—'))} ms</div>
            <pre>${esc(body || '(no response body)')}</pre>`;
        }
        return;
      } catch (_err) {
        // transient — keep polling
      }
    }
    if (status) status.textContent = `timed out waiting for ${replayRunId}`;
  }

  $('#detail-source-tabs').addEventListener('click', (e) => {
    const tab = e.target.closest('.tab[data-detail-source]');
    if (!tab) return;
    setResultDetailSource(tab.dataset.detailSource);
  });

  $('#result-detail-close').addEventListener('click', () => {
    setHidden($('#result-detail'), true);
  });

  async function applyEngagementRouteFromLocation() {
    const slug = getEngagementSlugFromRoute();
    if (!slug) return false;

    if (!$('#view-engagements').classList.contains('active')) {
      showView('view-engagements');
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
          setTopbarProgress(
            run.completed,
            run.total_prompts,
            run.status === 'running' ? 'running' : 'done',
          );
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
  // The Workbench-specific bindings ('/' focus picker, Cmd/Ctrl+Enter to
  // fire) were retired with the Workbench view in Phase 2F of
  // docs/RefactorPlan.md.
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') {
      document.querySelectorAll('.modal').forEach(m => { m.style.display = 'none'; });
      const pickerSearch = $('#picker-search');
      if (pickerSearch && document.activeElement === pickerSearch) {
        pickerSearch.blur();
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

  // ── Settings modal: open + close handlers (outer-scope, robust) ────
  // These run independently of initAnalyzerUI() so the dialog can be
  // opened and closed even if the analyzer init below throws.
  const settingsModal = document.querySelector('#settings-modal');
  const settingsNavBtn = document.querySelector('[data-nav="settings"]');
  const settingsCloseX = document.querySelector('#settings-modal-close');
  const settingsCloseBtn = document.querySelector('#settings-modal-cancel');

  if (settingsNavBtn && settingsModal) {
    settingsNavBtn.addEventListener('click', () => {
      settingsModal.style.display = 'flex';
      // The analyzer section refreshes its variant list off this event.
      // We can't call refreshAnalyzerModal directly because it lives
      // inside initAnalyzerUI's closure (and that init might have
      // thrown — the whole point of this outer handler).
      window.dispatchEvent(new CustomEvent('settings-modal-opened'));
    });
  }
  if (settingsCloseX && settingsModal) {
    settingsCloseX.addEventListener('click', () => {
      settingsModal.style.display = 'none';
    });
  }
  if (settingsCloseBtn && settingsModal) {
    settingsCloseBtn.addEventListener('click', () => {
      settingsModal.style.display = 'none';
    });
  }

  // ── Analyzer activation ────────────────────────────────────────────
  // Wrapped: if init throws, the rest of the app keeps working and the
  // settings modal still opens/closes thanks to the handlers above.
  try {
    initAnalyzerUI();
  } catch (err) {
    console.error('[Settings init] initAnalyzerUI failed:', err);
  }
});

// ============================================================
// Analyzer activation flow
// ============================================================
function initAnalyzerUI() {
  // Hardware class label lookup
  const HW_LABELS = {
    apple_silicon: 'Apple Silicon (Metal)',
    x86_64_avx2:  'x86-64 AVX2 (CPU)',
    generic:       'Generic CPU (slow)',
  };

  let selectedVariantId = null;
  let downloadUnlisten = null;
  let analyzerStatus = null;
  let defaultJudgePromptTemplate = '';
  let currentJudgeMode = 'local';
  let currentSettingsView = 'general';
  let currentAnalyzerSettingsView = 'prompt';
  let hostedSecretStatus = {
    secret_ref: 'HOSTED_JUDGE_API_KEY',
    secret_stored: false,
    keychain_available: true,
  };

  function renderSettingsView(view) {
    currentSettingsView = ['general', 'logging', 'analyzer'].includes(view) ? view : 'general';
    $$('.settings-nav-item').forEach((btn) => {
      const isActive = btn.dataset.settingsView === currentSettingsView;
      btn.classList.toggle('active', isActive);
      btn.setAttribute('aria-current', isActive ? 'page' : 'false');
    });
    $$('.settings-view').forEach((panel) => {
      panel.classList.toggle('active', panel.dataset.settingsView === currentSettingsView);
    });
  }

  function renderAnalyzerSettingsView(view) {
    currentAnalyzerSettingsView = ['prompt', 'local', 'hosted'].includes(view) ? view : 'prompt';
    $$('.settings-subnav-item').forEach((btn) => {
      btn.classList.toggle('active', btn.dataset.analyzerView === currentAnalyzerSettingsView);
    });
    $$('.settings-subview').forEach((panel) => {
      panel.classList.toggle('active', panel.dataset.analyzerView === currentAnalyzerSettingsView);
    });
  }

  function renderJudgeModeSections(mode) {
    currentJudgeMode = mode === 'hosted' ? 'hosted' : 'local';
    const hostedTestBtn = $('#btn-settings-test-hosted-judge');
    if (hostedTestBtn) hostedTestBtn.disabled = currentJudgeMode !== 'hosted';
    const localDisabled = currentJudgeMode !== 'local';
    ['btn-analyzer-install', 'btn-analyzer-repair', 'btn-analyzer-uninstall'].forEach((id) => {
      const btn = $(`#${id}`);
      if (!btn) return;
      if (localDisabled) {
        btn.dataset.disabledByJudgeMode = 'true';
        btn.disabled = true;
        btn.title = 'Local analyzer controls are inactive while Hosted Judge is selected.';
      } else {
        delete btn.dataset.disabledByJudgeMode;
        btn.title = '';
      }
    });
    updateHostedJudgeUiState();
  }

  function updateHostedSecretStatus(hosted) {
    hostedSecretStatus = {
      secret_ref: hosted?.secret_ref || 'HOSTED_JUDGE_API_KEY',
      secret_stored: !!hosted?.secret_stored,
      keychain_available: hosted?.keychain_available !== false,
    };
    updateHostedJudgeUiState();
  }

  function getHostedJudgeValidation() {
    const endpoint = ($('#settings-hosted-endpoint')?.value || '').trim();
    const deployment = ($('#settings-hosted-deployment')?.value || '').trim();
    const secretRef = ($('#settings-hosted-secret-ref')?.value || '').trim() || 'HOSTED_JUDGE_API_KEY';
    const typedApiKey = ($('#settings-hosted-api-key')?.value || '').trim();
    const issues = [];
    if (!endpoint) issues.push('Endpoint is required.');
    if (!deployment) issues.push('Deployment / Model is required.');
    if (!secretRef) issues.push('API key reference is required.');
    if (!hostedSecretStatus.keychain_available) {
      issues.push('OS keychain is not available.');
    }
    if (!hostedSecretStatus.secret_stored && !typedApiKey) {
      issues.push('Store an API key or enter one before saving.');
    }
    return {
      endpoint,
      deployment,
      secretRef,
      typedApiKey,
      issues,
      ready: issues.length === 0,
    };
  }

  function updateHostedJudgeUiState() {
    const el = $('#settings-hosted-secret-status');
    if (!el) return;
    const hostedTestBtn = $('#btn-settings-test-hosted-judge');
    const clearBtn = $('#btn-settings-clear-hosted-secret');
    const validation = getHostedJudgeValidation();
    if (clearBtn) clearBtn.disabled = !hostedSecretStatus.secret_stored;

    if (currentJudgeMode !== 'hosted') {
      el.innerHTML = 'Hosted Judge is currently inactive because Local Judge is selected.';
      if (hostedTestBtn) hostedTestBtn.disabled = true;
      return;
    }

    if (!hostedSecretStatus.keychain_available) {
      el.innerHTML = 'OS keychain is not available. Hosted Judge cannot store or use an API key right now.';
      if (hostedTestBtn) hostedTestBtn.disabled = true;
      return;
    }

    if (validation.issues.length > 0) {
      el.innerHTML = `Hosted Judge is not ready: ${esc(validation.issues.join(' '))}`;
      if (hostedTestBtn) hostedTestBtn.disabled = true;
      return;
    }

    if (hostedSecretStatus.secret_stored) {
      el.innerHTML = `Hosted Judge is ready. API key stored in keychain for <code>${esc(validation.secretRef)}</code>. Leave the password field blank to keep it.`;
    } else {
      el.innerHTML = `Hosted Judge will be ready after saving the API key to <code>${esc(validation.secretRef)}</code>.`;
    }
    if (hostedTestBtn) hostedTestBtn.disabled = false;
  }

  async function refreshHostedSecretStatusFromUi() {
    const secretRef = ($('#settings-hosted-secret-ref')?.value || '').trim() || 'HOSTED_JUDGE_API_KEY';
    try {
      const status = await API.call('secret_ref_status', { secret_ref: secretRef });
      hostedSecretStatus = {
        secret_ref: secretRef,
        secret_stored: !!status?.stored_in_keychain,
        keychain_available: status?.keychain_available !== false,
      };
    } catch (_) {
      hostedSecretStatus = {
        secret_ref: secretRef,
        secret_stored: false,
        keychain_available: false,
      };
    }
    updateHostedJudgeUiState();
  }

  function openSettingsModal() {
    setHidden($('#settings-modal'), false);
    loadAppSettings().catch(err => {
      $('#settings-save-status').textContent = `Failed to load settings: ${err.message}`;
    });
    refreshAnalyzerModal();
  }

  // The sidebar's Settings nav button lives in the outer scope (see
  // the resilience note around `settingsNavBtn`) and only sets display.
  // Listen for its event here so the analyzer section actually
  // populates when the user opens Settings from the sidebar — without
  // it, the static "loading variants…" placeholder never gets replaced.
  window.addEventListener('settings-modal-opened', () => {
    loadAppSettings().catch(err => {
      $('#settings-save-status').textContent = `Failed to load settings: ${err.message}`;
    });
    refreshAnalyzerModal();
  });

  function updateJudgePromptStatus(settings) {
    const status = $('#settings-analyzer-prompt-status');
    if (!status) return;
    if (settings.analyzer?.uses_default_judge_prompt) {
      status.innerHTML = 'Using the built-in default prompt for future Analyze and Judge actions.';
    } else {
      status.innerHTML = 'Using a custom global judge prompt for future Analyze and Judge actions.';
    }
  }

  async function loadAppSettings() {
    const settings = await API.call('get_app_settings');
    $('#settings-app-version').value = settings.app_version || '0.4';
    const theme = normalizeTheme(settings.ui?.theme || 'default');
    $('#settings-theme').value = theme;
    applyTheme(theme);
    $('#settings-logging-enabled').checked = !!settings.logging?.enabled;
    $('#settings-log-level').value = settings.logging?.level || 'info';
    $('#settings-body-logging-enabled').checked = !!settings.logging?.body_logging_enabled;
    $('#settings-analyzer-judge-mode').value = settings.analyzer?.judge_mode || 'local';
    defaultJudgePromptTemplate = settings.analyzer?.default_judge_prompt_template || '';
    $('#settings-analyzer-judge-prompt').value =
      settings.analyzer?.judge_prompt_template || defaultJudgePromptTemplate;
    $('#settings-hosted-provider').value =
      settings.analyzer?.hosted_judge?.provider || 'azure_openai';
    $('#settings-hosted-endpoint').value =
      settings.analyzer?.hosted_judge?.endpoint || '';
    $('#settings-hosted-deployment').value =
      settings.analyzer?.hosted_judge?.deployment || '';
    $('#settings-hosted-api-style').value =
      settings.analyzer?.hosted_judge?.api_style || 'auto';
    $('#settings-hosted-api-version').value =
      settings.analyzer?.hosted_judge?.api_version || '';
    $('#settings-hosted-secret-ref').value =
      settings.analyzer?.hosted_judge?.secret_ref || 'HOSTED_JUDGE_API_KEY';
    $('#settings-hosted-api-key').value = '';
    $('#settings-hosted-max-input-chars').value =
      settings.analyzer?.hosted_judge?.max_input_chars || 24000;
    $('#settings-hosted-max-output-tokens').value =
      settings.analyzer?.hosted_judge?.max_output_tokens || 1200;
    $('#settings-hosted-timeout-seconds').value =
      settings.analyzer?.hosted_judge?.request_timeout_seconds || 60;
    $('#settings-hosted-max-retries').value =
      settings.analyzer?.hosted_judge?.max_retries || 1;
    updateJudgePromptStatus(settings);
    updateHostedSecretStatus(settings.analyzer?.hosted_judge || null);
    renderJudgeModeSections(settings.analyzer?.judge_mode || 'local');
    $('#settings-logging-status').innerHTML =
      'Logging changes apply after restarting the app.';
    $('#settings-save-status').innerHTML =
      'Settings are saved to <code>~/hamm0r/config.yaml</code>.';
  }

  async function refreshAnalyzerModal() {
    // Reset state
    selectedVariantId = null;
    $('#btn-analyzer-install').disabled = true;
    $('#btn-analyzer-repair').style.display = 'none';
    $('#analyzer-download-section').style.display = 'none';
    $('#analyzer-variants').innerHTML = '<div class="analyzer-loading">loading…</div>';

    try {
      analyzerStatus = await API.call('get_analyzer_status');
      const hw = analyzerStatus.hardware;
      $('#analyzer-hw-badge').textContent = HW_LABELS[hw] || hw;
      $('#analyzer-hw-badge').dataset.hw = hw;

      // The five install states are the source of truth from the backend —
      // `installed` is just `state === 'installed'` in disguise.
      const badge = $('#analyzer-install-badge');
      const state = analyzerStatus.state || (analyzerStatus.installed ? 'installed' : 'not_installed');
      badge.dataset.state = state;
      switch (state) {
        case 'installed':
          badge.textContent = `installed: ${analyzerStatus.model_file || analyzerStatus.variant_id || ''}`;
          $('#btn-analyzer-uninstall').style.display = '';
          $('#btn-analyzer-install').textContent = 'Re-download';
          break;
        case 'downloading':
          badge.textContent = `downloading: ${analyzerStatus.downloading_variant_id || ''}`;
          $('#btn-analyzer-uninstall').style.display = 'none';
          $('#btn-analyzer-install').textContent = 'Download & Install';
          break;
        case 'broken_install':
          badge.textContent = analyzerStatus.variant_id
            ? `broken install: ${analyzerStatus.variant_id}`
            : 'broken install';
          $('#btn-analyzer-uninstall').style.display = '';
          $('#btn-analyzer-install').textContent = 'Download & Install';
          // Offer one-click repair using the recorded variant id.
          if (analyzerStatus.variant_id) {
            $('#btn-analyzer-repair').style.display = '';
            $('#btn-analyzer-repair').dataset.variantId = analyzerStatus.variant_id;
          }
          break;
        case 'incompatible_version':
          badge.textContent = 'incompatible install — reinstall required';
          $('#btn-analyzer-uninstall').style.display = '';
          $('#btn-analyzer-install').textContent = 'Download & Install';
          break;
        case 'not_installed':
        default:
          badge.textContent = 'not installed';
          $('#btn-analyzer-uninstall').style.display = 'none';
          $('#btn-analyzer-install').textContent = 'Download & Install';
          break;
      }

      const manifest = await API.call('fetch_analyzer_manifest');
      renderVariants(manifest.variants, hw);
      renderJudgeModeSections(currentJudgeMode);
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
      const sizeGb = (v.bundle.size_bytes / 1e9).toFixed(1);
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
    if (currentJudgeMode === 'local') {
      $('#btn-analyzer-install').disabled = false;
    }
  }

  // ── Install (shared by Install + Repair buttons) ────────────────────
  // Both the variant-picker install and the broken-install repair flow
  // funnel through this so they share progress UI, listener lifecycle,
  // and error handling. Repair just hands in the variant id recorded in
  // install.json instead of one the user clicked.
  async function startInstall(variantId) {
    if (!variantId) return;

    $('#analyzer-download-section').style.display = '';
    $('#btn-analyzer-install').disabled = true;
    $('#btn-analyzer-repair').disabled = true;
    $('#analyzer-progress-bar').style.width = '0%';
    $('#analyzer-progress-text').textContent = 'Starting download…';
    $('#analyzer-progress-pct').textContent = '0%';
    $('#analyzer-progress-bytes').textContent = '';

    if (downloadUnlisten) { downloadUnlisten(); downloadUnlisten = null; }
    downloadUnlisten = await window.__TAURI__.event.listen(
      'analyzer-download-progress',
      (ev) => onDownloadProgress(ev.payload)
    );

    try {
      await API.call('download_and_install_analyzer', { variant_id: variantId });
    } catch (err) {
      toast(`Download failed: ${err.message}`, 'error');
      $('#analyzer-download-section').style.display = 'none';
      $('#btn-analyzer-install').disabled = false;
      $('#btn-analyzer-repair').disabled = false;
    }
  }

  $('#btn-analyzer-install').addEventListener('click', () => {
    startInstall(selectedVariantId);
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
      // Notify other views (runs view) that availability flipped so
      // their per-row Analyze buttons can re-render without waiting
      // for the user to navigate away and back.
      window.dispatchEvent(new CustomEvent('analyzer-state-changed'));
      toast('Analyz0r activated. Judgments will use the local LLM on next analysis run.', 'success');
    } else {
      $('#analyzer-progress-text').textContent = 'Downloading…';
    }
  }

  // ── Repair button ───────────────────────────────────────────────────
  // Repair re-runs install for the variant recorded in install.json.
  // do_install moves the existing layout aside before extracting, so a
  // failure mid-repair rolls back rather than leaving the user worse off.
  $('#btn-analyzer-repair').addEventListener('click', () => {
    startInstall($('#btn-analyzer-repair').dataset.variantId);
  });

  // ── Uninstall button ────────────────────────────────────────────────
  $('#btn-analyzer-uninstall').addEventListener('click', async () => {
    try {
      await API.call('uninstall_analyzer');
      toast('Analyzer model removed.', 'success');
      refreshAnalyzerModal();
      checkAnalyzerCta();
      window.dispatchEvent(new CustomEvent('analyzer-state-changed'));
    } catch (err) {
      toast(`Uninstall failed: ${err.message}`, 'error');
    }
  });

  $('#btn-settings-reset-analyzer-prompt').addEventListener('click', () => {
    $('#settings-analyzer-judge-prompt').value = defaultJudgePromptTemplate;
    $('#settings-analyzer-prompt-status').innerHTML =
      'Reset to the built-in default prompt. Save Settings to persist it.';
  });

  $('#settings-analyzer-judge-mode').addEventListener('change', (event) => {
    renderJudgeModeSections(event.target.value);
    renderAnalyzerSettingsView(event.target.value === 'hosted' ? 'hosted' : 'local');
  });

  $$('.settings-nav-item').forEach((btn) => {
    btn.addEventListener('click', () => {
      renderSettingsView(btn.dataset.settingsView);
    });
  });

  $$('.settings-subnav-item').forEach((btn) => {
    btn.addEventListener('click', () => {
      renderAnalyzerSettingsView(btn.dataset.analyzerView);
    });
  });

  [
    '#settings-hosted-endpoint',
    '#settings-hosted-deployment',
    '#settings-hosted-api-style',
    '#settings-hosted-api-version',
    '#settings-hosted-api-key',
    '#settings-hosted-max-input-chars',
    '#settings-hosted-max-output-tokens',
    '#settings-hosted-timeout-seconds',
    '#settings-hosted-max-retries',
  ].forEach((selector) => {
    $(selector)?.addEventListener('input', () => {
      updateHostedJudgeUiState();
    });
  });

  $('#settings-hosted-secret-ref')?.addEventListener('input', () => {
    hostedSecretStatus = {
      secret_ref: ($('#settings-hosted-secret-ref').value || '').trim() || 'HOSTED_JUDGE_API_KEY',
      secret_stored: false,
      keychain_available: hostedSecretStatus.keychain_available,
    };
    updateHostedJudgeUiState();
  });
  $('#settings-hosted-secret-ref')?.addEventListener('change', () => {
    refreshHostedSecretStatusFromUi().catch(() => {});
  });

  $('#btn-settings-clear-hosted-secret').addEventListener('click', async () => {
    const secretRef = ($('#settings-hosted-secret-ref').value || '').trim() || 'HOSTED_JUDGE_API_KEY';
    try {
      await API.call('forget_secret_ref', { secret_ref: secretRef });
      $('#settings-hosted-api-key').value = '';
      updateHostedSecretStatus({
        secret_ref: secretRef,
        secret_stored: false,
        keychain_available: true,
      });
      toast('Hosted API key cleared from keychain.', 'success');
    } catch (err) {
      toast(`Could not clear hosted API key: ${err.message}`, 'error');
    }
  });

  $('#btn-settings-test-hosted-judge').addEventListener('click', async () => {
    const btn = $('#btn-settings-test-hosted-judge');
    btn.disabled = true;
    try {
      const result = await API.call('test_hosted_judge');
      $('#settings-hosted-secret-status').innerHTML =
        `Hosted Judge test succeeded. Model: <code>${esc(result.model_used)}</code>. Verdict: <code>${esc(result.verdict)}</code>.`;
      toast(`Hosted Judge test passed with ${result.model_used}.`, 'success');
    } catch (err) {
      $('#settings-hosted-secret-status').textContent = `Hosted Judge test failed: ${err.message}`;
      toast(`Hosted Judge test failed: ${err.message}`, 'error');
    } finally {
      btn.disabled = false;
    }
  });

  async function saveSettingsFromModal(triggerButton) {
    const btn = triggerButton || $('#btn-settings-save');
    btn.disabled = true;
    try {
      const secretRef = ($('#settings-hosted-secret-ref').value || '').trim() || 'HOSTED_JUDGE_API_KEY';
      const hostedApiKey = $('#settings-hosted-api-key').value;
      if (hostedApiKey) {
        await API.call('set_secret_ref', {
          secret_ref: secretRef,
          token: hostedApiKey,
        });
        hostedSecretStatus.secret_stored = true;
        hostedSecretStatus.secret_ref = secretRef;
      }
      const saved = await API.call('save_app_settings', {
        settings: {
          analyzer: {
            judge_mode: $('#settings-analyzer-judge-mode').value,
            judge_prompt_template: $('#settings-analyzer-judge-prompt').value,
            hosted_judge: {
              provider: $('#settings-hosted-provider').value,
              endpoint: $('#settings-hosted-endpoint').value,
              deployment: $('#settings-hosted-deployment').value,
              api_style: $('#settings-hosted-api-style').value,
              api_version: $('#settings-hosted-api-version').value,
              secret_ref: secretRef,
              max_input_chars: Number($('#settings-hosted-max-input-chars').value || 24000),
              max_output_tokens: Number($('#settings-hosted-max-output-tokens').value || 1200),
              request_timeout_seconds: Number($('#settings-hosted-timeout-seconds').value || 60),
              max_retries: Number($('#settings-hosted-max-retries').value || 1),
            },
          },
          logging: {
            enabled: $('#settings-logging-enabled').checked,
            level: $('#settings-log-level').value,
            body_logging_enabled: $('#settings-body-logging-enabled').checked,
          },
          ui: {
            theme: normalizeTheme($('#settings-theme').value),
          },
        },
      });
      const savedTheme = normalizeTheme(saved.ui?.theme || $('#settings-theme').value);
      $('#settings-theme').value = savedTheme;
      applyTheme(savedTheme);
      defaultJudgePromptTemplate = saved.analyzer?.default_judge_prompt_template || defaultJudgePromptTemplate;
      $('#settings-analyzer-judge-mode').value = saved.analyzer?.judge_mode || 'local';
      $('#settings-analyzer-judge-prompt').value =
        saved.analyzer?.judge_prompt_template || defaultJudgePromptTemplate;
      $('#settings-hosted-provider').value = saved.analyzer?.hosted_judge?.provider || 'azure_openai';
      $('#settings-hosted-endpoint').value = saved.analyzer?.hosted_judge?.endpoint || '';
      $('#settings-hosted-deployment').value = saved.analyzer?.hosted_judge?.deployment || '';
      $('#settings-hosted-api-style').value = saved.analyzer?.hosted_judge?.api_style || 'auto';
      $('#settings-hosted-api-version').value = saved.analyzer?.hosted_judge?.api_version || '';
      $('#settings-hosted-secret-ref').value = saved.analyzer?.hosted_judge?.secret_ref || secretRef;
      $('#settings-hosted-api-key').value = '';
      $('#settings-hosted-max-input-chars').value =
        saved.analyzer?.hosted_judge?.max_input_chars || 24000;
      $('#settings-hosted-max-output-tokens').value =
        saved.analyzer?.hosted_judge?.max_output_tokens || 1200;
      $('#settings-hosted-timeout-seconds').value =
        saved.analyzer?.hosted_judge?.request_timeout_seconds || 60;
      $('#settings-hosted-max-retries').value =
        saved.analyzer?.hosted_judge?.max_retries || 1;
      updateJudgePromptStatus(saved);
      updateHostedSecretStatus(saved.analyzer?.hosted_judge || null);
      renderJudgeModeSections(saved.analyzer?.judge_mode || 'local');
      $('#settings-logging-status').innerHTML =
        'Logging changes apply after restarting the app.';
      $('#settings-save-status').innerHTML =
        'Saved to <code>~/hamm0r/config.yaml</code>. Judge changes apply to future Analyze and Judge actions immediately.';
      window.dispatchEvent(new CustomEvent('analyzer-state-changed'));
      toast('Settings saved. Restart the app only if you changed logging.', 'success');
    } catch (err) {
      $('#settings-save-status').textContent = `Save failed: ${err.message}`;
      toast(`Saving settings failed: ${err.message}`, 'error');
    } finally {
      btn.disabled = false;
    }
  }

  $('#btn-settings-save')?.addEventListener('click', () => {
    saveSettingsFromModal($('#btn-settings-save'));
  });

  $('#settings-theme')?.addEventListener('change', (event) => {
    applyTheme(event.target.value);
    $('#settings-save-status').innerHTML =
      'Theme preview applied. Save Settings to persist it.';
  });

  $('#btn-settings-save-logging')?.addEventListener('click', () => {
    saveSettingsFromModal($('#btn-settings-save-logging'));
  });

  // Open + close handlers for the settings modal live in the outer
  // DOMContentLoaded scope (above) so they survive any failure here.

  // ── Home CTA ────────────────────────────────────────────────────────
  $('#btn-home-activate-analyzer').addEventListener('click', openSettingsModal);

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
  if (window.__TAURI__) {
    checkAnalyzerCta();
    loadAppSettings().catch(() => {});
  }
}
