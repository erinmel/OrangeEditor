//! `orange`: check and run graph documents from a terminal.
//!
//! This is the smallest front end of `orange-host`. It exists so a graph can be run and
//! controlled without an editor, and as the reference for how any front end attaches.
use orange_host::{
    FlowEvent, FlowId, Grants, Host, HostError, HostEvent, HostOptions, NodeId, StopMode,
};
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

const USAGE: &str = "\
usage:
  orange check <graph.json>... [--package <file.orangeplug>]... [--grant-log] [--store <dir>]
  orange run   <graph.json>... [--package <file.orangeplug>]... [--grant-log] [--store <dir>] [--for <ms>]
  orange pack  <package-dir> <out.orangeplug>
  orange nodes [--package <file.orangeplug>]... [--store <dir>]
  orange packages [--store <dir>]
  orange enable <package-id> --store <dir>
  orange disable <package-id> --store <dir>
  orange remove <package-id> <version> --store <dir>
  orange updates [--apply] [--grant-log] --store <dir>
  orange devices
  orange secret set <name> | remove <name> | check <name>

`run` starts every graph as its own flow, numbered from 1 in argument order, and prints
what they do. While it runs, type commands on standard input:

  pause <flow> | resume <flow>      hold a flow, or let it continue
  stop <flow>  | kill <flow>        stop gracefully, or at once
  trigger <flow> <node-id>          fire a manual entry
  reload <flow>                     read the flow's file again and apply the edit live
  quit                              stop everything and exit

`pack` builds a package from a directory holding `manifest.json` and the package's
files (`nodes/<name>.json`, `graphs/<name>.json`, a component), filling in its digest.

`--share <id> <adapter> <settings.json>` gives every graph that declares connection
`<id>` as shared the same session, with settings read from the file. The base network
package is built in: adapters `org.orange.net/http` and `org.orange.net/websocket`.

`--store <dir>` keeps installed packages in a directory: packages given with
`--package` are saved there, and later commands with the same `--store` have them
without `--package`.

`packages` lists the base, native and stored packages and whether each is in use.
`disable <id>` leaves a package out from the next command on and `enable <id>` brings
it back; both need `--store`, where the choice is kept. `remove` takes one out of the
store. `updates` reads the update feed each package names and lists newer releases;
with `--apply` it downloads each, checks its digest against the feed and installs it.
A package that names a signing key is updated only from a feed that key signed. A
newer native plugin library built for this program replaces the loaded one in its
`--plugins` directory and loads at the next start. Packages are installed at your own risk: their code is not reviewed.

`nodes` lists every node a graph can use: its ID and version, its name, its category
and the package that implements it. `devices` lists the audio devices an audio
connection's `device` setting can name.

The base audio package is built in too: adapters `org.orange.audio/output` and
`org.orange.audio/input`.

