import {execFileSync} from 'node:child_process';

const corePackages = new Set([
  'orange-model',
  'orange-bundled',
  'orange-graph',
  'orange-types',
  'orange-compiler',
  'orange-runtime',
  'orange-contracts',
]);
// UI, window, renderer and engine frameworks that graph-core crates must never depend on
// (ADR-01, ADR-02, ADR-21). Host applications may use them.
const frameworkRoots = ['bevy', 'egui', 'eframe', 'epaint', 'winit', 'wgpu', 'slint', 'i-slint', 'iced'];
const isFrameworkPackage = name => frameworkRoots.some(root =>
  name === root || name.startsWith(`${root}_`) || name.startsWith(`${root}-`));

function loadMetadata() {
  try {
    const json = execFileSync('cargo', ['metadata', '--format-version=1', '--locked'], {
      encoding: 'utf8',
      maxBuffer: 64 * 1024 * 1024,
    });
    return JSON.parse(json);
  } catch (error) {
    throw new Error(`cargo metadata failed: ${error.message}`);
  }
}

function inspectMetadata(metadata) {
  if (!metadata || !Array.isArray(metadata.packages) || !Array.isArray(metadata.workspace_members) ||
      !metadata.workspace_members.length || !metadata.resolve || !Array.isArray(metadata.resolve.nodes)) {
    throw new Error('Cargo metadata is incomplete; refusing to pass the policy check.');
  }

  const packages = new Map(metadata.packages.map(pkg => [pkg.id, pkg]));
  const workspaceIds = new Set(metadata.workspace_members);
  const nodes = new Map(metadata.resolve.nodes.map(node => [node.id, node]));
  if ([...workspaceIds].some(id => !packages.has(id) || !nodes.has(id))) {
    throw new Error('Cargo metadata has a workspace member without package or resolve data.');
  }

  const errors = [];
  const members = [...workspaceIds].map(id => packages.get(id));
  if (!members.some(pkg => pkg.name === 'orange-model')) {
    throw new Error('Cargo metadata is missing the expected orange-model core root.');
  }
  for (const pkg of members) {
    for (const dependency of pkg.dependencies ?? []) {
      const dependencyName = dependency.package ?? dependency.name;
      if (corePackages.has(pkg.name) && isFrameworkPackage(dependencyName)) {
        errors.push(`${pkg.name} declares ${dependencyName}; graph core must stay free of UI and engine frameworks, including optional dependencies`);
      }
    }
  }

  const memberByName = new Map(members.map(pkg => [pkg.name, pkg]));
  for (const name of corePackages) {
    const root = memberByName.get(name);
    if (!root) continue;
    const visited = new Set();
    const pending = [root.id];
    while (pending.length) {
      const id = pending.pop();
      if (visited.has(id)) continue;
      visited.add(id);
      const pkg = packages.get(id);
      const node = nodes.get(id);
      if (!pkg || !node) throw new Error(`Cargo resolve graph is incomplete at ${id}; refusing to pass.`);
      if (isFrameworkPackage(pkg.name)) errors.push(`${name} has a transitive UI or engine dependency (${pkg.name} ${pkg.version})`);
      if (!Array.isArray(node.deps)) throw new Error(`Cargo resolve graph is missing dependencies for ${id}; refusing to pass.`);
      for (const dependency of node.deps) {
        if (!packages.has(dependency.pkg)) throw new Error(`Cargo resolve graph references unknown package ${dependency.pkg}.`);
        pending.push(dependency.pkg);
      }
    }
  }
  return errors;
}

function makePackage(id, name, version, dependencies = []) {
  return {id, name, version, dependencies};
}

function fixtureMetadata(packages, members, edges) {
  return {
    packages,
    workspace_members: members,
    resolve: {nodes: packages.map(pkg => ({
      id: pkg.id,
      deps: (edges[pkg.id] ?? []).map(target => ({pkg: target, dep_kinds: []})),
    }))},
  };
}

