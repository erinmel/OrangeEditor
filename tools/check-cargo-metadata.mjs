import {execFileSync} from 'node:child_process';

// Every workspace member, whether it is graph core, and the first-party crates it may depend
// on: `normal` for product builds, `dev` additionally for its own tests and examples
// (ADR-21). A new member fails the check until it is classified here.
const policy = {
  'orange-model': {core: true, normal: []},
  'orange-compiler': {core: true, normal: ['orange-model']},
  'orange-runtime': {core: true, normal: ['orange-model', 'orange-compiler']},
  'orange-bundled': {core: true, normal: ['orange-model', 'orange-compiler'], dev: ['orange-runtime']},
  // Development measurement binary; never part of an embedding host's dependency graph.
  'orange-measure': {core: false, normal: ['orange-model', 'orange-compiler', 'orange-runtime', 'orange-bundled']},
};
// UI, window, renderer and engine frameworks that graph-core crates must never depend on
// (ADR-01, ADR-02, ADR-21). Host applications may use them.
const frameworkRoots = ['bevy', 'egui', 'eframe', 'epaint', 'winit', 'wgpu', 'slint', 'i-slint', 'iced'];
// Concrete network, async-runtime, TLS, audio and OS-service crates. Hosts supply services
// behind provider and service ports; graph core builds without them. Cargo metadata cannot
// see `std::fs` or other standard-library I/O, which code review covers.
const serviceRoots = [
  'tokio', 'async-std', 'smol', 'mio', 'hyper', 'reqwest', 'ureq', 'h2', 'rustls', 'native-tls',
  'openssl', 'tungstenite', 'rumqttc', 'cpal', 'rodio', 'alsa', 'coreaudio-rs', 'wasapi', 'keyring', 'notify',
];
const matchesRoot = roots => name => roots.some(root =>
  name === root || name.startsWith(`${root}_`) || name.startsWith(`${root}-`));
const isFrameworkPackage = matchesRoot(frameworkRoots);
const isServicePackage = matchesRoot(serviceRoots);
// A resolve edge only for tests and examples; core product builds never include it.
const isDevOnly = dependency => Array.isArray(dependency.dep_kinds) && dependency.dep_kinds.length > 0 &&
  dependency.dep_kinds.every(kind => kind.kind === 'dev');

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

