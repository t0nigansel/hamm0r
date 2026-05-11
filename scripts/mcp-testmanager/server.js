import { Server } from '@modelcontextprotocol/sdk/server/index.js';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import {
  CallToolRequestSchema,
  ListResourcesRequestSchema,
  ListToolsRequestSchema,
  ReadResourceRequestSchema,
} from '@modelcontextprotocol/sdk/types.js';
import { readFileSync, writeFileSync, readdirSync, existsSync } from 'fs';
import { execSync, spawnSync } from 'child_process';
import path from 'path';
import { fileURLToPath } from 'url';

// Project root is two levels above scripts/mcp-testmanager/
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const SPECS_DIR = path.join(ROOT, 'tests/e2e/specs');
const E2E_DIR = path.join(ROOT, 'tests/e2e');

function readProjectFile(relPath) {
  const abs = path.join(ROOT, relPath);
  if (!existsSync(abs)) return `(file not found: ${relPath})`;
  return readFileSync(abs, 'utf8');
}

// Parse data-view attributes and key DOM IDs from index.html
function parseUiViews() {
  const html = readProjectFile('ui/index.html');
  const views = [...html.matchAll(/data-view="([^"]+)"/g)].map(m => m[1]);
  const ids = [...html.matchAll(/\bid="([^"]+)"/g)].map(m => m[1]);
  return {
    views: [...new Set(views)],
    // Return only IDs likely useful for selectors (not internal framework ids)
    domIds: [...new Set(ids)].filter(id => !id.startsWith('__')),
  };
}