function selfTest() {
  const core = makePackage('model 0.0.0', 'orange-model', '0.0.0');
  const app = makePackage('orange-app 0.0.0', 'orange-app', '0.0.0', [{name: 'eframe', req: '=1.0.0'}]);
  const eframe = makePackage('eframe 1.0.0', 'eframe', '1.0.0');
  const transitive = makePackage('graphics-helper 1.0.0', 'graphics-helper', '1.0.0');
  const winit = makePackage('winit 1.0.0', 'winit', '1.0.0');
  const eguiExtras = makePackage('egui_extras 1.0.0', 'egui_extras', '1.0.0');
  const check = (label, metadata, expectedErrors) => {
    const actual = inspectMetadata(metadata);
    if (actual.length !== expectedErrors) throw new Error(`${label}: expected ${expectedErrors} errors, received ${actual.length}: ${actual.join('; ')}`);
  };
  check('host-only UI framework is allowed', fixtureMetadata([core, app, eframe], [core.id, app.id], {[app.id]: [eframe.id]}), 0);
  check('transitive core leak is rejected', fixtureMetadata([core, transitive, winit], [core.id], {[core.id]: [transitive.id], [transitive.id]: [winit.id]}), 1);
  check('framework subcrate leak is rejected', fixtureMetadata([core, eguiExtras], [core.id], {[core.id]: [eguiExtras.id]}), 1);
  check('optional core declaration is rejected', fixtureMetadata([makePackage('model 0.0.0', 'orange-model', '0.0.0', [{name: 'egui', req: '=1.0.0', optional: true}]), eframe], ['model 0.0.0'], {}), 1);
  const bundled = makePackage('bundled 0.0.0', 'orange-bundled', '0.0.0', [{name: 'egui', req: '=1.0.0'}]);
  check('bundled kernel UI dependency is rejected', fixtureMetadata([core, bundled, eframe], [core.id, bundled.id], {[bundled.id]: [eframe.id]}), 2);
  const mustFail = (label, metadata) => {
    let failed = false;
    try { inspectMetadata(metadata); } catch { failed = true; }
    if (!failed) throw new Error(`${label}: malformed metadata unexpectedly passed`);
  };
  mustFail('broken graph', {packages: [], workspace_members: ['missing'], resolve: {nodes: []}});
  mustFail('empty workspace', {packages: [], workspace_members: [], resolve: {nodes: []}});
  mustFail('missing orange-model root', fixtureMetadata([app], [app.id], {}));
  mustFail('missing resolve deps', {...fixtureMetadata([core], [core.id], {}), resolve: {nodes: [{id: core.id}]}});
  console.log('Cargo metadata policy fixtures passed (9 cases).');
}

if (process.argv.includes('--self-test')) {
  selfTest();
} else if (process.argv.includes('--msrv')) {
  let metadata;
  try { metadata = loadMetadata(); }
  catch (error) {
    console.error(`Cannot derive workspace MSRV. ${error.message}`);
    process.exit(1);
  }
  const memberIds = new Set(metadata.workspace_members);
  const versions = [...new Set(metadata.packages.filter(pkg => memberIds.has(pkg.id)).map(pkg => pkg.rust_version).filter(Boolean))];
  if (versions.length !== 1 || [...memberIds].some(id => !metadata.packages.find(pkg => pkg.id === id)?.rust_version)) {
    console.error('Workspace members must each declare one consistent rust-version.');
    process.exit(1);
  }
  console.log(`version=${versions[0]}`);
} else {
  let metadata;
  try { metadata = loadMetadata(); }
  catch (error) {
    console.error(`${error.message}; refusing to pass the dependency policy check.`);
    process.exit(1);
  }
  const errors = inspectMetadata(metadata);
  if (errors.length) {
    for (const error of errors) console.error(`FAIL: ${error}`);
    process.exit(1);
  }
  console.log('Cargo metadata policy passed; graph core is free of UI and engine frameworks.');
}
