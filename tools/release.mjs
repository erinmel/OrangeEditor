import {execFileSync} from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';

export const platforms = {
  linux: {target: 'x86_64-unknown-linux-gnu', checks: ['dependencies', 'installation', 'launch']},
  windows: {target: 'x86_64-pc-windows-msvc', checks: ['signature', 'timestamp', 'installation', 'launch']},
  macos: {target: 'aarch64-apple-darwin', checks: ['signature', 'notarization', 'staple', 'installation', 'launch']},
};
const identifier = /^[a-zA-Z0-9][a-zA-Z0-9_-]*$/;
const releaseTag = /^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/;

export function inspectConfig(config, metadata) {
  if (!config || typeof config.qualified !== 'boolean') throw new Error('Release configuration must declare qualified as a boolean.');
  if (config.package === null && config.binary === null && !config.qualified) return null;
  if (!identifier.test(config.package ?? '') || !identifier.test(config.binary ?? '')) throw new Error('Configure an explicit release package and binary.');
  const member = metadata.packages.find(pkg => metadata.workspace_members.includes(pkg.id) && pkg.name === config.package);
  if (!member) throw new Error('Release package is not a workspace member.');
  if (!member.targets.some(target => target.kind.includes('bin') && target.name === config.binary)) throw new Error('Release target is not a workspace executable.');
  if (!member.license && !member.license_file) throw new Error('The release package needs a reviewed license declaration.');
  return member;
}

export function releasePlan(config, metadata, tag, exists) {
  const member = inspectConfig(config, metadata);
  if (!member || !config.qualified) throw new Error('Desktop release is not configured/qualified: add the real executable and complete packaging/signing/runtime qualification before releasing.');
  if (!releaseTag.test(tag) || tag.slice(1) !== member.version) throw new Error('Annotated v<version> release tag must match the executable package version.');
  for (const platform of Object.keys(platforms)) {
    if (!exists(`tools/release/${platform}.ps1`)) throw new Error(`Missing qualified packaging adapter: tools/release/${platform}.ps1`);
  }
  return {package: config.package, binary: config.binary, version: member.version};
}

export function inspectReceipt(receipt, platform, version, commit) {
  const contract = platforms[platform];
  if (Object.keys(receipt).some(key => !['target', 'version', 'commit', 'checks', 'files'].includes(key))) throw new Error('Unexpected release receipt field.');
  if (!contract || receipt.target !== contract.target || receipt.version !== version || receipt.commit !== commit) throw new Error('Release receipt does not match platform, version and source commit.');
  if (Object.keys(receipt.checks ?? {}).some(check => !contract.checks.includes(check))) throw new Error('Unexpected release verification field.');
  for (const check of contract.checks) if (receipt.checks?.[check] !== true) throw new Error(`Release receipt lacks passing ${platform}/${check} evidence.`);
  if (!Array.isArray(receipt.files) || !receipt.files.length || new Set(receipt.files).size !== receipt.files.length) throw new Error('Release receipt must list unique package files.');
  for (const file of receipt.files) {
    if (!/^[a-zA-Z0-9][a-zA-Z0-9_.-]*\.(zip|tar\.gz|msi|dmg|pkg|deb|rpm|AppImage)$/.test(file) || file !== path.basename(file)) throw new Error('Invalid release package filename.');
  }
  return receipt.files;
}

