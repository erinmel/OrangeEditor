import test from 'node:test';
import assert from 'node:assert/strict';
import {inspectConfig, releasePlan, rehearsalPlan, inspectReceipt} from './release.mjs';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';

const member = {id: 'editor', name: 'orange-editor', version: '1.2.3', license: 'MIT', targets: [{name: 'orange-editor', kind: ['bin']}]};
const metadata = {workspace_members: ['editor'], packages: [member], resolve: {nodes: []}};
const config = {qualified: true, package: 'orange-editor', binary: 'orange-editor'};
const receipt = {target: 'x86_64-pc-windows-msvc', version: '1.2.3', commit: 'abc', files: ['OrangeEditor-windows.zip'], checks: {signature: true, timestamp: true, installation: true, launch: true}};

test('release metadata has LF line endings and a final newline', () => {
  const script = [
    ". ./common.ps1",
    "$file = [System.IO.Path]::GetTempFileName()",
    "try {",
    "    $text = 'Description: OrangeEditor' + [char]13 + [char]10 + ' Edits typed node graphs.'",
    "    Write-TextFile $file $text",
    "    [Convert]::ToBase64String([System.IO.File]::ReadAllBytes($file))",
    "} finally {",
    "    Remove-Item -LiteralPath $file",
    "}",
  ].join('\n');
  const result = spawnSync('pwsh', ['-NoProfile', '-NonInteractive', '-Command', script], {
    cwd: fileURLToPath(new URL('./release/', import.meta.url)),
    encoding: 'utf8',
  });
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr);
  const bytes = Buffer.from(result.stdout.trim(), 'base64');
  assert.deepEqual(bytes, Buffer.from('Description: OrangeEditor\n Edits typed node graphs.\n'));
});

test('unconfigured library workspace cannot produce a release', () => {
  const disabled = {qualified: false, package: null, binary: null};
  assert.equal(inspectConfig(disabled, metadata), null);
  assert.throws(() => releasePlan(disabled, metadata, 'v1.2.3', () => true), /not configured\/qualified/);
});
test('library, unknown package and unlicensed executable are rejected', () => {
  assert.throws(() => inspectConfig({...config, package: 'missing'}, metadata), /workspace member/);
  assert.throws(() => inspectConfig(config, {...metadata, packages: [{...member, targets: [{name: 'orange-editor', kind: ['lib']}]}]}), /executable/);
  const unlicensed = {...metadata, packages: [{...member, license: null}]};
  assert.equal(inspectConfig(config, unlicensed), unlicensed.packages[0]);
  assert.throws(() => releasePlan(config, unlicensed, 'v1.2.3', () => true), /license/);
});
test('an unsigned preview publishes only prerelease tags and may skip signing checks', () => {
  const preview = {...config, unsigned_preview: true};
  const prerelease = {...metadata, packages: [{...member, version: '1.2.3-preview.1'}]};
  assert.throws(() => releasePlan(preview, metadata, 'v1.2.3', () => true), /prerelease/);
  assert.equal(releasePlan(preview, prerelease, 'v1.2.3-preview.1', () => true).unsigned, true);
  assert.equal(releasePlan(config, metadata, 'v1.2.3', () => true).unsigned, false);
  assert.throws(() => inspectConfig({...config, unsigned_preview: 'yes'}, metadata), /boolean/);
  const unsigned = {...receipt, checks: {...receipt.checks, signature: false, timestamp: false}};
  assert.deepEqual(inspectReceipt(unsigned, 'windows', '1.2.3', 'abc', true), receipt.files);
  assert.throws(() => inspectReceipt(unsigned, 'windows', '1.2.3', 'abc'), /signature/);
  // Waived checks must still be reported, and the install and launch checks still pass.
  const {signature, ...missing} = unsigned.checks;
  assert.throws(() => inspectReceipt({...unsigned, checks: missing}, 'windows', '1.2.3', 'abc', true), /signature/);
  assert.throws(() => inspectReceipt({...unsigned, checks: {...unsigned.checks, launch: false}}, 'windows', '1.2.3', 'abc', true), /launch/);
});
test('a rehearsal needs the package and every adapter but no tag, qualification or license', () => {
  const unqualified = {...config, qualified: false};
  const unlicensed = {...metadata, packages: [{...member, license: null}]};
  assert.equal(rehearsalPlan(unqualified, unlicensed, () => true).version, '1.2.3');
  assert.throws(() => rehearsalPlan({qualified: false, package: null, binary: null}, metadata, () => true), /package and binary/);
  assert.throws(() => rehearsalPlan(unqualified, metadata, file => !file.includes('linux')), /linux/);
});
test('qualification, exact version and every native packager are required', () => {
  assert.throws(() => releasePlan({...config, qualified: false}, metadata, 'v1.2.3', () => true), /qualified/);
  assert.throws(() => releasePlan(config, metadata, 'v1.2.4', () => true), /match/);
  assert.throws(() => releasePlan(config, metadata, 'v1.2.3', file => !file.includes('macos')), /macos/);
  assert.equal(releasePlan(config, metadata, 'v1.2.3', () => true).version, '1.2.3');
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
