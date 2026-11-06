import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {canonical, vectors} from './package-archive.mjs';

const fixture = JSON.parse(readFileSync(
  new URL('../app/crates/orange-packages/tests/fixtures/archive-v1.json', import.meta.url), 'utf8'));

test('the reference encoder reproduces the committed archive vectors byte for byte', () => {
  assert.deepEqual(vectors(), fixture);
});

test('canonical JSON sorts by UTF-16 code units and escapes only what JCS escapes', () => {
  assert.equal(canonical({'\u{e000}': 1, '\u{1f600}': 2, A: 3, a: 4}), '{"A":3,"a":4,"\u{1f600}":2,"\u{e000}":1}');
  assert.equal(canonical('q" \\ / \b\f\n\r\t \u0001 \u007f  '), '"q\\" \\\\ / \\b\\f\\n\\r\\t \\u0001 \u007f  "');
  assert.throws(() => canonical(1.5));
});