`--plugins <dir>` loads every native plugin library in a directory (`.dll`, `.so` or
`.dylib` files built on the plugin SDK with this application's compiler and lockfile).
A native plugin runs with all of this program's rights; load only what you trust.

`--no-base` leaves out the bundled base nodes, `--no-net` the network package and
`--no-audio` the audio package.

`secret set <name>` reads a credential from standard input and keeps it in the
operating system's credential store; a connection's settings name it as
`credential://<name>` (for example `"bearer": "credential://api/token"`), so graph files
hold no secrets. `secret check` says whether one is stored without showing it.

`--grant-log` lets installed package code write log lines, `--grant-files` lets it
read and write files in its own data directory (`--plugin-data <dir>`, or `data` in the
store), and `--grant-http` lets it send HTTPS requests. A grant is given to the
packages installed by that command and is kept with them in the store. `--for` stops after a time.";

struct Arguments {
    command: String,
    graphs: Vec<String>,
    packages: Vec<String>,
    shared: Vec<[String; 3]>,
    grants: Grants,
    duration: Option<Duration>,
    store: Option<String>,
    base_nodes: bool,
    network: bool,
    audio: bool,
    apply: bool,
    plugins: Vec<String>,
    plugin_data: Option<String>,
}

fn arguments() -> Result<Arguments, String> {
    let mut words = std::env::args().skip(1);
    let command = words.next().ok_or("missing command")?;
    let mut parsed = Arguments {
        command,
        graphs: Vec::new(),
        packages: Vec::new(),
        shared: Vec::new(),
        grants: Grants::default(),
        duration: None,
        store: None,
        base_nodes: true,
        network: true,
        audio: true,
        apply: false,
        plugins: Vec::new(),
        plugin_data: None,
    };
    while let Some(word) = words.next() {
        match word.as_str() {
            "--package" => parsed
                .packages
                .push(words.next().ok_or("--package needs a file")?),
            "--share" => {
                let mut next = || {
                    words
                        .next()
                        .ok_or("--share needs an id, an adapter and a file")
                };
                parsed.shared.push([next()?, next()?, next()?]);
            }
            "--grant-log" => parsed.grants.log = true,
            "--grant-files" => parsed.grants.files = true,
            "--grant-http" => parsed.grants.http = true,
            "--plugin-data" => {
                parsed.plugin_data = Some(words.next().ok_or("--plugin-data needs a directory")?);
            }
            "--no-base" => parsed.base_nodes = false,
            "--no-net" => parsed.network = false,
            "--no-audio" => parsed.audio = false,
            "--apply" => parsed.apply = true,
            "--plugins" => parsed
                .plugins
                .push(words.next().ok_or("--plugins needs a directory")?),
            "--store" => parsed.store = Some(words.next().ok_or("--store needs a directory")?),
            "--for" => {
                let text = words.next().ok_or("--for needs milliseconds")?;
                let ms: u64 = text.parse().map_err(|_| "--for needs milliseconds")?;
                parsed.duration = Some(Duration::from_millis(ms));
            }
            flag if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
            _ => parsed.graphs.push(word),
        }
    }
    if parsed.graphs.is_empty() && matches!(parsed.command.as_str(), "check" | "run") {
        return Err("no graph given".into());
    }
    Ok(parsed)
}

fn read(path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|error| format!("{path}: {error}"))
}

fn share(host: &mut Host, id: &str, adapter: &str, settings: &[u8]) -> Result<(), String> {
    let id = id.parse().map_err(|_| "not a connection id".to_owned())?;
    let settings = orange_host::settings_from_json(settings).map_err(|error| error.to_string())?;
    host.share_connection(id, adapter, settings)
        .map_err(|error| error.to_string())
}

fn report(path: &str, error: &HostError) {
    eprintln!("{path}: {error}");
    if let HostError::Invalid(diagnostics) = error {
        for diagnostic in diagnostics {
            eprintln!("  {diagnostic:?}");
        }
    }
}

fn print(event: &HostEvent) {
    let flow = event.flow;
    match &event.kind {
        FlowEvent::State(state) => println!("[{flow}] state {state:?}"),
        FlowEvent::Node(step) => match &step.fault {
            None => println!("[{flow}] node {} {:?}", step.node, step.outcome),
            Some(fault) => println!(
                "[{flow}] node {} {:?} fault {} from {}",
                step.node, step.outcome, fault.fault.code, fault.source
            ),
        },
        FlowEvent::TraceLost(count) => println!("[{flow}] {count} steps not shown"),
        FlowEvent::Refused(reason) => println!("[{flow}] input refused: {reason:?}"),
        FlowEvent::Fault(error) => println!("[{flow}] error: {error}"),
        FlowEvent::Updated { .. } => println!("[{flow}] graph replaced"),
        FlowEvent::Finished => println!("[{flow}] finished"),
        _ => println!("[{flow}] {:?}", event.kind),
    }
}

/// Applies one line typed by the user. Returns `false` for `quit`.
/// Applies an edited graph file to its running flow.
fn reload(host: &mut Host, flow: FlowId, path: &str) -> Result<(), String> {
    let text = String::from_utf8(read(path)?).map_err(|_| format!("{path}: not UTF-8 text"))?;
    match host.update(flow, &text) {
        Ok(updated) if !updated.changed => println!("[{flow}] nothing to apply"),
        Ok(updated) => println!(
            "[{flow}] edit applied: {} variables kept, {} reset",
            updated.kept_variables, updated.reset_variables
        ),
        Err(error) => {
            report(path, &error);
            println!("[{flow}] edit refused; the flow is unchanged");
        }
    }
    Ok(())
}

