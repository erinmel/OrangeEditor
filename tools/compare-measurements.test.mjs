import {test} from 'node:test';
import assert from 'node:assert/strict';
import {annotations, compare, markdown} from './compare-measurements.mjs';

const report = (p50s, pumps = 10) => ({
  workloads: Object.entries(p50s).map(([workload, p50]) => ({
    workload, pumps_per_run: pumps, total_ms: {p50},
  })),
});

test('a verdict needs every head process on one side of every base process', () => {
  const {rows} = compare({
    base: [report({fork: 10, cas: 8, wobble: 5}), report({fork: 11, cas: 8.2, wobble: 7})],
    head: [report({fork: 6, cas: 9, wobble: 6.5}), report({fork: 6.5, cas: 9.5, wobble: 4.6})],
  });
  const verdict = Object.fromEntries(rows.map(r => [r.workload, r.verdict]));
  assert.deepEqual(verdict, {fork: 'faster', cas: 'slower', wobble: 'noisy'});
});

test('changes under the threshold are the same, whatever the ordering', () => {
  const {rows} = compare({base: [report({a: 1.0})], head: [report({a: 1.02})]});
  assert.equal(rows[0].verdict, 'same');
  assert.ok(Math.abs(rows[0].change - 0.02) < 1e-9);
});

test('new, removed and re-counted workloads are reported rather than compared silently', () => {
  const result = compare({
    base: [report({kept: 1, gone: 1}, 10)],
    head: [report({kept: 1, added: 2}, 12)],
  });
  assert.deepEqual(result.removed, ['gone']);
  assert.equal(result.rows.find(r => r.workload === 'added').verdict, 'new');
  assert.equal(result.rows.find(r => r.workload === 'kept').pumpsChanged, true);
  assert.match(markdown(result), /pump count changed/);
  assert.match(markdown(result), /Workloads only in base: gone/);
});

test('allocation counts appear side by side and only slowdowns annotate', () => {
  const counts = (allocations, peak) => ({
    workloads: [{workload: 'fork', allocation_counts: {allocations, peak_live_bytes: peak}}],
  });
  const result = compare({
    base: [report({fork: 10})], head: [report({fork: 20})],
    baseAllocations: counts(66589, 6319021), headAllocations: counts(1899, 4545037),
  });
  assert.match(markdown(result), /66,589 → 1,899/);
  assert.match(markdown(result), /6,319,021 → 4,545,037/);
  assert.deepEqual(annotations(result).length, 1);
  assert.deepEqual(annotations(compare({base: [report({a: 2})], head: [report({a: 1})]})), []);
});

test('scheduled runs without a base report head only, and a missing head is refused', () => {
  const {rows} = compare({base: [], head: [report({a: 1})]});
  assert.deepEqual(rows.map(r => [r.verdict, r.base]), [['unpaired', undefined]]);
  assert.throws(() => compare({base: [report({a: 1})], head: []}), /at least one/);
});
