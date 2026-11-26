import {readFile} from 'node:fs/promises';
import {join} from 'node:path';

const graphPath = 'app/crates/apps/orange-cli/graphs/';
const examples = [
  ['set-on-trigger', 'Set a value on demand', 'Run the graph and fire On manual. Open Variables and confirm total is 7.'],
  ['idle-watchdog', 'Restart an idle countdown', 'Fire Heartbeat twice within 750 ms. The next Listen activation replaces the earlier countdown. Idle becomes true after the latest countdown ends.'],
  ['power-with-count', 'Compute a power using a loop', 'Run Calculate power. The example starts the result at 1, multiplies it by the base on each iteration, then publishes the final value.'],
  ['running-total-with-reduce', 'Accumulate a running total', 'Open the graph and follow the running state from Reduce begin through Add to Reduce end. Inspect the final sum.'],
  ['json-extraction', 'Read a field from JSON', 'Run the graph and fire its manual trigger. Follow the parsed document into JSON Read Text and inspect the variable it writes.'],
  ['http-with-failure-handling', 'Route the result of an HTTP request', 'Choose a reachable test endpoint in the connection. Fire the graph and compare an HTTP success status, an HTTP error status and a connection failure.'],
  ['quiet-mic-gate', 'Meter audio after a noise gate', 'Choose an input device and run the graph. Watch the gated level as the source becomes quiet. The example uses silent devices until you configure real ones.'],
];

export async function loadNodeExamples(root) {
  return Promise.all(examples.map(async ([slug, title, instruction]) => ({
    slug, title, instruction, path: graphPath + slug + '.json',
    graph: JSON.parse(await readFile(join(root, graphPath, slug + '.json'), 'utf8')),
  })));
}

const kinds = {
  entry: 'Starts an activation when its trigger fires. Connect an execution output to the first step.',
  pure: 'Produces data for downstream inputs. A downstream read asks for the result.',
  volatile: 'Supplies a value that can change across evaluation epochs.',
  action: 'Runs when execution reaches it and follows one of its outgoing routes.',
  async: 'Starts work when execution reaches it and resumes the flow on an outcome.',
  iterator: 'Supplies items one at a time when the loop requests the next item.',
};
const commonPorts = {
  in: 'Connect the execution wire that starts this step.',
  then: 'Continue along this execution route after the step completes.',
  failed: 'Follow this execution route when the operation faults.',
  cancelled: 'Follow this execution route when the work is cancelled.',
  body: 'Runs the loop body for the current iteration.',
  completed: 'Continues after the loop finishes.',
};
// What the site adds to a node page: a way to try it, ports that appear only after
// configuration, and internal ports to leave out. What a node and its ports do comes
// from the node catalogues (`purpose-*`, `<node key>-port-*`) through `orange nodes --json`,
// so the editor shows the same text.
const notes = {
  'core/get-variable': {
    tryIt: 'Create a variable and select it in Properties. The Value output takes the variable’s type.',
    configured: [{id: 'value', input: false}],
  },
  'org.orange.collections/enum-payload': {
    tryIt: 'Choose an enum and a variant that carries a payload. The enum and variant determine the port types.',
  },
  'core/branch': {
    tryIt: 'Wire a Boolean into Condition. Connect True and False to different steps, then fire the upstream trigger.',
  },
  'core/bridge-in': {
    tryIt: 'Choose a Bridge Out channel and connect Result to a value input. The consuming node pulls the shared value.',
    hidden: ['value'],
  },
  'core/bridge-out': {
    tryIt: 'Connect a source to Value, name the channel, then choose it on one or more Bridge In cards.',
    hidden: ['result'],
  },
  'core/coalesce': {
    tryIt: 'Feed a missing optional number and set Fallback to 10. The result is 10. Supply an optional 4 and the result becomes 4.',
  },
  'core/emit': {
    configured: [{id: 'payload', input: true}],
    tryIt: 'Select a typed event to add its Payload input. Then choose the same event on a Listen card and fire Dispatch.',
  },
  'core/on-event': {
    configured: [{id: 'payload', input: false}],
    tryIt: 'Choose the event used by Dispatch. When that event is emitted, this card starts a fresh activation.',
  },
  'org.orange.math/add': {
    tryIt: 'Set A to 2 and B to 3 using the same number type. A downstream read of Sum yields 5.',
  },
  'org.orange.math/multiply': {
    tryIt: 'Set A to 3 and B to 4 using the same number type. A downstream read of Product yields 12.',
  },
};

// Catalogue help is keyed by port key: the port ID with underscores as hyphens.
const portKey = id => id.split('#')[0].toLowerCase().replace(/_/g, '-');

function portUse(port, schema, node) {
  const specific = port.description ?? node.port_help?.en?.[portKey(port.id)];
  if (specific) return specific;
  if (port.implicit_failure && port.id === 'error') return 'Fault value available on the Failed route.';
  if (node.kind === 'entry' && port.execution && !port.input && port.id === 'then') return 'Runs when the trigger fires.';
  if (port.execution) return commonPorts[port.id] ??
    (port.input ? 'Execution enters the node here.' : 'Connect the next step for this outcome here.');
  if (port.input) return port.optional ? 'Accepts an optional value.' : 'Wire a value of the shown type to this input.';
  if (schema?.source === 'iteration') return 'Available to the current iteration.';
  if (schema?.source === 'state') return 'Exposes the current state of the node.';
  if (schema?.source === 'payload') return 'Available as data from the activation.';
  return schema?.available_on?.length ? 'Available on the listed execution route.' : 'Read this value from a compatible downstream input.';
}

