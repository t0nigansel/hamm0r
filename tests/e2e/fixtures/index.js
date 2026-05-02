import { test as base, expect } from '@playwright/test';
import { readFileSync } from 'fs';
import { fileURLToPath } from 'url';
import path from 'path';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const tauriMockScript = readFileSync(path.join(__dirname, 'tauri-mock.js'), 'utf8');

// Extend base test to inject window.__TAURI__ before any page scripts run.
// Tests may call page.addInitScript() for command-specific overrides
// before page.goto() — those scripts execute after this one.
export const test = base.extend({
  page: async ({ page }, use) => {
    await page.addInitScript(tauriMockScript);
    await use(page);
  },
});

export { expect };
