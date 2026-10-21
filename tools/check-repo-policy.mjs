import fs from 'node:fs';
import {execFileSync} from 'node:child_process';

// Inspect tracked content without printing credential matches or file contents.
const files = execFileSync('git', ['ls-files', '-z'], {encoding: 'utf8'}).split('\0').filter(Boolean);
const errors = [];
const patterns = [
  /-----BEGIN(?: RSA| EC| OPENSSH)? PRIVATE KEY-----/,
  /\bgh[pousr]_[A-Za-z0-9]{30,}\b/,
  /\bgithub_pat_[A-Za-z0-9_]{60,}\b/,
  /\bAKIA[0-9A-Z]{16}\b/,
];
for (const file of files) {
  if (/(^|\/)(\.idea|\.vscode|\.artifacts|target|node_modules|dist)\//.test(file)) errors.push(file + ': local/generated content');
  if (/(^|\/)\.env(?:\..+)?$/.test(file) && !/(^|\/)\.env\.example$/.test(file)) errors.push(file + ': local environment file');
  if (/\.(key|p12|pfx)$/i.test(file)) errors.push(file + ': credential/key file');
  const bytes = fs.readFileSync(file);
  if (!bytes.includes(0) && patterns.some(pattern => pattern.test(bytes.toString('utf8')))) errors.push(file + ': potential credential material; inspect locally');
}
if (!files.length) errors.push('No tracked files; stage the reviewed baseline first');
let hasCommit = true;
try { execFileSync('git', ['rev-parse', '--verify', 'HEAD'], {stdio: 'pipe'}); }
catch { hasCommit = false; }
try {
  execFileSync('git', hasCommit ? ['show', '--format=', '--check', 'HEAD'] : ['diff', '--cached', '--check'], {stdio: 'pipe'});
} catch { errors.push('Git whitespace validation failed'); }
console.log(JSON.stringify({status: errors.length ? 'FAIL' : 'PASS', trackedFiles: files.length,
  checks: ['tracked file policy', 'high-confidence credential patterns', 'Git whitespace'],
  limitations: 'Pattern scanning is not a complete secret audit; this script does not run Rust or product tests, which CI reports separately.', errors}, null, 2));
if (errors.length) process.exitCode = 1;
