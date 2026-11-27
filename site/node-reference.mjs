import {readFile} from 'node:fs/promises';
import {join} from 'node:path';
import {icon} from './icons.mjs';
import {locales} from './locales/index.mjs';

const graphPath = 'app/crates/apps/orange-cli/graphs/';
const exampleSlugs = [
  "counter-with-reset",
  "set-on-trigger",
  "idle-watchdog",
  "power-with-count",
  "running-total-with-reduce",
  "json-extraction",
  "http-with-failure-handling",
  "http-with-bounded-retry",
  "file-change-journal",
  "shared-score",
  "quiet-mic-gate"
];


export async function loadNodeExamples(root) {
  return Promise.all(exampleSlugs.map(async slug => ({
    slug, path: graphPath + slug + '.json',
    graph: JSON.parse(await readFile(join(root, graphPath, slug + '.json'), 'utf8')),
  })));
}

const portLabel = (port, language) => port.label ??
  locales[language].portLabels?.[port.id] ?? port.id.replaceAll('_', ' ');

export function renderCounterWire(example, catalogue, nodePath, escape, language) {
  const {nodes, edges} = example.graph;
  const trigger = nodes.find(node => node.name === 'Increment' && node.definition_id === 'core/on-manual');
  const edge = edges.find(edge => edge.from.node === trigger?.id && edge.from.port === 'then');
  if (!edge) throw new Error('Counter example is missing the Increment execution wire');
  const cards = [edge.from, edge.to].map((endpoint, index) => {
    const instance = nodes.find(node => node.id === endpoint.node);
    const definition = catalogue.find(node => node.id === instance.definition_id && node.version === instance.definition_version);
    const ports = language === 'es' ? (definition.preview_ports_es ?? definition.preview_ports) : definition.preview_ports;
    const port = ports.find(port => port.id === endpoint.port && port.input === Boolean(index));
    return '<div class="node-card"><strong>' + escape(instance.name) + '</strong><ul><li class="exec-port">' +
      '<a href="../' + nodePath(definition) + '#port-' + encodeURIComponent(port.id) + '">' +
      escape(definition.name[language]) + ' · ' + escape(portLabel(port, language)) + '</a></li></ul></div>';
  });
  return '<figure class="node-preview"><div class="node-stage graph-wire">' + cards.join('<span aria-hidden="true"></span>') +
    '</div><figcaption>' + (language === 'es' ? 'La salida de Increment activa Add one.' :
      'The execution wire runs from Increment to Add one.') + '</figcaption></figure>';
}
// What the site adds to a node page: a way to try it, ports that appear only after
// configuration, and internal ports to leave out. What a node and its ports do comes
// from the node catalogues (`purpose-*`, `<node key>-port-*`) through `orange nodes --json`,
// so the editor shows the same text.
const notes = {
  'core/get-variable': {
    tryIt: true,
    configured: [{id: 'value', input: false}],
  },
  'org.orange.collections/enum-payload': {
    tryIt: true,
  },
  'core/branch': {
    quickTry: true,
  },
  'core/bridge-in': {
    tryIt: true,
    hidden: ['value'],
  },
  'core/bridge-out': {
    tryIt: true,
    hidden: ['result'],
  },
  'core/coalesce': {
    tryIt: true,
  },
  'core/emit': {
    configured: [{id: 'payload', input: true}],
    tryIt: true,
  },
  'core/on-event': {
    configured: [{id: 'payload', input: false}],
    tryIt: true,
  },
  'org.orange.math/add': {
    tryIt: true,
  },
  'org.orange.math/multiply': {
    tryIt: true,
  },
};

// Catalogue help is keyed by port key: the port ID with underscores as hyphens.
const portKey = id => id.split('#')[0].toLowerCase().replace(/_/g, '-');

function portUse(port, schema, node, language) {
  const es = language === 'es';
  const specific = node.port_help?.[language]?.[portKey(port.id)] ?? port.description;
  if (specific) return specific;
  if (port.implicit_failure && port.id === 'error') return es ? 'Error disponible en la ruta Falló.' : 'Fault value available on the Failed route.';
  if (node.kind === 'entry' && port.execution && !port.input && port.id === 'then') return es ? 'Se ejecuta al activarse el disparador.' : 'Runs when the trigger fires.';
  if (port.execution) return locales[language].commonPorts[port.id] ??
    (port.input ? (es ? 'Recibe la ejecución.' : 'Execution enters the node here.') :
      (es ? 'Conecta el siguiente paso de esta ruta.' : 'Connect the next step for this outcome here.'));
  if (port.input) return port.optional ? (es ? 'Acepta un valor opcional.' : 'Accepts an optional value.') :
    (es ? 'Conecta un valor del tipo indicado.' : 'Wire a value of the shown type to this input.');
  if (schema?.source === 'iteration') return es ? 'Disponible en la iteración actual.' : 'Available to the current iteration.';
  if (schema?.source === 'state') return es ? 'Expone el estado actual del nodo.' : 'Exposes the current state of the node.';
  if (schema?.source === 'payload') return es ? 'Disponible como dato de la activación.' : 'Available as data from the activation.';
  return schema?.available_on?.length ? (es ? 'Disponible en la ruta de ejecución indicada.' : 'Available on the listed execution route.') :
    (es ? 'Conecta esta salida a una entrada compatible.' : 'Read this value from a compatible downstream input.');
}