function verifyDirectory(directory, platform, version, commit, requireChecksums = false) {
  if (!fs.lstatSync(path.join(directory, 'receipt.json')).isFile()) throw new Error('Release receipt must be a regular file.');
  const receipt = JSON.parse(fs.readFileSync(path.join(directory, 'receipt.json'), 'utf8'));
  const files = inspectReceipt(receipt, platform, version, commit);
  if (fs.readdirSync(directory).some(file => ![...files, 'receipt.json', 'SHA256SUMS'].includes(file))) throw new Error('Unexpected file in release output directory.');
  const sums = files.map(file => {
    const filename = path.join(directory, file);
    const stat = fs.lstatSync(filename);
    if (!stat.isFile() || stat.size === 0) throw new Error('Release package must be a nonempty regular file.');
    const hash = createHash('sha256');
    const buffer = Buffer.alloc(1024 * 1024);
    const descriptor = fs.openSync(filename, 'r');
    try {
      let length;
      while ((length = fs.readSync(descriptor, buffer)) > 0) hash.update(buffer.subarray(0, length));
    } finally { fs.closeSync(descriptor); }
    return `${hash.digest('hex')}  ${file}`;
  });
  const checksumPath = path.join(directory, 'SHA256SUMS');
  const checksumText = sums.join('\n') + '\n';
  if (requireChecksums && !fs.existsSync(checksumPath)) throw new Error('Verified package checksums are missing.');
  if (fs.existsSync(checksumPath) && (!fs.lstatSync(checksumPath).isFile() || fs.readFileSync(checksumPath, 'utf8') !== checksumText)) throw new Error('Package digest changed after native verification.');
  fs.writeFileSync(checksumPath, checksumText);
  return files.map(file => path.join(directory, file));
}

function metadata() {
  return JSON.parse(execFileSync('cargo', ['metadata', '--format-version=1', '--locked'], {encoding: 'utf8', maxBuffer: 64 * 1024 * 1024}));
}

function main() {
  const [command, ...args] = process.argv.slice(2);
  if (command === 'check' || command === 'plan') {
    const config = JSON.parse(fs.readFileSync('.github/release.json', 'utf8'));
    const graph = metadata();
    if (command === 'check') {
      const member = inspectConfig(config, graph);
      console.log(member ? 'Release configuration has a valid executable; qualification is checked only on release.' : 'Release configuration is valid but disabled: no desktop executable is configured.');
      return;
    }
    const tag = process.env.RELEASE_TAG ?? '';
    const plan = releasePlan(config, graph, tag, file => fs.existsSync(file) && fs.lstatSync(file).isFile());
    // Publishing is restricted to annotated tags from the protected default branch.
    const ref = `refs/tags/${tag}`;
    if (execFileSync('git', ['cat-file', '-t', ref], {encoding: 'utf8'}).trim() !== 'tag') throw new Error('Release tag must be annotated.');
    const taggedCommit = execFileSync('git', ['rev-parse', `${ref}^{commit}`], {encoding: 'utf8'}).trim();
    const head = execFileSync('git', ['rev-parse', 'HEAD'], {encoding: 'utf8'}).trim();
    if (taggedCommit !== head) throw new Error('Checkout does not match release tag.');
    execFileSync('git', ['merge-base', '--is-ancestor', head, 'origin/main']);
    if (!process.env.GITHUB_OUTPUT) throw new Error('Release plan requires GitHub Actions outputs.');
    fs.appendFileSync(process.env.GITHUB_OUTPUT, Object.entries({...plan, commit: head}).map(([key, value]) => `${key}=${value}`).join('\n') + '\n');
  } else if (command === 'verify') {
    const [directory, platform] = args;
    verifyDirectory(directory, platform, process.env.RELEASE_VERSION, process.env.RELEASE_COMMIT);
    console.log(`Verified ${platform} package receipt and computed SHA-256 digests.`);
  } else if (command === 'collect') {
    const [directory] = args;
    const files = Object.keys(platforms).flatMap(platform => verifyDirectory(path.join(directory, `release-${platform}`), platform, process.env.RELEASE_VERSION, process.env.RELEASE_COMMIT, true));
    if (new Set(files.map(file => path.basename(file))).size !== files.length) throw new Error('Release filenames must be unique across platforms.');
    // A line per validated path lets the publish job pass filenames as arguments, not shell code.
    fs.writeFileSync(path.join(directory, 'publish-files.txt'), files.join('\n') + '\n');
    const sums = Object.keys(platforms).map(platform => fs.readFileSync(path.join(directory, `release-${platform}`, 'SHA256SUMS'), 'utf8')).join('');
    fs.writeFileSync(path.join(directory, 'SHA256SUMS'), sums);
  } else throw new Error('Usage: node tools/release.mjs check|plan|verify <directory> <platform>|collect <directory>');
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { main(); } catch (error) { console.error(`Release error: ${error.message}`); process.exitCode = 1; }
}
