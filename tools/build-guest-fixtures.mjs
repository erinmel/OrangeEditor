// Builds the probe guest (sdk/guests/probe) for wasm32-unknown-unknown and wraps it as a
// component. Default output is the committed host test fixture; `--out <path>` writes
// elsewhere, as CI does before running the host tests against the fresh build.
import {execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import path from 'node:path';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const sdk = path.join(root, 'sdk');
const args = process.argv.slice(2);
const outIndex = args.indexOf('--out');
const out = outIndex >= 0 && args[outIndex + 1]
  ? path.resolve(args[outIndex + 1])
  : path.join(root, 'app', 'crates', 'orange-guest', 'tests', 'fixtures', 'probe.wasm');

const run = (...cargoArgs) => execFileSync('cargo', cargoArgs, {cwd: sdk, stdio: 'inherit'});
run('build', '--release', '--locked', '-p', 'orange-probe-guest', '--target', 'wasm32-unknown-unknown');
// Ask Cargo where it builds: .cargo/config.toml can move the target directory.
const metadata = JSON.parse(execFileSync('cargo', ['metadata', '--format-version', '1', '--no-deps', '--locked'], {cwd: sdk, encoding: 'utf8'}));
const core = path.join(metadata.target_directory, 'wasm32-unknown-unknown', 'release', 'orange_probe_guest.wasm');
run('run', '--release', '--locked', '-q', '-p', 'orange-componentize', '--', core, out);
console.log(`Probe component written to ${out}`);
