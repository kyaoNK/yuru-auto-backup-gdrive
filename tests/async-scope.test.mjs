import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';

const source = readFileSync(new URL('../src/lib/async-scope.ts', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } });
const { createAsyncScope } = await import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);
const tick = () => new Promise(resolve => setImmediate(resolve));

test('newest response wins independently for each operation', () => {
  const scope = createAsyncScope();
  const old = scope.ticket('status');
  const preview = scope.ticket('preview');
  const current = scope.ticket('status');
  assert.equal(old(), false);
  assert.equal(current(), true);
  assert.equal(preview(), true);
  scope.dispose();
  assert.equal(current(), false);
  assert.equal(preview(), false);
});

test('subscription resolving after unmount is immediately removed', async () => {
  const scope = createAsyncScope();
  let resolve;
  let removed = 0;
  const own = scope.own(new Promise(r => { resolve = r; }), assert.fail);
  scope.dispose();
  resolve(() => { removed++; });
  await own;
  await tick();
  assert.equal(removed, 1);
});

test('subscription failures are handled; late failures are ignored', async () => {
  const scope = createAsyncScope();
  let count = 0;
  await scope.own(Promise.reject(new Error('listen')), () => { count++; });
  scope.dispose();
  await scope.own(Promise.reject(new Error('late')), () => { count++; });
  assert.equal(count, 1);
});

test('cleanup is idempotent and rejected async cleanup is handled', async () => {
  const scope = createAsyncScope();
  let count = 0;
  await scope.own(Promise.resolve(async () => { count++; throw new Error('unlisten'); }), assert.fail);
  scope.dispose();
  scope.dispose();
  await tick();
  assert.equal(count, 1);
});
