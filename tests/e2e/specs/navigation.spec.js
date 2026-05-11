import { test, expect } from '../fixtures/index.js';

test('sidebar navigation updates active state and breadcrumb', async ({ page }) => {
  await page.goto('/');

  // REQ-001 REQ-002 REQ-003
  // Targets and Workbench views were removed in Phase 2F of
  // docs/RefactorPlan.md. The sidebar now exposes Home, Requests,
  // Libraries, Scenarios, Engagements (view-runs).
  const cases = [
    { view: 'view-home', label: 'home' },
    { view: 'view-runs', label: 'runs' },
    { view: 'view-requests', label: 'requests' },
    { view: 'view-library', label: 'library' },
    { view: 'view-scenarios', label: 'scenarios' },
  ];

  for (const { view, label } of cases) {
    await page.locator(`.nav-item[data-view="${view}"]`).click();
    await expect(page.locator('#breadcrumb-view')).toHaveText(label);
    await expect(page.locator(`.nav-item[data-view="${view}"]`)).toHaveClass(/active/);
    await expect(page.locator(`#${view}`)).toHaveClass(/active/);
  }
});

test('topbar shows default engagement labels and action buttons', async ({ page }) => {
  await page.goto('/');

  // REQ-004 REQ-005 REQ-008
  await expect(page.locator('#breadcrumb-engagement')).toHaveText('no engagement open');
  await expect(page.locator('#db-label')).toHaveText('no engagement');
  await expect(page.locator('#btn-new-engagement')).toBeVisible();
  await expect(page.locator('#btn-open-engagement')).toBeVisible();
});
