import test from 'node:test';
import assert from 'node:assert/strict';
import {inspectConfig, releasePlan, inspectReceipt} from './release.mjs';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';

const member = {id: 'editor', name: 'orange-editor', version: '1.2.3', license: 'MIT', targets: [{name: 'orange-editor', kind: ['bin']}]};
const metadata = {workspace_members: ['editor'], packages: [member], resolve: {nodes: []}};
const config = {qualified: true, package: 'orange-editor', binary: 'orange-editor'};
const receipt = {target: 'x86_64-pc-windows-msvc', version: '1.2.3', commit: 'abc', files: ['OrangeEditor-windows.zip'], checks: {signature: true, timestamp: true, installation: true, launch: true}};

test('unconfigured library workspace cannot produce a release', () => {
  const disabled = {qualified: false, package: null, binary: null};
  assert.equal(inspectConfig(disabled, metadata), null);
  assert.throws(() => releasePlan(disabled, metadata, 'v1.2.3', () => true), /not configured\/qualified/);
});
test('library, unknown package and unlicensed executable are rejected', () => {
  assert.throws(() => inspectConfig({...config, package: 'missing'}, metadata), /workspace member/);
  assert.throws(() => inspectConfig(config, {...metadata, packages: [{...member, targets: [{name: 'orange-editor', kind: ['lib']}]}]}), /executable/);
  assert.throws(() => inspectConfig(config, {...metadata, packages: [{...member, license: null}]}), /license/);
});
test('qualification, exact version and every native packager are required', () => {
  assert.throws(() => releasePlan({...config, qualified: false}, metadata, 'v1.2.3', () => true), /qualified/);
  assert.throws(() => releasePlan(config, metadata, 'v1.2.4', () => true), /match/);
  assert.throws(() => releasePlan(config, metadata, 'v1.2.3', file => !file.includes('macos')), /macos/);
  assert.equal(releasePlan(config, metadata, 'v1.2.3', () => true).version, '1.2.3');
});
test('Bevy development linking cannot enter a release', () => {
  const graph = {...metadata, packages: [...metadata.packages, {id: 'bevy', name: 'bevy'}], resolve: {nodes: [{id: 'bevy', features: ['dynamic_linking']}]}};
  assert.throws(() => inspectConfig(config, graph), /development/);
});
test('receipts must identify the exact target, version and commit', () => {
  assert.throws(() => inspectReceipt(receipt, 'linux', '1.2.3', 'abc'), /match/);
  assert.throws(() => inspectReceipt(receipt, 'windows', '1.2.4', 'abc'), /match/);
  assert.throws(() => inspectReceipt(receipt, 'windows', '1.2.3', 'other'), /match/);
});
test('failed signing or install checks block publication', () => {
  assert.throws(() => inspectReceipt({...receipt, checks: {...receipt.checks, signature: false}}, 'windows', '1.2.3', 'abc'), /signature/);
  assert.throws(() => inspectReceipt({...receipt, checks: {...receipt.checks, installation: false}}, 'windows', '1.2.3', 'abc'), /installation/);
});
test('package paths and duplicate or empty artifact lists are rejected', () => {
  for (const files of [[], ['../escape.zip'], ['duplicate.zip', 'duplicate.zip'], ['script.sh']]) assert.throws(() => inspectReceipt({...receipt, files}, 'windows', '1.2.3', 'abc'));
  assert.deepEqual(inspectReceipt(receipt, 'windows', '1.2.3', 'abc'), receipt.files);
});

test('package collection requires all three native receipts and hashes actual package bytes', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'orange-release-'));
  try {
    const targets = {linux: ['x86_64-unknown-linux-gnu', ['dependencies', 'installation', 'launch']], windows: [receipt.target, Object.keys(receipt.checks)], macos: ['aarch64-apple-darwin', ['signature', 'notarization', 'staple', 'installation', 'launch']]};
    const invoke = () => spawnSync(process.execPath, [fileURLToPath(new URL('./release.mjs', import.meta.url)), 'collect', root], {encoding: 'utf8', env: {...process.env, RELEASE_VERSION: '1.2.3', RELEASE_COMMIT: 'abc'}});
    assert.notEqual(invoke().status, 0);
    for (const [platform, [target, checks]] of Object.entries(targets)) {
      const directory = path.join(root, `release-${platform}`);
      fs.mkdirSync(directory);
      const filename = `OrangeEditor-${platform}.zip`;
      fs.writeFileSync(path.join(directory, filename), 'abc');
      fs.writeFileSync(path.join(directory, 'SHA256SUMS'), `ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  ${filename}\n`);
      fs.writeFileSync(path.join(directory, 'receipt.json'), JSON.stringify({...receipt, target, files: [filename], checks: Object.fromEntries(checks.map(check => [check, true]))}));
    }
    const result = invoke();
    assert.equal(result.status, 0, result.stderr);
    assert.match(fs.readFileSync(path.join(root, 'SHA256SUMS'), 'utf8'), /ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad/);
    assert.equal(fs.readFileSync(path.join(root, 'publish-files.txt'), 'utf8').trim().split('\n').length, 3);
    fs.writeFileSync(path.join(root, 'release-linux', 'OrangeEditor-linux.zip'), 'tampered');
    assert.notEqual(invoke().status, 0);
    fs.writeFileSync(path.join(root, 'release-linux', 'OrangeEditor-linux.zip'), 'abc');
    fs.writeFileSync(path.join(root, 'release-linux', 'unexpected.env'), 'not-a-secret');
    assert.notEqual(invoke().status, 0);
  } finally {
    const relative = path.relative(fs.realpathSync(os.tmpdir()), fs.realpathSync(root));
    if (relative.startsWith('..') || path.isAbsolute(relative)) throw new Error('Refusing cleanup outside the temporary test directory.');
    fs.rmSync(root, {recursive: true, force: true});
  }
});
