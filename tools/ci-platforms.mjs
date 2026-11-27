import {pathToFileURL} from 'node:url';

// Native tests: each host triple is asserted against rustc before running Cargo.
// Extended runs (weekly and release validation) test the same hosts; Intel Macs were
// dropped on 2026-11-27, as no release builds for them.
export function platforms(extended) {
  if (typeof extended !== 'boolean') throw new TypeError('extended must be a boolean');
  const include = [
    {label: 'Linux x64', os: 'ubuntu-24.04', host: 'x86_64-unknown-linux-gnu'},
    {label: 'Windows x64', os: 'windows-2022', host: 'x86_64-pc-windows-msvc'},
    {label: 'macOS ARM64', os: 'macos-15', host: 'aarch64-apple-darwin'},
    // ARM laptops, servers and boards. Wasmtime itself does not test Windows ARM64
    // (its tier 3), so this run is the evidence for it.
    {label: 'Linux ARM64', os: 'ubuntu-24.04-arm', host: 'aarch64-unknown-linux-gnu'},
    {label: 'Windows ARM64', os: 'windows-11-arm', host: 'aarch64-pc-windows-msvc'},
  ];
  return {include};
}

// An omitted value means ordinary PR coverage. Reject malformed dispatch/environment data.
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const mode = process.env.EXTENDED ?? 'false';
  if (!['true', 'false'].includes(mode)) throw new Error('EXTENDED must be true or false');
  console.log(JSON.stringify(platforms(mode === 'true')));
}
