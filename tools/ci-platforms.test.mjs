import {test} from 'node:test';
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {platforms} from './ci-platforms.mjs';

test('ordinary coverage retains the three product platforms and both ARM64 hosts', () => {
  assert.deepEqual(platforms(false).include.map(p => p.host), [
    'x86_64-unknown-linux-gnu', 'x86_64-pc-windows-msvc', 'aarch64-apple-darwin',
    'aarch64-unknown-linux-gnu', 'aarch64-pc-windows-msvc',
  ]);
});

test('extended coverage tests the same hosts, without Intel Macs', () => {
  const extended = platforms(true).include;
  assert.deepEqual(extended, platforms(false).include);
  assert.ok(!extended.some(p => p.host === 'x86_64-apple-darwin'));
  assert.equal(new Set(extended.map(p => p.os)).size, extended.length);
});

test('CLI emits a nonempty matrix and rejects malformed mode instead of silently reducing coverage', () => {
  for (const mode of ['true', 'false']) {
    const result = execFileSync(process.execPath, ['tools/ci-platforms.mjs'], {
      encoding: 'utf8', env: {...process.env, EXTENDED: mode},
    });
    assert.deepEqual(JSON.parse(result), platforms(mode === 'true'));
  }
  assert.throws(() => execFileSync(process.execPath, ['tools/ci-platforms.mjs'], {
    env: {...process.env, EXTENDED: 'yes'}, stdio: 'pipe',
  }));
  assert.throws(() => platforms('false'), /boolean/);
});