fn command(host: &mut Host, flows: &[(FlowId, String)], line: &str) -> bool {
    let words: Vec<&str> = line.split_whitespace().collect();
    let flow = |index: usize| -> Result<(FlowId, &str), String> {
        let number: usize = words
            .get(index)
            .and_then(|word| word.parse().ok())
            .ok_or("expected a flow number")?;
        number
            .checked_sub(1)
            .and_then(|at| flows.get(at))
            .map(|(flow, path)| (*flow, path.as_str()))
            .ok_or_else(|| "no such flow".to_owned())
    };
    let done = |result: Result<(), HostError>| result.map_err(|error| error.to_string());
    let outcome = match words.first().copied() {
        None => Ok(()),
        Some("quit") => return false,
        Some("pause") => flow(1).and_then(|(flow, _)| done(host.pause(flow))),
        Some("resume") => flow(1).and_then(|(flow, _)| done(host.resume(flow))),
        Some("reload") => flow(1).and_then(|(flow, path)| reload(host, flow, path)),
        Some("stop") => flow(1).and_then(|(flow, _)| done(host.stop(flow, StopMode::Graceful))),
        Some("kill") => flow(1).and_then(|(flow, _)| done(host.stop(flow, StopMode::Immediate))),
        Some("trigger") => flow(1).and_then(|(flow, _)| {
            let node: NodeId = words
                .get(2)
                .and_then(|word| word.parse().ok())
                .ok_or("expected a node id")?;
            done(host.trigger(flow, node, None))
        }),
        Some(other) => Err(format!("unknown command `{other}`")),
    };
    if let Err(message) = outcome {
        eprintln!("{message}");
    }
    true
}