function exampleLinks(matches, escape, language) {
  return matches.map(({slug, graph, nodeId}) => {
    const instance = graph.nodes.find(node => node.id === nodeId);
    return '<li><a href="../../../../examples/#' + slug + '">' + escape(locales[language].examples[slug][0]) + '</a>' +
      (instance?.name ? ' <span class="dim">· ' + escape(instance.name) + '</span>' : '') + '</li>';
  }).join('');
}

export function renderNodeReference(node, examples, escape, language, images) {
  const es = language === 'es';
  const note = notes[node.id] ?? {};
  const matches = examples.flatMap(example => {
    const instance = example.graph.nodes.find(item => item.definition_id === node.id && item.definition_version === node.version);
    return instance ? [{...example, nodeId: instance.id}] : [];
  });
  const schemaPorts = node.ports ?? [];
  const preview = es ? (node.preview_ports_es ?? node.preview_ports) : node.preview_ports;
  const projected = (preview ?? []).filter(port => !note.hidden?.includes(port.id));
  const projections = new Set(projected.map(port => port.id));
  const configured = (note.configured ?? []).filter(port => !projections.has(port.id))
    .map(port => ({...port, declaration: true, configured: true}));
  const ports = [...projected, ...schemaPorts.filter(port => !projections.has(port.port_id) && !note.hidden?.includes(port.port_id))
    .map(port => ({id: port.port_id, input: port.direction === 'input', execution: port.kind === 'exec', type: port.type, declaration: true})), ...configured];
  const label = port => portLabel(port, language);
  const direction = input => input ? (es ? 'Entrada' : 'Input') : (es ? 'Salida' : 'Output');
  const exec = es ? 'Ejecución' : 'Execution';
  const data = es ? 'Datos' : 'Data';
  const groups = [true, false].map(input => {
    const members = projected.filter(port => port.input === input);
    const heading = input ? (es ? 'Entradas' : 'Inputs') : (es ? 'Salidas' : 'Outputs');
    return '<div class="' + (input ? 'node-inputs' : 'node-outputs') + '"><strong>' + heading + '</strong><ul>' +
      (members.map(port => '<li class="' + (port.execution ? 'exec-port' : 'value-port') +
        '"><a href="#port-' + encodeURIComponent(port.id) + '">' + escape(label(port)) + '</a></li>').join('') ||
        '<li class="dim">—</li>') + '</ul></div>';
  });
  const diagram = '<div class="node-stage"><div class="node-card"><strong>' +
    escape(node.name[language]) + '</strong><div class="node-port-columns">' + groups.join('') + '</div></div></div>' +
    (projected.length ? '<ul class="port-legend"><li class="exec-port">' + exec + '</li><li class="value-port">' + data + '</li></ul>' : '');
  const figure = images
    ? '<figure class="node-preview native-preview">' + Object.entries(images).map(([theme, path]) =>
      '<img class="native-' + theme + '" src="' + escape(path) + '" alt="' +
      escape((es ? 'Tarjeta de ' : 'Node card for ') + node.name[language]) + '" decoding="async">').join('') +
      '<figcaption>' + (es ? 'Vista previa del catálogo del editor.' : 'Rendered by the editor’s node palette.') +
      '</figcaption></figure><details class="port-diagram"><summary>' +
      (es ? 'Diagrama de puertos con enlaces' : 'Linked port diagram') + '</summary>' + diagram + '</details>'
    : '<figure class="node-preview">' + diagram +
      '<figcaption>' + (projected.length
      ? (es ? 'Vista del nodo al colocarlo. Selecciona un puerto para leer su descripción.' :
        'The card as you place it on the canvas. Select a port for its description.')
      : (es ? 'Los puertos aparecen al configurar este nodo.' : 'Ports appear after you configure this node.')) + '</figcaption>' +
      '</figure>';
  const rows = ports.map(port => {
    const schema = schemaPorts.find(p => p.port_id === port.id.split('#')[0] && (p.direction === 'input') === port.input);
    const ty = port.type ?? schema?.type;
    const availability = schema?.available_on?.length ? ' <small>' + (es ? 'Ruta: ' : 'Outcome: ') + escape(schema.available_on.join(', ')) + '.</small>' : '';
    const value = port.default ?? schema?.default?.value;
    const fallback = value === undefined || value === null ? '' :
      ' <small>' + (es ? 'Valor inicial: ' : 'Default: ') + '<code>' + escape(JSON.stringify(value)) + '</code>.</small>';
    const modifiers = (schema?.lazy ? (es ? ' Se lee bajo demanda.' : ' Read on demand.') : '') +
      (schema?.variadic ? (es ? ' Puedes añadir más puertos de este grupo.' : ' Repeatable port family.') : '') +
      (port.implicit_failure ? (es ? ' Puede estar contraído en el lienzo.' : ' May be collapsed on the canvas.') : '') +
      (port.configured ? (es ? ' Aparece tras la configuración.' : ' Appears after configuration.') : '');
    return '<tr><th scope="row" id="port-' + escape(port.id) + '">' + escape(label(port)) +
      '<br><code>' + escape(port.id) + '</code></th><td>' + direction(port.input) +
      '</td><td>' + (port.execution ? exec : ty ? '<code>' + escape(typeof ty === 'string' ? ty : JSON.stringify(ty)) +
      '</code>' : (es ? 'Tipo inferido' : 'Inferred value')) + '</td><td>' + escape(portUse(port, schema, node, language) + modifiers) +
      availability + fallback + '</td></tr>';
  }).join('');
  const linked = exampleLinks(matches, escape, language);
  const quickTry = note.quickTry && locales[language].notes[node.id];
  const tryIt = note.tryIt && locales[language].notes[node.id];
  return figure + '<h2 id="behavior">' + (es ? 'Cómo funciona' : 'How it works') + '</h2>' +
    '<p>' + escape(locales[language].kinds[node.kind]) + '</p>' +
    (quickTry ? '<h2 id="try-it">' + (es ? 'Pruébalo' : 'Try it') + '</h2><p>' + escape(quickTry) + '</p>' : '') +
    '<h2 id="ports">' + (es ? 'Puertos' : 'Ports') + '</h2>' +
    (rows ? '<div class="table-scroll" tabindex="0" role="region" aria-labelledby="ports"><table><thead><tr>' +
      (es ? '<th>Puerto</th><th>Dirección</th><th>Tipo</th><th>Uso</th>' :
        '<th>Port</th><th>Direction</th><th>Type</th><th>Use</th>') + '</tr></thead><tbody>' +
      rows + '</tbody></table></div>' : '<p>' + (es ? 'Elige la declaración o el tipo en Propiedades para ver los puertos.' :
        'Choose the declaration or type in Properties to resolve the ports for this node.') + '</p>') +
    (tryIt || linked ? '<h2 id="examples">' + (es ? 'Ejemplos' : 'Examples') + '</h2>' +
      (tryIt ? '<p>' + escape(tryIt) + '</p>' : '') + (linked ? '<ul>' + linked + '</ul>' : '') : '');
}

