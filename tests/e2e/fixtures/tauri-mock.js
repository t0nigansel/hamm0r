// Injected via page.addInitScript() before api.js and app.js load.
// Stubs window.__TAURI__ so the UI boots without a real Tauri runtime.
// Tests can override individual handlers via window.__tauriHandlers before goto().

window.__tauriHandlers = {
  // Minimal defaults so the page loads cleanly (no engagement open).
  // list_prompts / list_scenarios return HashMap format that api.js flattens.
  list_targets:        () => [],
  list_prompts:        () => ({}),
  list_engagements:    () => [],
  list_scenarios:      () => ({}),
  list_runs:           () => [],
  get_app_settings:    () => ({ logging: { enabled: false, level: 'info', body_logging_enabled: false } }),
  get_analyzer_status: () => ({ installed: false }),
};

window.__tauriListeners = {};

window.__TAURI__ = {
  core: {
    invoke: (cmd, args) => {
      const h = window.__tauriHandlers[cmd];
      if (!h) return Promise.reject(new Error('[tauri-mock] No handler for: ' + cmd));
      try {
        return Promise.resolve(h(args ?? {}));
      } catch (err) {
        return Promise.reject(err);
      }
    },
  },
  event: {
    listen: (event, cb) => {
      window.__tauriListeners[event] = cb;
      return Promise.resolve(() => { delete window.__tauriListeners[event]; });
    },
    emit: () => Promise.resolve(),
  },
};

// Fire a synthetic backend event from test code:
//   await page.evaluate(() => window.__emitTauriEvent('run-progress', { ... }))
window.__emitTauriEvent = (event, payload) => {
  const cb = window.__tauriListeners[event];
  if (cb) cb({ payload });
};