function inspectMetadata(metadata, memberPolicy = policy) {
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
  const isCore = name => memberPolicy[name]?.core === true;
  for (const pkg of members) {
    const rule = memberPolicy[pkg.name];
    if (!rule) {
      errors.push(`${pkg.name} is not classified in the dependency policy; add it as core or host before use`);
      continue;
    }
    for (const dependency of pkg.dependencies ?? []) {
      const dependencyName = dependency.package ?? dependency.name;
      if (rule.core && isFrameworkPackage(dependencyName)) {
        errors.push(`${pkg.name} declares ${dependencyName}; graph core must stay free of UI and engine frameworks, including optional dependencies`);
      }
      if (!(dependencyName in memberPolicy)) continue;
      const allowed = dependency.kind === 'dev'
        ? [...rule.normal, ...(rule.dev ?? [])]
        : rule.normal;
      if (!allowed.includes(dependencyName)) {
        const kind = dependency.kind === 'dev' ? 'development' : 'normal';
        errors.push(`${pkg.name} has a ${kind} dependency on ${dependencyName}, which the first-party dependency matrix does not allow`);
      }
    }
  }

  const memberByName = new Map(members.map(pkg => [pkg.name, pkg]));
  // Product builds of core crates: normal and build edges only, transitively.
  for (const [name, root] of memberByName) {
    if (!isCore(name)) continue;
    const visited = new Set();
    const pending = [root.id];
    while (pending.length) {
      const id = pending.pop();
      if (visited.has(id)) continue;
      visited.add(id);
      const pkg = packages.get(id);
      const node = nodes.get(id);
      if (!pkg || !node) throw new Error(`Cargo resolve graph is incomplete at ${id}; refusing to pass.`);
      if (isServicePackage(pkg.name)) errors.push(`${name} has a transitive network, audio or OS-service dependency (${pkg.name} ${pkg.version})`);
      if (!Array.isArray(node.deps)) throw new Error(`Cargo resolve graph is missing dependencies for ${id}; refusing to pass.`);
      for (const dependency of node.deps) {
        if (!packages.has(dependency.pkg)) throw new Error(`Cargo resolve graph references unknown package ${dependency.pkg}.`);
        if (!isDevOnly(dependency)) pending.push(dependency.pkg);
      }
    }
  }

  for (const [name, root] of memberByName) {
    if (!isCore(name)) continue;
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

// `edges` maps a package ID to its targets; a target written `dev:ID` is a dev-only edge.
function fixtureMetadata(packages, members, edges) {
  return {
    packages,
    workspace_members: members,
    resolve: {nodes: packages.map(pkg => ({
      id: pkg.id,
      deps: (edges[pkg.id] ?? []).map(target => target.startsWith('dev:')
        ? {pkg: target.slice(4), dep_kinds: [{kind: 'dev', target: null}]}
        : {pkg: target, dep_kinds: [{kind: null, target: null}]}),
    }))},
  };
}

function selfTest() {
  // Fixture members: the real crates plus a host application.
  const fixturePolicy = {...policy, 'orange-app': {core: false, normal: ['orange-model']}};
  const core = makePackage('model 0.0.0', 'orange-model', '0.0.0');
  const app = makePackage('orange-app 0.0.0', 'orange-app', '0.0.0', [{name: 'eframe', req: '=1.0.0'}]);
  const eframe = makePackage('eframe 1.0.0', 'eframe', '1.0.0');
  const transitive = makePackage('graphics-helper 1.0.0', 'graphics-helper', '1.0.0');
  const winit = makePackage('winit 1.0.0', 'winit', '1.0.0');
  const eguiExtras = makePackage('egui_extras 1.0.0', 'egui_extras', '1.0.0');
  const tokio = makePackage('tokio 1.0.0', 'tokio', '1.0.0');
  const first = (id, name, dependencies = []) => makePackage(id, name, '0.0.0',
    dependencies.map(([dependency, kind]) => ({name: dependency, req: '*', kind: kind ?? null})));
  let cases = 0;
  const check = (label, metadata, expectedErrors) => {
    cases += 1;
    const actual = inspectMetadata(metadata, fixturePolicy);
    if (actual.length !== expectedErrors) throw new Error(`${label}: expected ${expectedErrors} errors, received ${actual.length}: ${actual.join('; ')}`);
  };
  check('host-only UI framework is allowed', fixtureMetadata([core, app, eframe], [core.id, app.id], {[app.id]: [eframe.id]}), 0);
  check('transitive core leak is rejected', fixtureMetadata([core, transitive, winit], [core.id], {[core.id]: [transitive.id], [transitive.id]: [winit.id]}), 1);
  check('framework subcrate leak is rejected', fixtureMetadata([core, eguiExtras], [core.id], {[core.id]: [eguiExtras.id]}), 1);
  check('optional core declaration is rejected', fixtureMetadata([makePackage('model 0.0.0', 'orange-model', '0.0.0', [{name: 'egui', req: '=1.0.0', optional: true}]), eframe], ['model 0.0.0'], {}), 1);
  const bundled = makePackage('bundled 0.0.0', 'orange-bundled', '0.0.0', [{name: 'egui', req: '=1.0.0'}]);
  check('bundled kernel UI dependency is rejected', fixtureMetadata([core, bundled, eframe], [core.id, bundled.id], {[bundled.id]: [eframe.id]}), 2);

  const compiler = first('compiler 0.0.0', 'orange-compiler', [['orange-model']]);
  const runtime = first('runtime 0.0.0', 'orange-runtime', [['orange-model'], ['orange-compiler']]);
  const workspace = [core, compiler, runtime];
  const ids = workspace.map(pkg => pkg.id);
  const allowedEdges = {[compiler.id]: [core.id], [runtime.id]: [core.id, compiler.id]};
  check('the documented matrix passes', fixtureMetadata(workspace, ids, allowedEdges), 0);
  const backwards = first('model 0.0.0', 'orange-model', [['orange-runtime']]);
  check('core depending upward is rejected', fixtureMetadata([backwards, compiler, runtime], ids, {...allowedEdges, [core.id]: [runtime.id]}), 1);
  const toHost = first('runtime 0.0.0', 'orange-runtime', [['orange-model'], ['orange-compiler'], ['orange-measure']]);
  const measure = first('measure 0.0.0', 'orange-measure', [['orange-model']]);
  check('core depending on a host crate is rejected', fixtureMetadata([core, compiler, toHost, measure], [...ids, measure.id], {...allowedEdges, [toHost.id]: [core.id, compiler.id, measure.id], [measure.id]: [core.id]}), 1);
  const stray = first('stray 0.0.0', 'orange-stray', [['orange-model']]);
  check('an unclassified member is rejected', fixtureMetadata([core, stray], [core.id, stray.id], {[stray.id]: [core.id]}), 1);
  const bundledDev = first('bundled 0.0.0', 'orange-bundled', [['orange-model'], ['orange-compiler'], ['orange-runtime', 'dev']]);
  check('bundled may use the runtime in its tests', fixtureMetadata([...workspace, bundledDev], [...ids, bundledDev.id], {...allowedEdges, [bundledDev.id]: [core.id, compiler.id, `dev:${runtime.id}`]}), 0);
  const bundledNormal = first('bundled 0.0.0', 'orange-bundled', [['orange-model'], ['orange-compiler'], ['orange-runtime']]);
  check('bundled may not ship depending on the runtime', fixtureMetadata([...workspace, bundledNormal], [...ids, bundledNormal.id], {...allowedEdges, [bundledNormal.id]: [core.id, compiler.id, runtime.id]}), 1);
  check('a core crate with a transitive async runtime is rejected', fixtureMetadata([core, transitive, tokio], [core.id], {[core.id]: [transitive.id], [transitive.id]: [tokio.id]}), 1);
  check('a service crate only in core tests is allowed', fixtureMetadata([core, tokio], [core.id], {[core.id]: [`dev:${tokio.id}`]}), 0);
  const host = makePackage('orange-app 0.0.0', 'orange-app', '0.0.0');
  check('a host may use services', fixtureMetadata([core, host, tokio], [core.id, host.id], {[host.id]: [tokio.id]}), 0);

  const mustFail = (label, metadata) => {
    cases += 1;
    let failed = false;
    try { inspectMetadata(metadata, fixturePolicy); } catch { failed = true; }
    if (!failed) throw new Error(`${label}: malformed metadata unexpectedly passed`);
  };
  mustFail('broken graph', {packages: [], workspace_members: ['missing'], resolve: {nodes: []}});
  mustFail('empty workspace', {packages: [], workspace_members: [], resolve: {nodes: []}});
  mustFail('missing orange-model root', fixtureMetadata([app], [app.id], {}));
  mustFail('missing resolve deps', {...fixtureMetadata([core], [core.id], {}), resolve: {nodes: [{id: core.id}]}});
  console.log(`Cargo metadata policy fixtures passed (${cases} cases).`);
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
  console.log('Cargo metadata policy passed: first-party dependency matrix holds and graph core is free of UI, engine and service frameworks.');
}