fn run(
    host: &mut Host,
    events: &Receiver<HostEvent>,
    flows: &[(FlowId, String)],
    duration: Option<Duration>,
) {
    let (lines, typed) = channel::<String>();
    // Standard input blocks, so it gets its own thread; it ends with the process.
    let _ = std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if lines.send(line).is_err() {
                break;
            }
        }
    });
    let started = Instant::now();
    let mut running = flows.len();
    let mut stopping = false;
    while running > 0 {
        match events.recv_timeout(Duration::from_millis(20)) {
            Ok(event) => {
                print(&event);
                if matches!(event.kind, FlowEvent::Finished) {
                    running -= 1;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        let mut quit = duration.is_some_and(|limit| started.elapsed() >= limit);
        while let Ok(line) = typed.try_recv() {
            quit |= !command(host, flows, &line);
        }
        if quit && !stopping {
            stopping = true;
            for (flow, _) in flows {
                // A flow that already ended refuses the stop; that is the goal anyway.
                let _ = host.stop(*flow, StopMode::Graceful);
            }
        }
    }
    let dropped = host.events_dropped();
    if dropped > 0 {
        println!("{dropped} events were not shown");
    }
}

fn collect(root: &Path, at: &Path, files: &mut Vec<(String, Vec<u8>)>) -> Result<(), String> {
    let failed = |error: std::io::Error| format!("{}: {error}", at.display());
    for entry in std::fs::read_dir(at).map_err(failed)? {
        let path = entry.map_err(failed)?.path();
        if path.is_dir() {
            collect(root, &path, files)?;
            continue;
        }
        let relative = path.strip_prefix(root).map_err(|error| error.to_string())?;
        let name: Vec<_> = relative
            .components()
            .map(|part| part.as_os_str().to_string_lossy())
            .collect();
        let name = name.join("/");
        if name != "manifest.json" {
            files.push((name, read(&path.to_string_lossy())?));
        }
    }
    Ok(())
}

fn pack(directory: &str, output: &str) -> Result<usize, String> {
    let root = Path::new(directory);
    let manifest = read(&root.join("manifest.json").to_string_lossy())?;
    let mut files = Vec::new();
    collect(root, root, &mut files)?;
    let archive = orange_host::pack(&manifest, &files).map_err(|error| error.to_string())?;
    std::fs::write(output, &archive).map_err(|error| format!("{output}: {error}"))?;
    Ok(files.len())
}

/// Largest release archive `updates --apply` downloads.
const MAX_RELEASE_BYTES: u64 = 256 * 1024 * 1024;

/// Downloads a package's update feed. When the package names a signing key, the feed's
/// signature is downloaded too and the feed is refused unless the key signed it.
fn feed(source: &orange_host::UpdateSource) -> Result<orange_host::UpdateFeed, String> {
    let bytes = orange_net::download(source.feed(), 1024 * 1024)?;
    if let Some(key) = source.key() {
        let signature = orange_net::download(&format!("{}.sig", source.feed()), 1024)
            .map_err(|error| format!("the feed's signature: {error}"))?;
        orange_net::verify_feed(&bytes, &signature, key)?;
    }
    orange_host::UpdateFeed::from_json(&bytes).map_err(|error| error.to_string())
}

/// Reads the feed of every package that names one and reports, or installs, the newer
/// releases: archives for installed packages, libraries built for this program's
/// target for native plugins. `plugins` maps a loaded plugin's ID to its file.
fn updates(
    host: &mut Host,
    apply: bool,
    grants: Grants,
    plugins: &[(String, PathBuf)],
) -> ExitCode {
    // (release, the plugin file it replaces)
    let mut found: Vec<(orange_host::Release, Option<PathBuf>)> = Vec::new();
    let mut failed = false;
    for package in host.packages() {
        let Some(source) = package.update() else {
            continue;
        };
        let file = plugins
            .iter()
            .find(|(id, _)| id == package.id)
            .map(|(_, path)| path.clone());
        let native = package.kind == orange_host::PackageKind::Native;
        let release = feed(source).and_then(|feed| match package.manifest {
            Some(manifest) => Ok(feed.update_for(manifest).cloned()),
            None => {
                let current = orange_host::PackageKey {
                    id: package
                        .id
                        .parse()
                        .map_err(|_| "the package ID is not valid")?,
                    version: package
                        .version
                        .parse()
                        .map_err(|_| "the package version is not valid")?,
                };
                let target = native.then_some(orange_host::TARGET);
                Ok(feed.newer(&current, source.channel(), target).cloned())
            }
        });
        let signed = if source.key().is_some() {
            "signed feed"
        } else {
            "unsigned feed"
        };
        match release {
            Ok(Some(release)) => {
                println!(
                    "{} {} -> {} ({}, {signed})",
                    package.id,
                    package.version,
                    release.key().version.as_str(),
                    release.archive()
                );
                if native && file.is_none() {
                    println!(
                        "  {} is built into this program and is not replaced",
                        package.id
                    );
                } else if package.active {
                    found.push((release, file));
                }
            }
            Ok(None) => println!(
                "{} {} is up to date ({signed})",
                package.id, package.version
            ),
            Err(message) => {
                eprintln!("{} {}: {message}", package.id, package.version);
                failed = true;
            }
        }
    }
    if apply {
        for (release, file) in found {
            let downloaded = orange_net::download(release.archive(), MAX_RELEASE_BYTES);
            let installed = match file {
                None => downloaded.and_then(|archive| {
                    host.install_release(&archive, grants, release.content_digest())
                        .map(|key| {
                            format!("installed {} {}", key.id.as_str(), key.version.as_str())
                        })
                        .map_err(|error| error.to_string())
                }),
                Some(current) => downloaded.and_then(|library| {
                    if orange_net::sha256(&library) != release.content_digest() {
                        return Err("its content digest is not the announced one".to_owned());
                    }
                    let key = release.key();
                    orange_native::replace(
                        &current,
                        key.id.as_str(),
                        key.version.as_str(),
                        &library,
                    )
                    .map(|path| format!("saved {}; it loads at the next start", path.display()))
                    .map_err(|error| error.to_string())
                }),
            };
            match installed {
                Ok(message) => println!("{message}"),
                Err(message) => {
                    eprintln!("{}: {message}", release.archive());
                    failed = true;
                }
            }
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Name under which the operating system's credential store lists this program's
/// secrets.
const VAULT_SERVICE: &str = "OrangeEditor";

/// `secret set|remove|check <name>`: manages one credential in the operating system's
/// store. The secret itself is read from standard input, never from the command line,
/// where the shell's history and the process list would keep it.
fn secret(words: &[String]) -> ExitCode {
    use orange_host::SecretStore;
    let [action, name] = words else {
        eprintln!("secret needs `set`, `remove` or `check` and a name\n{USAGE}");
        return ExitCode::from(2);
    };
    let vault = orange_vault::OsVault::new(VAULT_SERVICE);
    let done = match action.as_str() {
        "set" => {
            let mut line = String::new();
            match std::io::stdin().lock().read_line(&mut line) {
                Ok(_) => vault
                    .set(name, line.trim_end_matches(['\r', '\n']))
                    .map(|()| format!("stored {name}")),
                Err(_) => {
                    eprintln!("the secret could not be read from standard input");
                    return ExitCode::FAILURE;
                }
            }
        }
        "remove" => vault.remove(name).map(|()| format!("removed {name}")),
        "check" => vault.get(name).map(|_| format!("{name} is stored")),
        _ => {
            eprintln!("unknown secret action `{action}`\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match done {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{name}: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The network client of installed packages that hold the `http` grant.
#[derive(Debug)]
struct PluginHttp;
impl orange_host::HttpClient for PluginHttp {
    fn send(
        &self,
        request: &orange_host::HttpRequest,
    ) -> Result<orange_host::HttpResponse, String> {
        let limit = u64::try_from(request.max_response_bytes).unwrap_or(u64::MAX);
        orange_net::fetch(
            &request.method,
            &request.url,
            &request.headers,
            &request.body,
            limit,
        )
        .map(|(status, body)| orange_host::HttpResponse { status, body })
    }
}

fn main() -> ExitCode {
    let words: Vec<String> = std::env::args().skip(1).collect();
    if words.first().map(String::as_str) == Some("secret") {
        return secret(&words[1..]);
    }
    if words.first().map(String::as_str) == Some("pack") {
        let [_, directory, output] = &words[..] else {
            eprintln!(
                "pack needs a directory and an output file
{USAGE}"
            );
            return ExitCode::from(2);
        };
        return match pack(directory, output) {
            Ok(files) => {
                println!("{output}: manifest and {files} files");
                ExitCode::SUCCESS
            }
            Err(message) => {
                eprintln!("{message}");
                ExitCode::FAILURE
            }
        };
    }
    if words.first().map(String::as_str) == Some("devices") {
        return match orange_audio::devices() {
            Ok(devices) => {
                for name in devices.outputs {
                    println!("output  {name}");
                }
                for name in devices.inputs {
                    println!("input   {name}");
                }
                ExitCode::SUCCESS
            }
            Err(message) => {
                eprintln!("{message}");
                ExitCode::FAILURE
            }
        };
    }
    let arguments = match arguments() {
        Ok(arguments)
            if matches!(
                arguments.command.as_str(),
                "check"
                    | "run"
                    | "nodes"
                    | "packages"
                    | "remove"
                    | "updates"
                    | "enable"
                    | "disable"
            ) =>
        {
            arguments
        }
        Ok(arguments) => {
            eprintln!("unknown command `{}`\n{USAGE}", arguments.command);
            return ExitCode::from(2);
        }
        Err(message) => {
            eprintln!("{message}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let mut services = Vec::new();
    let mut plugin_files = Vec::new();
    if arguments.network {
        services.push(orange_net::package());
    }
    if arguments.audio {
        services.push(orange_audio::package());
    }
    for directory in &arguments.plugins {
        let loaded = match orange_native::load_directory(Path::new(directory)) {
            Ok(loaded) => loaded,
            Err(error) => {
                eprintln!("{directory}: {error}");
                return ExitCode::FAILURE;
            }
        };
        for (path, package) in loaded {
            match package {
                Ok(package) => {
                    println!("loaded plugin {} {}", package.id(), package.version());
                    plugin_files.push((package.id().to_owned(), path));
                    services.push(package);
                }
                // One bad library does not stop the others or the program.
                Err(error) => eprintln!("{}: {error}", path.display()),
            }
        }
    }
    let options = HostOptions {
        package_dir: arguments.store.as_ref().map(Into::into),
        base_nodes: arguments.base_nodes,
        // Without `--plugin-data`, package data lives beside the stored packages.
        plugin_data_dir: arguments
            .plugin_data
            .as_ref()
            .map(PathBuf::from)
            .or_else(|| {
                arguments
                    .store
                    .as_ref()
                    .map(|store| Path::new(store).join("data"))
            }),
        plugin_http: Some(std::sync::Arc::new(PluginHttp)),
        secrets: Some(std::sync::Arc::new(orange_vault::OsVault::new(VAULT_SERVICE))),
        ..HostOptions::default()
    };
    let (mut host, events) = match Host::with_services(options, services) {
        Ok(started) => started,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    for (file, error) in host.not_restored() {
        eprintln!("stored package {file} was not installed: {error}");
    }
    for path in &arguments.packages {
        let installed = read(path).and_then(|bytes| {
            host.install(&bytes, arguments.grants)
                .map_err(|error| format!("{path}: {error}"))
        });
        match installed {
            Ok(key) => println!("installed {} {}", key.id.as_str(), key.version.as_str()),
            Err(message) => {
                eprintln!("{message}");
                return ExitCode::FAILURE;
            }
        }
    }
    match arguments.command.as_str() {
        "packages" => {
            for package in host.packages() {
                println!(
                    "{}	{}	{}	{}	{}",
                    package.id,
                    package.version,
                    match package.kind {
                        orange_host::PackageKind::Base => "base",
                        orange_host::PackageKind::Native => "native",
                        _ => "installed",
                    },
                    match (package.active, package.enabled) {
                        (true, true) => "on",
                        (false, false) => "disabled",
                        (true, false) => "on, disabled from the next start",
                        (false, true) => "off",
                    },
                    package.name().unwrap_or("-"),
                );
            }
            return ExitCode::SUCCESS;
        }
        "enable" | "disable" => {
            let enable = arguments.command == "enable";
            let [id] = &arguments.graphs[..] else {
                eprintln!("{} needs a package ID\n{USAGE}", arguments.command);
                return ExitCode::from(2);
            };
            if arguments.store.is_none() {
                eprintln!(
                    "{} needs --store, where the choice is kept",
                    arguments.command
                );
                return ExitCode::from(2);
            }
            if !host.packages().any(|package| package.id == id) {
                eprintln!("no package {id}");
                return ExitCode::FAILURE;
            }
            return match host.set_enabled(id, enable) {
                Ok(()) => {
                    println!("{} {id}", if enable { "enabled" } else { "disabled" });
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("{error}");
                    ExitCode::FAILURE
                }
            };
        }
        "remove" => {
            let [id, version] = &arguments.graphs[..] else {
                eprintln!(
                    "remove needs a package ID and a version
{USAGE}"
                );
                return ExitCode::from(2);
            };
            let found = host
                .packages()
                .any(|package| {
                    package.kind == orange_host::PackageKind::Installed
                        && package.id == id
                        && package.version == version
                })
                .then(|| {
                    Some(orange_host::PackageKey {
                        id: id.parse().ok()?,
                        version: version.parse().ok()?,
                    })
                })
                .flatten();
            let removed = found
                .ok_or_else(|| format!("{id} {version} is not installed"))
                .and_then(|key| host.uninstall(&key).map_err(|error| error.to_string()));
            return match removed {
                Ok(()) => {
                    println!("removed {id} {version}");
                    ExitCode::SUCCESS
                }
                Err(message) => {
                    eprintln!("{message}");
                    ExitCode::FAILURE
                }
            };
        }
        "updates" => return updates(&mut host, arguments.apply, arguments.grants, &plugin_files),
        _ => {}
    }
    if arguments.command == "nodes" {
        for info in host.nodes() {
            let definition = &info.definition.definition;
            println!(
                "{}@{}	{}	{}	{}",
                definition.id.as_str(),
                definition.version,
                info.label(),
                info.category(),
                info.provider
                    .map_or("-", |provider| provider.package.as_str()),
            );
        }
        return ExitCode::SUCCESS;
    }
    for [id, adapter, path] in &arguments.shared {
        let shared = read(path).and_then(|bytes| share(&mut host, id, adapter, &bytes));
        if let Err(message) = shared {
            eprintln!("--share {id}: {message}");
            return ExitCode::FAILURE;
        }
    }
    let mut failed = false;
    let mut flows = Vec::new();
    for path in &arguments.graphs {
        let text = match read(path).map(String::from_utf8) {
            Ok(Ok(text)) => text,
            Ok(Err(_)) => {
                eprintln!("{path}: not UTF-8 text");
                failed = true;
                continue;
            }
            Err(message) => {
                eprintln!("{message}");
                failed = true;
                continue;
            }
        };
        if arguments.command == "check" {
            match host.check(&text) {
                Ok(diagnostics) if diagnostics.is_empty() => println!("{path}: ok"),
                Ok(diagnostics) => {
                    report(path, &HostError::Invalid(diagnostics));
                    failed = true;
                }
                Err(error) => {
                    report(path, &error);
                    failed = true;
                }
            }
        } else {
            match host.open(&text) {
                Ok(flow) => {
                    println!("[{flow}] opened {path}");
                    flows.push((flow, path.clone()));
                }
                Err(error) => {
                    report(path, &error);
                    failed = true;
                }
            }
        }
    }
    if failed {
        // Nothing runs unless everything asked for can; flows already opened are
        // stopped when the host is dropped.
        return ExitCode::FAILURE;
    }
    if arguments.command == "run" {
        run(&mut host, &events, &flows, arguments.duration);
    }
    ExitCode::SUCCESS
}
