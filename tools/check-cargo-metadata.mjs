import {execFileSync} from 'node:child_process';

const corePackages = new Set([
  'orange-model',
  'orange-graph',
  'orange-types',
  'orange-compiler',
  'orange-runtime',
  'orange-contracts',
]);
const bevyVersion = '0.20.0';
const isBevyFrameworkPackage = name => name === 'bevy' || name.startsWith('bevy_');

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
  for (const pkg of metadata.packages.filter(pkg => pkg.name === 'bevy')) {
    if (pkg.version !== bevyVersion) errors.push(`resolved Bevy ${pkg.version}; required ${bevyVersion}`);
  }
  for (const pkg of members) {
    for (const dependency of pkg.dependencies ?? []) {
      const dependencyName = dependency.package ?? dependency.name;
      if (dependencyName === 'bevy' && dependency.req !== `=${bevyVersion}`) {
        errors.push(`${pkg.name} requires Bevy as '${dependency.req}'; use '=${bevyVersion}'`);
      }
      if (corePackages.has(pkg.name) && isBevyFrameworkPackage(dependencyName)) {
        errors.push(`${pkg.name} declares ${dependencyName}; graph core must remain Bevy-free, including optional dependencies`);
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
      if (isBevyFrameworkPackage(pkg.name)) errors.push(`${name} has a transitive Bevy dependency (${pkg.name} ${pkg.version})`);
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
  const app = makePackage('orange-app 0.0.0', 'orange-app', '0.0.0', [{name: 'bevy', req: '=0.20.0'}]);
  const bevy = makePackage('bevy 0.20.0', 'bevy', '0.20.0');
  const transitive = makePackage('graphics-helper 1.0.0', 'graphics-helper', '1.0.0');
  const bevyEcs = makePackage('bevy_ecs 0.20.0', 'bevy_ecs', '0.20.0');
  const check = (label, metadata, expectedErrors) => {
    const actual = inspectMetadata(metadata);
    if (actual.length !== expectedErrors) throw new Error(`${label}: expected ${expectedErrors} errors, received ${actual.length}: ${actual.join('; ')}`);
  };
  check('host-only Bevy is allowed', fixtureMetadata([core, app, bevy], [core.id, app.id], {[app.id]: [bevy.id]}), 0);
  check('transitive core leak is rejected', fixtureMetadata([core, transitive, bevy], [core.id], {[core.id]: [transitive.id], [transitive.id]: [bevy.id]}), 1);
  check('transitive Bevy subsystem leak is rejected', fixtureMetadata([core, bevyEcs], [core.id], {[core.id]: [bevyEcs.id]}), 1);
  check('optional core declaration is rejected', fixtureMetadata([makePackage('model 0.0.0', 'orange-model', '0.0.0', [{name: 'bevy', req: '=0.20.0', optional: true}]), bevy], ['model 0.0.0'], {}), 1);
  check('invalid Bevy version is rejected', fixtureMetadata([core, app, makePackage('bevy 0.21.0', 'bevy', '0.21.0')], [core.id, app.id], {}), 1);
  check('non-exact member pin is rejected', fixtureMetadata([core, makePackage('app 0.0.0', 'orange-app', '0.0.0', [{name: 'bevy', req: '^0.20'}]), bevy], [core.id, 'app 0.0.0'], {}), 1);
  const mustFail = (label, metadata) => {
    let failed = false;
    try { inspectMetadata(metadata); } catch { failed = true; }
    if (!failed) throw new Error(`${label}: malformed metadata unexpectedly passed`);
  };
  mustFail('broken graph', {packages: [], workspace_members: ['missing'], resolve: {nodes: []}});
  mustFail('empty workspace', {packages: [], workspace_members: [], resolve: {nodes: []}});
  mustFail('missing orange-model root', fixtureMetadata([app], [app.id], {}));
  mustFail('missing resolve deps', {...fixtureMetadata([core], [core.id], {}), resolve: {nodes: [{id: core.id}]}});
  console.log('Cargo metadata policy fixtures passed (10 cases).');
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
  const bevyPresent = metadata.packages.some(pkg => pkg.name === 'bevy');
  console.log(bevyPresent
    ? 'Cargo metadata policy passed; core dependency boundary and exact Bevy 0.20.0 pin verified.'
    : 'Cargo metadata policy passed; core dependency boundary holds; Bevy pin check is inapplicable because Bevy is absent.');
}
