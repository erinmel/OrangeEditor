// Independent reference encoder for the canonical .orangeplug archive and content digest
// (docs/04 "Package format and SDK"). It shares no code with the Rust reader in
// orange-packages; both must reproduce app/crates/orange-packages/tests/fixtures/archive-v1.json.
//
//   node tools/package-archive.mjs   prints the vector set as JSON
import {createHash} from 'node:crypto';
import {pathToFileURL} from 'node:url';

const MAGIC = Buffer.from([0x89, 0x4f, 0x50, 0x4b, 0x47, 0x0d, 0x0a, 0x1a]);
const DOMAIN = Buffer.from('orange.plugin/content/v1\0', 'latin1');

const sha256 = bytes => createHash('sha256').update(bytes).digest();
const u16 = n => { const b = Buffer.alloc(2); b.writeUInt16LE(n); return b; };
const u32 = n => { const b = Buffer.alloc(4); b.writeUInt32LE(n); return b; };
const u64 = n => { const b = Buffer.alloc(8); b.writeBigUInt64LE(BigInt(n)); return b; };

// RFC 8785 for integer-only JSON: JS sorts strings by UTF-16 code units and JSON.stringify
// escapes exactly what JCS escapes.
export function canonical(value) {
  if (Array.isArray(value)) return `[${value.map(canonical).join(',')}]`;
  if (value !== null && typeof value === 'object') {
    return `{${Object.keys(value).sort().map(k => `${JSON.stringify(k)}:${canonical(value[k])}`).join(',')}}`;
  }
  if (typeof value === 'number' && !Number.isSafeInteger(value)) throw new Error('non-integer');
  return JSON.stringify(value);
}

export function canonicalBody(manifestText) {
  const {integrity: _ignored, ...rest} = JSON.parse(manifestText);
  return Buffer.from(canonical(rest), 'utf8');
}

const byPath = (a, b) => Buffer.compare(Buffer.from(a.path), Buffer.from(b.path));

export function contentDigest(manifestText, files) {
  const body = canonicalBody(manifestText);
  const parts = [DOMAIN, u64(body.length), body, u32(files.length)];
  for (const {path, bytes} of [...files].sort(byPath)) {
    parts.push(u16(Buffer.byteLength(path)), Buffer.from(path), u64(bytes.length), sha256(bytes));
  }
  return `sha256:${sha256(Buffer.concat(parts)).toString('hex')}`;
}

export function encode(manifestText, files) {
  const entry = (path, bytes) => [u16(Buffer.byteLength(path)), Buffer.from(path), Buffer.from([0, 0]), u64(bytes.length), bytes];
  const parts = [MAGIC, u16(1), u16(0), u32(files.length + 1), ...entry('manifest.json', Buffer.from(manifestText, 'utf8'))];
  for (const {path, bytes} of [...files].sort(byPath)) parts.push(...entry(path, bytes));
  return Buffer.concat(parts);
}

const PLACEHOLDER = '0'.repeat(64);

// The vector package: unsorted keys, non-ASCII and control-character text, nested opaque
// permissions, a component, two catalogues and an empty file.
export function vectors() {
  const draft = `{
  "format": "orange.plugin", "format_version": 1,
  "id": "org.example.vector", "version": "1.0.0-rc.1", "publisher": "org.example",
  "host_api": {"min_minor": 0, "major": 1, "max_minor": 2},
  "execution": {"world": "orange:expansion/plugin@1.0.0", "kind": "wasm-component", "entry": "component.wasm"},
  "localization": {"format_version": 1, "format": "fluent", "default_locale": "en",
    "catalogs": {"es": ["locales/es.ftl"], "en": ["locales/en.ftl"]}, "arguments": {"count-label": {"count": "u64"}}},
  "dependencies": [{"id": "org.example.base", "version": "2.0.0", "content_digest": "sha256:${'ab'.repeat(32)}"}],
  "contributions": {"nodes": ["org.example.vector/count"], "panels": ["org.example.vector/main"]},
  "permissions": [{"z": [3, -1], "a": "caf\\u00e9 \\ud83d\\ude00 \\u001f\\t\\"q\\""}],
  "integrity": {"algorithm": "sha256", "content_digest": "${PLACEHOLDER}"},
  "license": "MIT"
}
`;
  const files = [
    {path: 'locales/es.ftl', bytes: Buffer.from('count-label = { $count } nodos\n', 'utf8')},
    {path: 'component.wasm', bytes: Buffer.from([0x00, 0x61, 0x73, 0x6d, 0x0d, 0x00, 0x01, 0x00])},
    {path: 'z/empty.bin', bytes: Buffer.alloc(0)},
    {path: 'locales/en.ftl', bytes: Buffer.from('count-label = { $count } nodes\n', 'utf8')},
  ];
  const digest = contentDigest(draft, files);
  const manifest = draft.replace(PLACEHOLDER, digest.slice('sha256:'.length));
  const archive = encode(manifest, files);
  return {
    manifest,
    files: files.map(({path, bytes}) => ({path, hex: bytes.toString('hex')})),
    canonical_body: canonicalBody(manifest).toString('utf8'),
    content_digest: digest,
    archive_hex: archive.toString('hex'),
    archive_digest: `sha256:${sha256(archive).toString('hex')}`,
  };
}

if (import.meta.url === `file:///${process.argv[1]?.replaceAll('\\', '/').replace(/^\//, '')}`) {
  process.stdout.write(`${JSON.stringify(vectors(), null, 2)}\n`);
}