export function renderGraphExamples(examples, nodes, source, escape, nodePath, language) {
  const es = language === 'es';
  const graphs = es ? '../../examples/graphs/' : 'graphs/';
  const byIdentity = new Map(nodes.map(node => [node.id + '@' + node.version, node]));
  return examples.map(({slug, path, graph}) => {
    const [title, instruction] = locales[language].examples[slug];
    const used = new Map();
    for (const instance of graph.nodes) {
      const identity = instance.definition_id + '@' + instance.definition_version;
      if (!used.has(identity) && byIdentity.has(identity)) used.set(identity, byIdentity.get(identity));
    }
    return '<h2 id="' + slug + '">' + escape(title) + '</h2><p>' + escape(instruction) + '</p>' +
      '<p class="example-actions"><button class="icon-action" type="button" data-clip="' + graphs + slug + '.clip.json" hidden>' +
      icon('copy') + '<span class="copy-label">' + (es ? 'Copiar nodos' : 'Copy nodes') + '</span></button> ' +
      '<a class="icon-action" href="' + graphs + slug + '.json" download>' + icon('download') + (es ? 'Descargar grafo JSON' : 'Download graph JSON') + '</a> ' +
      '<a class="icon-action" href="' + source + path + '">' + icon('external-link') + (es ? 'Ver código fuente' : 'View source') + '</a></p>' +
      '<p>' + (es ? 'Nodos de este grafo' : 'Nodes in this graph') + '</p><ul class="example-nodes">' + [...used.values()].map(node =>
        '<li><a href="../' + nodePath(node) + '">' + escape(node.name[language]) + '</a></li>').join('') + '</ul>';
  }).join('');
}
