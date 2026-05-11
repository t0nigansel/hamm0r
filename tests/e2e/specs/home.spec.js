import { test, expect } from '../fixtures/index.js';

test('home shows empty state when there are no engagements', async ({ page }) => {
  await page.goto('/');

  // REQ-006
  await expect(page.locator('#home-recent-list')).toContainText('no engagements yet');
});

test('home shows recent engagements with names and dates', async ({ page }) => {
  await page.addInitScript(() => {
    window.__tauriHandlers.list_engagements = () => ([
      { slug: 'alpha-scan', name: 'Alpha Scan', created_at: '2026-05-02T10:00:00Z' },
      { slug: 'bravo-audit', name: 'Bravo Audit', created_at: '2026-05-01T09:30:00Z' },
    ]);
    window.__tauriHandlers.list_runs = ({ engagementSlug }) => {
      if (engagementSlug === 'alpha-scan') return [{ id: 'run-1', status: 'completed' }];
      if (engagementSlug === 'bravo-audit') return [{ id: 'run-2', status: 'running' }];
      return [];
    };
  });

  await page.goto('/');

  // REQ-006
  await expect(page.locator('#home-recent-list')).toContainText('Alpha Scan');
  await expect(page.locator('#home-recent-list')).toContainText('Bravo Audit');
  await expect(page.locator('#home-recent-list')).toContainText('2026-05-02');
  await expect(page.locator('#home-recent-list')).toContainText('2026-05-01');
  await expect(page.locator('#home-recent-list')).toContainText('Done');
  await expect(page.locator('#home-recent-list')).toContainText('Running');
});

test('home shows an error empty state when engagements cannot be loaded', async ({ page }) => {
  await page.addInitScript(() => {
    window.__tauriHandlers.list_engagements = () => {
      throw new Error('backend unavailable');
    };
  });

  await page.goto('/');

  // REQ-006 plus test.md error-state directive
  await expect(page.locator('#home-recent-list')).toContainText('could not load recent engagements');
});
