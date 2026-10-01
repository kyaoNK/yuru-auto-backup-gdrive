// Requires Playwright (or PLAYWRIGHT_MODULE pointing to its installed index.mjs)
// and a locally installed Chrome. All IPC is mocked: no real backups or OS settings.
import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';
import { createServer } from 'vite';
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
const server = await createServer({ server: { host: '127.0.0.1', port: 1439, strictPort: true } });
let browser;
try {
  await server.listen();
  browser = await chromium.launch({ channel: 'chrome', headless: true });
  const page = await browser.newPage({ viewport: { width: 800, height: 700 } });
  const pageErrors = [];
  page.on('pageerror', error => pageErrors.push(error.message));
  await page.addInitScript(() => {
    let id = 0;
    const callbacks = new Map();
    const events = new Map();
    const cfg = { source: 'C:/Projects', destination: 'G:/My Drive/Backup', scheduleTime: '09:00', autoStart: true, excludedFolders: [], excludedFolderNames: [], lastRunAt: null, lastSummary: null, lastError: null };
    const mock = window.__test = {
      config: cfg, status: { ...cfg, running: false, serviceError: null, nextRunAt: '2026-10-01T09:00:00+09:00' },
      preview: { checkedAt: '2026-09-30T12:00:00+09:00', candidates: ['G:/My Drive/Backup/old_Latest.prproj'], retained: ['G:/My Drive/Backup/legacy_Latest.prproj'], errors: [] },
      failLogs: false, failConfig: false, failPicker: false, failOpen: false, failSave: false,
      listenDelay: 0, previewDelay: 0, saveDelay: 0, statusQueue: [], runCount: 0, saved: 0, deleted: 0, failDelete: false,
      listeners: event => [...events.values()].filter(e => e.event === event).length,
      emit(event, payload) { for (const [eventId, entry] of events) if (entry.event === event) callbacks.get(entry.handler)?.({ event, id: eventId, payload }); },
    };
    const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
    const clone = value => JSON.parse(JSON.stringify(value));
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener(_event, eventId) { const entry = events.get(eventId); if (entry) callbacks.delete(entry.handler); } };
    window.__TAURI_INTERNALS__ = {
      transformCallback(callback) { const token = ++id; callbacks.set(token, callback); return token; },
      async invoke(command, args) {
        if (command === 'plugin:event|listen') { await delay(mock.listenDelay); const token = ++id; events.set(token, args); return token; }
        if (command === 'plugin:event|unlisten') { events.delete(args.eventId); return; }
        if (command === 'get_status') {
          const queued = mock.statusQueue.shift();
          if (queued) { await delay(queued.delay); return queued.value; }
          return clone(mock.status);
        }
        if (command === 'get_config') { if (mock.failConfig) throw 'config unreadable'; return clone(mock.config); }
        if (command === 'update_config') { await delay(mock.saveDelay); if (mock.failSave) throw '設定は保存していません: OS error'; mock.config = clone(args.config); mock.saved++; return; }
        if (command === 'preview_deletions') { await delay(mock.previewDelay); return clone(mock.preview); }
        if (command === 'delete_orphan_backup') { if (mock.failDelete) throw '削除条件が変わりました'; mock.deleted++; mock.preview.orphans = []; return; }
        if (command === 'run_now') { mock.runCount++; return true; }
        if (command === 'list_recent_logs') { if (mock.failLogs) throw 'log read failed'; return ['[2026-09-30] [INFO] text containing [ERROR]', '[2026-09-30] [ERROR] real error']; }
        if (command === 'pick_folder') { if (mock.failPicker) throw 'picker failed'; return 'C:/Selected'; }
        if (command === 'open_app_dir') { if (mock.failOpen) throw 'open failed'; return; }
        if (command === 'detect_drive_roots') return [];
        throw new Error(`unhandled command ${command}`);
      },
    };
  });
  const nav = label => page.getByRole('navigation').getByRole('link', { name: label, exact: true });
  await page.goto('http://127.0.0.1:1439');
  await page.getByRole('button', { name: '削除予定を確認', exact: true }).waitFor();
  await page.getByRole('button', { name: '削除予定を確認', exact: true }).click();
  await page.getByRole('heading', { name: '削除予定：1 件' }).waitFor();
  assert.equal(await page.evaluate(() => window.__test.runCount), 0);
  await page.getByText('削除しないバックアップ：1 件', { exact: false }).click();
  assert(await page.getByText('G:/My Drive/Backup/legacy_Latest.prproj', { exact: true }).isVisible());
  await page.evaluate(() => { window.__test.preview = { ...window.__test.preview, candidates: [], errors: [['C:/denied', 'permission denied']] }; });
  await page.getByRole('button', { name: '削除予定を確認', exact: true }).click();
  await page.getByText('一覧は完全ではありません。', { exact: false }).waitFor();
  assert.equal(await page.getByText('現在、削除予定のバックアップはありません。').count(), 0);
  console.log('PASS preview is read-only; partial errors are not presented as empty success');

  await page.evaluate(() => {
    window.__test.preview.errors = [];
    window.__test.preview.orphans = [{ name: 'missing_Latest.prproj', backup: 'G:/Backup/missing_Latest.prproj', source: 'C:/Projects/missing.prproj', missingSince: '2026-07-31T12:00:00+09:00', eligibleAt: '2026-09-30T12:00:00+09:00', reason: '2ヶ月保留済み：確認して削除できます', token: 'test-token' }];
  });
  await page.getByRole('button', { name: '削除予定を確認', exact: true }).click();
  await page.getByRole('button', { name: 'このバックアップの削除を確認' }).click();
  const confirmDelete = page.getByRole('button', { name: '確認した1件を削除', exact: true });
  assert(await confirmDelete.isDisabled());
  await page.getByRole('button', { name: 'キャンセル', exact: true }).click();
  assert.equal(await page.evaluate(() => window.__test.deleted), 0);
  await page.getByRole('button', { name: 'このバックアップの削除を確認' }).click();
  await page.getByRole('checkbox', { name: '内容を確認し、このバックアップの削除に同意します' }).check();
  await page.evaluate(() => { window.__test.failDelete = true; });
  await confirmDelete.click();
  await page.getByRole('status').filter({ hasText: '削除条件が変わりました' }).waitFor();
  assert.equal(await page.evaluate(() => window.__test.deleted), 0);
  await page.evaluate(() => { window.__test.failDelete = false; });
  await page.getByRole('button', { name: 'このバックアップの削除を確認' }).click();
  await page.getByRole('checkbox', { name: '内容を確認し、このバックアップの削除に同意します' }).check();
  await confirmDelete.click();
  await page.getByRole('status').filter({ hasText: '削除しました' }).waitFor();
  assert.equal(await page.evaluate(() => window.__test.deleted), 1);
  console.log('PASS orphan deletion requires acknowledgement, supports cancel, and reports stale-state rejection');

  await page.evaluate(() => { window.__test.failLogs = true; });
  await nav('ログ').click();
  await page.getByRole('alert').filter({ hasText: 'log read failed' }).waitFor();
  await page.evaluate(() => { window.__test.failLogs = false; });
  await page.getByRole('button', { name: '再読み込み', exact: true }).click();
  await page.getByText('[2026-09-30] [ERROR] real error', { exact: true }).waitFor();
  await page.getByRole('button', { name: 'エラー', exact: true }).click();
  assert.equal(await page.getByText('[2026-09-30] [INFO] text containing [ERROR]', { exact: true }).count(), 0);
  console.log('PASS log errors, retry, and level filtering');

  await nav('設定').click();
  await page.getByLabel('監視元フォルダ', { exact: true }).waitFor();
  await page.evaluate(() => { window.__test.failPicker = true; });
  await page.getByRole('button', { name: '選択…', exact: true }).first().click();
  await page.getByRole('status').filter({ hasText: 'picker failed' }).waitFor();
  await page.evaluate(() => { window.__test.failPicker = false; });
  await page.getByRole('button', { name: '選択…', exact: true }).first().click();
  await page.waitForFunction(() => document.querySelector('#source')?.value === 'C:/Selected');
  const exclude = page.getByLabel('追加する除外フォルダ名');
  await exclude.fill('Cache');
  await exclude.dispatchEvent('keydown', { key: 'Enter', isComposing: true });
  assert.equal(await page.getByRole('button', { name: '除外名 Cache を削除' }).count(), 0);
  await page.evaluate(() => { window.__test.saveDelay = 150; });
  await page.getByRole('button', { name: '保存', exact: true }).click();
  assert(await page.getByLabel('実行時刻').isDisabled());
  await page.getByRole('status').filter({ hasText: '保存しました' }).waitFor();
  assert.equal(await page.evaluate(() => window.__test.saved), 1);
  console.log('PASS picker errors, IME Enter, and edit exclusion during save');

  await page.evaluate(() => { window.__test.listenDelay = 200; });
  await nav('ダッシュボード').click();
  await nav('設定').click();
  await page.waitForTimeout(450);
  assert.equal(await page.evaluate(() => window.__test.listeners('status-changed')), 0);
  assert.equal(await page.evaluate(() => window.__test.listeners('navigate-settings')), 1);
  await page.evaluate(() => { window.__test.listenDelay = 0; });
  console.log('PASS late event subscriptions are removed after navigation');

  await nav('ダッシュボード').click();
  await page.getByRole('button', { name: '削除予定を確認', exact: true }).waitFor();
  await page.evaluate(() => {
    const m = window.__test;
    m.statusQueue.push({ delay: 180, value: { ...m.status, serviceError: 'STALE' } }, { delay: 0, value: { ...m.status, serviceError: 'LATEST' } });
    m.emit('status-changed'); m.emit('status-changed');
  });
  await page.getByRole('alert').filter({ hasText: 'LATEST' }).waitFor();
  await page.waitForTimeout(250);
  assert.equal(await page.getByText('STALE', { exact: true }).count(), 0);
  await page.setViewportSize({ width: 360, height: 800 });
  assert(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth));
  await mkdir('test-results', { recursive: true });
  await page.screenshot({ path: 'test-results/dashboard.png', fullPage: true });
  assert.deepEqual(pageErrors, []);
  console.log('PASS stale responses, narrow layout; 0 unhandled browser errors');
} finally {
  await browser?.close();
  await server.close();
}
