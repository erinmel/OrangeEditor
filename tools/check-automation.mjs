#!/usr/bin/env node
// Run the same locked acceptance checks from CI or a local checkout.
import {spawnSync} from 'node:child_process';
import {createConnection, createServer} from 'node:net';
import {fileURLToPath} from 'node:url';

const help = 'Usage: node tools/check-automation.mjs [--offline] [--network]';
const options = new Set(process.argv.slice(2));
if (options.has('--help')) {
  console.log(help);
  process.exit(0);
}
for (const option of options) {
  if (option !== '--offline' && option !== '--network') {
    console.error(`Unknown option ${option}. ${help}`);
    process.exit(2);
  }
}

const root = fileURLToPath(new URL('../', import.meta.url));
const base = ['test', '--locked', ...(options.has('--offline') ? ['--offline'] : [])];
const results = [];

function check(name, args, testArgs = []) {
  console.log(`\n=== ${name} ===`);
  const result = spawnSync('cargo', [...base, ...args, '--', ...testArgs], {
    cwd: root,
    stdio: 'inherit',
  });
  if (result.error) console.error(result.error.message);
  results.push([name, result.status === 0 ? 'PASS' : 'FAIL']);
}

async function checkLoopback() {
  const listener = createServer(socket => socket.end());
  try {
    await new Promise((resolve, reject) => {
      listener.once('error', reject);
      listener.listen(0, '127.0.0.1', resolve);
    });
    await new Promise((resolve, reject) => {
      const client = createConnection(listener.address().port, '127.0.0.1', () => {
        client.end();
        resolve();
      });
      client.on('error', reject);
      client.setTimeout(2000, () => client.destroy(new Error('loopback connection timed out')));
    });
  } finally {
    if (listener.listening) listener.close();
  }
}

check('Core graph, desktop, host, files and audio', [
  '--no-fail-fast', '-p', 'orange-model', '-p', 'orange-bundled', '-p', 'orange-compiler',
  '-p', 'orange-runtime', '-p', 'orange-host', '-p', 'orange-desktop', '-p', 'orange-editor-ui',
  '-p', 'orange-files', '-p', 'orange-audio',
], [
  '--skip', 'automation::desktop_http_request_sends_graph_built_json_and_reports_the_socket_response',
  '--skip', 'remote_file_plays_before_http_response_finishes',
]);
check('Network codec, validation and in-memory HTTP', ['-p', 'orange-net', '--lib']);
check('Saved connection graph and failure branches', ['-p', 'orange-net', '--test', 'connection_smoke'],
  ['saved_connection_graph_runs_json_feedback_and_failure_routes_without_a_server', '--exact']);
check('Saved Docker lab graph and authentication profiles', ['-p', 'orange-net', '--test', 'docker_network']);
check('Docker HTTP and WebSocket mock logic', [
  '--manifest-path', 'tools/network-lab/web/Cargo.toml', '--target-dir', 'target/network-lab-web',
]);

if (options.has('--network')) {
  let reachable = true;
  try {
    await checkLoopback();
  } catch (error) {
    console.error(`IPv4 loopback connection failed: ${error.message}`);
    results.push(['TCP integration fixtures', 'BLOCKED']);
    reachable = false;
  }
  if (reachable) {
    check('HTTP, WebSocket, MQTT and AMQP socket fixtures', ['--no-fail-fast', '-p', 'orange-net']);
    check('Saved graph over real HTTP and WebSocket sockets', ['-p', 'orange-net', '--test', 'connection_smoke'],
      ['the_same_graph_parses_http_and_websocket_responses', '--exact']);
    check('Desktop editor HTTP request and result', ['-p', 'orange-desktop', '--bin', 'orange-editor'],
      ['automation::desktop_http_request_sends_graph_built_json_and_reports_the_socket_response', '--exact']);
    check('Stream audio from a local HTTP server', ['-p', 'orange-audio', '--test', 'audio'],
      ['remote_file_plays_before_http_response_finishes', '--exact']);
  }
} else {
  results.push(['TCP integration fixtures', 'SKIPPED (enable with --network)']);
}
results.push(['Public HTTPS, external brokers and physical devices', 'SEPARATE CHECKS']);

console.log('\nAutomation results');
for (const [name, status] of results) {
  console.log(`  ${status.padEnd(30)} ${name}`);
}
process.exitCode = results.some(([, status]) => status === 'FAIL' || status === 'BLOCKED') ? 1 : 0;

