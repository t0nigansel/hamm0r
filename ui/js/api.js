/**
 * api.js — Communication layer between the UI and the Python sidecar.
 *
 * When running inside Tauri, uses `window.__TAURI__.core.invoke()`.
 * When running standalone (for development), falls back to a WebSocket or
 * direct subprocess connection.
 *
 * Decision: For v0.1, we provide a dev-mode fallback that communicates
 * with the Python sidecar via a simple fetch-based proxy. In production,
 * all communication goes through Tauri's invoke().
 */

const API = (() => {
  let _requestId = 0;

  /**
   * Send a command to the Python sidecar and return the result.
   * @param {string} cmd - Command name (e.g. "list_prompts")
   * @param {object} params - Command parameters
   * @returns {Promise<any>} - The response data
   */
  async function call(cmd, params = {}) {
    // Tauri mode: use invoke to call the Rust sidecar_cmd handler
    if (window.__TAURI__) {
      try {
        const result = await window.__TAURI__.core.invoke('sidecar_cmd', {
          cmd: cmd,
          params: params,
        });
        return result;
      } catch (err) {
        throw new Error(typeof err === 'string' ? err : JSON.stringify(err));
      }
    }

    // Dev mode fallback: direct HTTP to a small dev server
    // (see sidecar/dev_server.py if you want to run the UI without Tauri)
    // For now, use the dev server if available, otherwise throw.
    try {
      const resp = await fetch('http://localhost:9274/api', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ id: String(++_requestId), cmd, params }),
      });
      const data = await resp.json();
      if (data.ok) return data.data;
      throw new Error(data.error || 'Unknown error');
    } catch (err) {
      if (err.message.includes('Failed to fetch') || err.message.includes('NetworkError')) {
        throw new Error(
          'Not running inside Tauri and dev server not available. ' +
          'Start the dev server: python -m sidecar.dev_server'
        );
      }
      throw err;
    }
  }

  return { call };
})();
