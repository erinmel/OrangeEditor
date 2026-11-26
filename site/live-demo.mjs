import {execFileSync} from 'node:child_process';
import {mkdir} from 'node:fs/promises';
import {join} from 'node:path';

const crate = 'orange-docs-web';
export const demoAssets = ['orange_docs_web.js', 'orange_docs_web_bg.wasm'];

export async function compileDemo(root, target, bindgen) {
  const release = execFileSync(bindgen, ['--version'], {encoding: 'utf8'}).trim();
  if (release !== 'wasm-bindgen 0.2.129') throw new Error('Expected wasm-bindgen 0.2.129, got ' + release);

  const messages = execFileSync('cargo', [
    'build', '--release', '--locked', '--target', 'wasm32-unknown-unknown',
    '-p', crate, '--message-format=json-render-diagnostics',
  ], {cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'], maxBuffer: 16 * 1024 * 1024});
  const wasm = messages.trim().split('\n').map(line => JSON.parse(line))
    .filter(message => message.reason === 'compiler-artifact' && message.target.name === 'orange_docs_web')
    .flatMap(message => message.filenames).find(file => file.endsWith('.wasm'));
  if (!wasm) throw new Error('Cargo did not produce the documentation demo WASM.');

  const output = join(target, 'orange-docs-bindgen');
  await mkdir(output, {recursive: true});
  execFileSync(bindgen, ['--target', 'web', '--no-typescript', '--out-dir', output, wasm],
    {cwd: root, stdio: 'inherit'});
  return output;
}
