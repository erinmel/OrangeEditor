// Every guide page, in navigation order, grouped the way readers look for help
// (Diátaxis): learn by doing, get a task done, understand why, or look something up.
// A page's source is site/guides/<slug>.md when it exists, else site/pages/<slug>.html.
// Fields: slug, title, description, section, and an optional shorter navigation label.
export const sections = [
  ['start', 'Start here'],
  ['tutorials', 'Tutorials'],
  ['how-to', 'How-to guides'],
  ['concepts', 'Concepts'],
  ['reference', 'Reference'],
];

export const pages = [
  ['', 'Overview', 'A typed node graph foundation with a Rust host and egui editor.', 'start'],
  ['getting-started', 'Getting started', 'Open the desktop editor, run your first graph, and try the terminal runner.', 'start'],

  ['count-button-presses', 'Count button presses', 'Build a graph from an empty canvas: triggers, a variable, and actions that change it.', 'tutorials'],

  ['editor', 'Use the editor', 'Place and wire typed nodes, configure variables and connections, and debug a flow.', 'how-to'],
  ['call-an-http-api', 'Call an HTTP API', 'Send a request through a connection and handle statuses, failures, and retries.', 'how-to'],
  ['react-to-file-changes', 'React to file changes', 'Watch a granted folder and act on what was created, changed, deleted, or renamed.', 'how-to'],
  ['share-between-graphs', 'Share values and events between graphs', 'Run several graphs at once and let them share variables and events.', 'how-to', 'Share between graphs'],
  ['custom-nodes', 'Make custom nodes', 'Extract graph functions, implement Wasm nodes, or register trusted Rust services.', 'how-to'],
  ['packages', 'Create packages', 'Export graph functions and pack, install, and update .orangeplug archives.', 'how-to'],
  ['embedding', 'Embed the runtime', 'Use orange-host to validate, execute, and observe graphs in a Rust application.', 'how-to'],
  ['egui', 'Embed and customize the egui editor', 'Compose OrangeEditor panels, themes, icons, and text in your existing egui app.', 'how-to', 'Embed the editor'],

  ['data-and-execution', 'Data wires and execution wires', 'Why a graph has two kinds of wires, and when each one runs.', 'concepts', 'Data and execution'],
  ['types-and-conversions', 'Types and conversions', 'Why every value has an exact type, and why changing one is a visible node.', 'concepts'],

  ['nodes', 'Node reference', 'Find nodes by English, Spanish or Japanese name, stable ID, provider, or port.', 'reference', 'Nodes'],
  ['examples', 'Example graphs', 'Run real graphs and follow the nodes that make them work.', 'reference'],
  ['controls', 'Controls reference', 'Choose, style, and use the shared OrangeEditor egui controls.', 'reference', 'UI controls'],
  ['sdk', 'SDK reference', 'Rustdoc, Wasm guest bindings, the graph language, and versioned package contracts.', 'reference', 'SDK and API'],
];
