import fs from 'node:fs';
import {pathToFileURL} from 'node:url';

// Paired base/head comparison of `orange-measure --execution` reports from one runner.
// Hosted runners differ in CPU model between runs, so only processes measured on the
// same machine are compared; the verdict needs every head process on the same side of
// every base process, so one noisy process cannot produce a claim.

const median = values => {
  const sorted = [...values].sort((a, b) => a - b);
  const middle = sorted.length >> 1;
  return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
};

function byWorkload(reports) {
  const rows = new Map();
  for (const report of reports) {
    for (const workload of report.workloads) {
      if (!rows.has(workload.workload)) rows.set(workload.workload, []);
      rows.get(workload.workload).push(workload);
    }
  }
  return rows;
}

/// Compare timing reports (several processes per side) and optional allocation reports.
export function compare({base, head, baseAllocations, headAllocations, threshold = 0.05}) {
  if (!head.length) throw new Error('head needs at least one timing report');
  const before = byWorkload(base);
  const after = byWorkload(head);
  const allocationsBefore = baseAllocations ? byWorkload([baseAllocations]) : new Map();
  const allocationsAfter = headAllocations ? byWorkload([headAllocations]) : new Map();
  const rows = [];
  for (const [name, heads] of after) {
    const bases = before.get(name);
    const row = {workload: name, verdict: base.length ? 'new' : 'unpaired'};
    const headP50 = heads.map(w => w.total_ms.p50);
    row.head = median(headP50);
    if (bases) {
      const baseP50 = bases.map(w => w.total_ms.p50);
      row.base = median(baseP50);
      row.change = row.head / row.base - 1;
      if (bases.some(w => w.pumps_per_run !== heads[0].pumps_per_run)) row.pumpsChanged = true;
      const slower = Math.min(...headP50) > Math.max(...baseP50);
      const faster = Math.max(...headP50) < Math.min(...baseP50);
      row.verdict = Math.abs(row.change) < threshold ? 'same'
        : slower ? 'slower' : faster ? 'faster' : 'noisy';
    }
    const allocation = side => side.get(name)?.[0]?.allocation_counts ?? null;
    row.allocationsBase = allocation(allocationsBefore);
    row.allocationsHead = allocation(allocationsAfter);
    rows.push(row);
  }
  const removed = [...before.keys()].filter(name => !after.has(name));
  return {rows, removed};
}

const ms = value => value === undefined ? '—' : value.toFixed(3);
const percent = value => value === undefined ? '—' : `${value >= 0 ? '+' : ''}${(value * 100).toFixed(1)}%`;
const count = (before, after, key) => before && after
  ? `${before[key].toLocaleString('en-US')} → ${after[key].toLocaleString('en-US')}`
  : after ? after[key].toLocaleString('en-US') : '—';

export function markdown({rows, removed}) {
  const lines = [
    '| Workload | Base p50 ms | Head p50 ms | Change | Verdict | Allocations | Peak live bytes |',
    '|---|---:|---:|---:|---|---:|---:|',
  ];
  for (const row of rows) {
    const verdict = row.pumpsChanged ? `${row.verdict} (pump count changed)` : row.verdict;
    lines.push(`| ${row.workload} | ${ms(row.base)} | ${ms(row.head)} | ${percent(row.change)} | ${verdict} | ${count(row.allocationsBase, row.allocationsHead, 'allocations')} | ${count(row.allocationsBase, row.allocationsHead, 'peak_live_bytes')} |`);
  }
  if (removed.length) lines.push('', `Workloads only in base: ${removed.join(', ')}`);
  return lines.join('\n');
}

/// GitHub annotations for consistent slowdowns; observations, never a failure.
export function annotations({rows}) {
  return rows
    .filter(row => row.verdict === 'slower')
    .map(row => `::warning title=Slower execution workload::${row.workload} p50 ${percent(row.change)} against base on the same runner`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [directory] = process.argv.slice(2);
  if (!directory) throw new Error('usage: compare-measurements.mjs <directory with base-*.json, head-*.json>');
  const read = file => JSON.parse(fs.readFileSync(`${directory}/${file}`, 'utf8'));
  const files = fs.readdirSync(directory);
  const timing = side => files.filter(f => f.startsWith(`${side}-timing-`) && f.endsWith('.json')).sort().map(read);
  const optional = file => files.includes(file) ? read(file) : undefined;
  const result = compare({
    base: timing('base'),
    head: timing('head'),
    baseAllocations: optional('base-allocations.json'),
    headAllocations: optional('head-allocations.json'),
  });
  console.log(markdown(result));
  for (const line of annotations(result)) console.error(line);
}
