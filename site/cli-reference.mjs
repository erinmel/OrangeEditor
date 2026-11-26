import {spawnSync} from 'node:child_process';

// The `orange` usage text, read from the program at build time so the page always
// matches the commands it really accepts.
export function loadCliUsage(root) {
  const run = spawnSync('cargo', ['run', '-q', '-p', 'orange-cli', '--locked', '--', 'help'],
    {cwd: root, encoding: 'utf8', maxBuffer: 4 * 1024 * 1024});
  const text = (run.stdout + run.stderr).replace(/\r\n/g, '\n');
  const start = text.indexOf('usage:');
  if (start < 0) throw new Error('orange printed no usage text');
  return text.slice(start).trim();
}

// Blank lines separate blocks. A block whose lines are indented, or the usage
// synopsis, is shown as code; any other block is a paragraph with `code` spans.
export function renderCliReference(usage, escape) {
  const inline = text => escape(text).replace(/`([^`]+)`/g, '<code>$1</code>');
  const blocks = usage.split(/\n\s*\n/);
  let synopsis = true;
  return '<p>The <code>orange</code> command checks, runs and inspects graphs and manages ' +
    'packages from a terminal. Build and run it from the repository root with ' +
    '<code>cargo run -p orange-cli --locked -- &lt;command&gt;</code>.</p>' +
    blocks.map(block => {
      const lines = block.split('\n');
      const code = synopsis || lines.every(line => /^\s{2,}/.test(line));
      if (synopsis) {
        synopsis = false;
        return '<h2 id="commands">Commands</h2><pre><code>' +
          escape(lines.filter(line => line !== 'usage:').map(line => line.replace(/^ {2}/, '')).join('\n')) +
          '</code></pre><h2 id="details">Details</h2>';
      }
      return code
        ? '<pre><code>' + escape(lines.map(line => line.replace(/^ {2}/, '')).join('\n')) + '</code></pre>'
        : '<p>' + inline(lines.join(' ')) + '</p>';
    }).join('');
}
