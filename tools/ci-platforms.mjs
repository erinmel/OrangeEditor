import {pathToFileURL} from 'node:url';

// Native tests: each host triple is asserted against rustc before running Cargo.
export function platforms(extended) {
  if (typeof extended !== 'boolean') throw new TypeError('extended must be a boolean');
  const include = [
    {label: 'Linux x64', os: 'ubuntu-24.04', host: 'x86_64-unknown-linux-gnu'},
    {label: 'Windows x64', os: 'windows-2022', host: 'x86_64-pc-windows-msvc'},
    {label: 'macOS ARM64', os: 'macos-15', host: 'aarch64-apple-darwin'},
  ];
  if (extended) include.push(
    {label: 'Linux ARM64', os: 'ubuntu-24.04-arm', host: 'aarch64-unknown-linux-gnu'},
    {label: 'macOS x64', os: 'macos-15-intel', host: 'x86_64-apple-darwin'},
  );
  return {include};
}

// An omitted value means ordinary PR coverage. Reject malformed dispatch/environment data.
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const mode = process.env.EXTENDED ?? 'false';
  if (!['true', 'false'].includes(mode)) throw new Error('EXTENDED must be true or false');
  console.log(JSON.stringify(platforms(mode === 'true')));
}