function exampleLinks(matches, escape) {
  return matches.map(({slug, title, graph, nodeId}) => {
    const instance = graph.nodes.find(node => node.id === nodeId);
    return '<li><a href="../../../../examples/#' + slug + '">' + escape(title) + '</a>' +
      (instance?.name ? ' <span class="dim">· ' + escape(instance.name) + '</span>' : '') + '</li>';
  }).join('');
}

export function renderNodeReference(node, examples, escape) {
  const note = notes[node.id] ?? {};
  const matches = examples.flatMap(example => {
    const instance = example.graph.nodes.find(item => item.definition_id === node.id && item.definition_version === node.version);
    return instance ? [{...example, nodeId: instance.id}] : [];
  });
  const schemaPorts = node.ports ?? [];
  const projected = (node.preview_ports ?? []).filter(port => !note.hidden?.includes(port.id));
  const projections = new Set(projected.map(port => port.id));
  const configured = (note.configured ?? []).filter(port => !projections.has(port.id))
    .map(port => ({...port, declaration: true, configured: true}));
  const ports = [...projected, ...schemaPorts.filter(port => !projections.has(port.port_id) && !note.hidden?.includes(port.port_id))
    .map(port => ({id: port.port_id, input: port.direction === 'input', execution: port.kind === 'exec', type: port.type, declaration: true})), ...configured];
  const groups = [true, false].map(input => '<div><strong>' + (input ? 'Inputs' : 'Outputs') + '</strong><ul>' +
    (ports.filter(port => port.input === input && !port.declaration).map(port =>
      '<li class="' + (port.execution ? 'exec-port' : 'value-port') + '">' +
      '<a href="#port-' + encodeURIComponent(port.id) + '">' + escape(port.label ?? port.id) + '</a></li>').join('') ||
      '<li class="dim">None in this view</li>') + '</ul></div>');
  const figure = '<figure class="node-preview"><div class="node-card"><strong>' + escape(node.name.en) +
    '</strong><div class="node-port-columns">' + groups.join('') + '</div></div>' +
    '<figcaption>' + (node.preview_ports ? 'Fresh authoring card. Configuration and types can change the available ports.' :
      'Port layout depends on this node’s configuration.') + '</figcaption></figure>';
  const rows = ports.map(port => {
    const schema = schemaPorts.find(p => p.port_id === port.id.split('#')[0] && (p.direction === 'input') === port.input);
    const ty = port.type ?? schema?.type;
    const availability = schema?.available_on?.length ? ' <small>Outcome: ' + escape(schema.available_on.join(', ')) + '.</small>' : '';
    const value = port.default ?? schema?.default?.value;
    const fallback = value === undefined || value === null ? '' :
      ' <small>Default: <code>' + escape(JSON.stringify(value)) + '</code>.</small>';
    const modifiers = (schema?.lazy ? ' Read on demand.' : '') + (schema?.variadic ? ' Repeatable port family.' : '') +
      (port.implicit_failure ? ' May be collapsed on the canvas.' : '') +
      (port.configured ? ' Appears after configuration.' : '');
    return '<tr><th scope="row" id="port-' + escape(port.id) + '">' + escape(port.label ?? port.id) +
      '<br><code>' + escape(port.id) + '</code></th><td>' + (port.input ? 'Input' : 'Output') +
      '</td><td>' + (port.execution ? 'Execution' : ty ? '<code>' + escape(typeof ty === 'string' ? ty : JSON.stringify(ty)) +
      '</code>' : 'Inferred value') + '</td><td>' + escape(portUse(port, schema, node) + modifiers) +
      availability + fallback + '</td></tr>';
  }).join('');
  const linked = exampleLinks(matches, escape);
  return '<h2 id="behavior">How it works</h2>' +
    '<p>' + escape(kinds[node.kind]) + '</p>' + figure +
    '<h2 id="ports">Ports</h2>' +
    (rows ? '<div class="table-scroll"><table><thead><tr><th>Port</th><th>Direction</th><th>Type</th><th>Use</th></tr></thead><tbody>' +
      rows + '</tbody></table></div>' : '<p>Choose the declaration or type in Properties to resolve the ports for this node.</p>') +
    (note.tryIt || linked ? '<h2 id="examples">Examples</h2>' +
      (note.tryIt ? '<p>' + escape(note.tryIt) + '</p>' : '') + (linked ? '<ul>' + linked + '</ul>' : '') : '');
}

export function renderGraphExamples(examples, nodes, source, escape, nodePath) {
  const byIdentity = new Map(nodes.map(node => [node.id + '@' + node.version, node]));
  return examples.map(({slug, title, instruction, path, graph}) => {
    const used = new Map();
    for (const instance of graph.nodes) {
      const identity = instance.definition_id + '@' + instance.definition_version;
      if (!used.has(identity) && byIdentity.has(identity)) used.set(identity, byIdentity.get(identity));
    }
    return '<h2 id="' + slug + '">' + escape(title) + '</h2><p>' + escape(instruction) + '</p>' +
      '<p><a href="graphs/' + slug + '.json" download>Download graph JSON</a> · <a href="' + source + path + '">View source</a></p>' +
      '<p>Nodes in this graph</p><ul class="example-nodes">' + [...used.values()].map(node =>
        '<li><a href="../' + nodePath(node) + '">' + escape(node.name.en) + '</a></li>').join('') + '</ul>';
  }).join('');
}