// Parse all invoke() command names from ui/js/api.js
function parseTauriCommands() {
  const src = readProjectFile('ui/js/api.js');
  const matches = [...src.matchAll(/invoke\(\s*['"]([^'"]+)['"]/g)];
  return [...new Set(matches.map(m => m[1]))].sort();
}

// Return git diff of ui/, req.md, and test.md since the previous commit.
// Falls back to the full HEAD tree on the first commit (no parent).
function getUiDiff() {
  const r = spawnSync('git', ['diff', 'HEAD^', '--', 'ui/', 'req.md', 'test.md'], {
    cwd: ROOT, encoding: 'utf8',
  });
  if (r.status !== 0) {
    // First commit — show what was introduced
    const r2 = spawnSync('git', ['show', '--stat', 'HEAD', '--', 'ui/', 'req.md', 'test.md'], {
      cwd: ROOT, encoding: 'utf8',
    });
    return r2.stdout || '(no diff available)';
  }
  return r.stdout || '(no changes to ui/, req.md, or test.md in last commit)';
}

// Guard: only allow writing .spec.js files inside tests/e2e/specs/
function validateSpecFilename(filename) {
  if (!filename.endsWith('.spec.js')) throw new Error('filename must end with .spec.js');
  if (filename.includes('..') || filename.includes('/')) throw new Error('filename must be a plain filename, no path separators');
}

const server = new Server(
  { name: 'hamm0r-testmanager', version: '1.0.0' },
  { capabilities: { resources: {}, tools: {} } }
);

// ── Resources ────────────────────────────────────────────────────────────────

server.setRequestHandler(ListResourcesRequestSchema, async () => ({
  resources: [
    {
      uri: 'hamm0r://requirements',
      name: 'Software Requirements (req.md)',
      description: 'PO-maintained functional requirements. Each REQ-NNN entry maps to expected test coverage.',
      mimeType: 'text/markdown',
    },
    {
      uri: 'hamm0r://test-guidance',
      name: 'Test Directives (test.md)',
      description: 'Test manager directives: scope, boundary rules, mock strategy, regression guards.',
      mimeType: 'text/markdown',
    },
    {
      uri: 'hamm0r://ui-source',
      name: 'UI Source (ui/index.html)',
      description: 'Raw HTML for the hamm0r UI — use to discover DOM IDs and data-view attributes for selectors.',
      mimeType: 'text/html',
    },
  ],
}));

server.setRequestHandler(ReadResourceRequestSchema, async (req) => {
  switch (req.params.uri) {
    case 'hamm0r://requirements':
      return { contents: [{ uri: req.params.uri, mimeType: 'text/markdown', text: readProjectFile('req.md') }] };
    case 'hamm0r://test-guidance':
      return { contents: [{ uri: req.params.uri, mimeType: 'text/markdown', text: readProjectFile('test.md') }] };
    case 'hamm0r://ui-source':
      return { contents: [{ uri: req.params.uri, mimeType: 'text/html', text: readProjectFile('ui/index.html') }] };
    default:
      throw new Error(`Unknown resource: ${req.params.uri}`);
  }
});

// ── Tools ─────────────────────────────────────────────────────────────────────

server.setRequestHandler(ListToolsRequestSchema, async () => ({
  tools: [
    {
      name: 'read_requirements',
      description: 'Returns the full text of req.md — the PO-maintained software requirements.',
      inputSchema: { type: 'object', properties: {} },
    },
    {
      name: 'read_test_directives',
      description: 'Returns the full text of test.md — test manager guidance on scope, boundaries, and strategy.',
      inputSchema: { type: 'object', properties: {} },
    },
    {
      name: 'list_ui_views',
      description: 'Parses ui/index.html and returns all data-view values and DOM IDs available as Playwright selectors.',
      inputSchema: { type: 'object', properties: {} },
    },
    {
      name: 'list_test_specs',
      description: 'Returns the filenames of all existing Playwright spec files in tests/e2e/specs/.',
      inputSchema: { type: 'object', properties: {} },
    },
    {
      name: 'read_test_spec',
      description: 'Returns the contents of a specific spec file from tests/e2e/specs/.',
      inputSchema: {
        type: 'object',
        properties: { filename: { type: 'string', description: 'e.g. navigation.spec.js' } },
        required: ['filename'],
      },
    },
    {
      name: 'write_test_spec',
      description: 'Writes (or overwrites) a spec file in tests/e2e/specs/. Filename must end with .spec.js and contain no path separators.',
      inputSchema: {
        type: 'object',
        properties: {
          filename: { type: 'string', description: 'e.g. navigation.spec.js' },
          content: { type: 'string', description: 'Full content of the spec file.' },
        },
        required: ['filename', 'content'],
      },
    },
    {
      name: 'run_tests',
      description: 'Runs the Playwright test suite from tests/e2e/. Optionally pass a spec filename to run only that file. Returns stdout, stderr, and exit code.',
      inputSchema: {
        type: 'object',
        properties: {
          spec: { type: 'string', description: 'Optional spec filename to run a single file, e.g. navigation.spec.js' },
        },
      },
    },
    {
      name: 'get_ui_diff',
      description: 'Returns the git diff of ui/, req.md, and test.md since the previous commit. Use this to determine which parts of the UI changed so only affected specs need updating.',
      inputSchema: { type: 'object', properties: {} },
    },
    {
      name: 'get_tauri_commands',
      description: 'Parses ui/js/api.js and returns every Tauri invoke() command name. Use this to verify that all backend commands have test coverage.',
      inputSchema: { type: 'object', properties: {} },
    },
    {
      name: 'get_test_report',
      description: 'Reads the last Playwright JSON report (tests/e2e/playwright-report/results.json) and returns a structured summary of passed, failed, and skipped tests. Returns an error if no report exists yet.',
      inputSchema: { type: 'object', properties: {} },
    },
  ],
}));

server.setRequestHandler(CallToolRequestSchema, async (req) => {
  const { name, arguments: args } = req.params;

  switch (name) {
    case 'read_requirements':
      return { content: [{ type: 'text', text: readProjectFile('req.md') }] };

    case 'read_test_directives':
      return { content: [{ type: 'text', text: readProjectFile('test.md') }] };

    case 'list_ui_views': {
      const result = parseUiViews();
      return { content: [{ type: 'text', text: JSON.stringify(result, null, 2) }] };
    }

    case 'list_test_specs': {
      if (!existsSync(SPECS_DIR)) return { content: [{ type: 'text', text: '[]' }] };
      const files = readdirSync(SPECS_DIR).filter(f => f.endsWith('.spec.js'));
      return { content: [{ type: 'text', text: JSON.stringify(files) }] };
    }

    case 'read_test_spec': {
      validateSpecFilename(args.filename);
      const content = readProjectFile(`tests/e2e/specs/${args.filename}`);
      return { content: [{ type: 'text', text: content }] };
    }

    case 'write_test_spec': {
      validateSpecFilename(args.filename);
      const dest = path.join(SPECS_DIR, args.filename);
      writeFileSync(dest, args.content, 'utf8');
      return { content: [{ type: 'text', text: `Written: ${dest}` }] };
    }

    case 'run_tests': {
      const specArg = args?.spec ? `specs/${args.spec}` : '';
      try {
        const stdout = execSync(
          `npx playwright test ${specArg} --reporter=list`.trim(),
          { cwd: E2E_DIR, timeout: 120_000, encoding: 'utf8' }
        );
        return { content: [{ type: 'text', text: stdout }] };
      } catch (err) {
        return {
          content: [{ type: 'text', text: `EXIT ${err.status}\n${err.stdout}\n${err.stderr}` }],
          isError: true,
        };
      }
    }

    case 'get_ui_diff':
      return { content: [{ type: 'text', text: getUiDiff() }] };

    case 'get_tauri_commands': {
      const commands = parseTauriCommands();
      return { content: [{ type: 'text', text: JSON.stringify(commands, null, 2) }] };
    }

    case 'get_test_report': {
      const reportPath = path.join(E2E_DIR, 'playwright-report', 'results.json');
      if (!existsSync(reportPath)) {
        return {
          content: [{ type: 'text', text: 'No report found. Run run_tests() first.' }],
          isError: true,
        };
      }
      const raw = JSON.parse(readFileSync(reportPath, 'utf8'));
      const summary = {
        passed: 0, failed: 0, skipped: 0,
        failures: [],
      };
      for (const suite of raw.suites ?? []) {
        for (const spec of suite.specs ?? []) {
          for (const test of spec.tests ?? []) {
            const status = test.results?.[0]?.status ?? 'unknown';
            if (status === 'passed') summary.passed++;
            else if (status === 'failed' || status === 'timedOut') {
              summary.failed++;
              summary.failures.push({
                title: `${suite.title} › ${spec.title}`,
                error: test.results?.[0]?.error?.message ?? '(no message)',
              });
            } else {
              summary.skipped++;
            }
          }
        }
      }
      return { content: [{ type: 'text', text: JSON.stringify(summary, null, 2) }] };
    }

    default:
      throw new Error(`Unknown tool: ${name}`);
  }
});

// ── Start ─────────────────────────────────────────────────────────────────────

const transport = new StdioServerTransport();
await server.connect(transport);
