//! One function per command. Each returns why it stopped; `main` turns that into what
//! is printed and the exit code.
use crate::*;
use orange_host::{PackageKey, PackageKind};

/// Why a command did not succeed.
pub(crate) enum Stop {
    /// It was asked for wrongly: the message is printed with the usage text.
    Usage(String),
    /// It could not be done.
    Failed(String),
    /// It could not be done and has already said why.
    Reported,
}

impl From<String> for Stop {
    fn from(message: String) -> Self {
        Self::Failed(message)
    }
}

pub(crate) type Done = Result<(), Stop>;

fn usage<T>(message: impl Into<String>) -> Result<T, Stop> {
    Err(Stop::Usage(message.into()))
}

/// A file as text.
pub(crate) fn text(path: &str) -> Result<String, String> {
    String::from_utf8(read(path)?).map_err(|_| format!("{path}: not UTF-8 text"))
}

fn write(path: &str, bytes: impl AsRef<[u8]>) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|error| format!("{path}: {error}"))
}

/// A host with the base packages, the plugin libraries and the stored packages the
/// options ask for.
pub(crate) struct Started {
    pub host: Host,
    pub events: Receiver<HostEvent>,
    /// Each loaded plugin library's package ID with the file it came from.
    pub plugin_files: Vec<(String, PathBuf)>,
}

pub(crate) fn publish(words: &[String]) -> Done {
    let [graph, function, package, version, node, output] = words else {
        return usage(
            "publish needs a graph file, a function, a package ID, a version, a node name and an output file",
        );
    };
    let publish = orange_host::Publish {
        package,
        version,
        node,
        label: None,
    };
    let archive = orange_host::publish_function(&text(graph)?, function, &publish)
        .map_err(|error| format!("{graph}: {error}"))?;
    write(output, archive)?;
    println!("{output}: node {package}/{node} {version}");
    Ok(())
}

pub(crate) fn oauth(words: &[String]) -> Done {
    let [action, settings, name] = words else {
        return usage("oauth needs `login <settings.json> <name>`");
    };
    if action != "login" {
        return usage(format!("unknown oauth action `{action}`"));
    }
    Ok(oauth_login(settings, name)?)
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

pub(crate) fn pack(words: &[String]) -> Done {
    let [directory, output] = words else {
        return usage("pack needs a directory and an output file");
    };
    let root = Path::new(directory);
    let manifest = read(&root.join("manifest.json").to_string_lossy())?;
    let mut files = Vec::new();
    collect(root, root, &mut files)?;
    let archive = orange_host::pack(&manifest, &files).map_err(|error| error.to_string())?;
    write(output, archive)?;
    println!("{output}: manifest and {} files", files.len());
    Ok(())
}

pub(crate) fn devices() -> Done {
    let devices = orange_audio::devices()?;
    for name in devices.outputs {
        println!("output  {name}");
    }
    for name in devices.inputs {
        println!("input   {name}");
    }
    Ok(())
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

/// Starts a host as the options describe and installs the packages given with
/// `--package`.
pub(crate) fn start(arguments: &Arguments) -> Result<Started, Stop> {
    let mut services = Vec::new();
    let mut plugin_files = Vec::new();
    if arguments.network {
        services.push(orange_net::package());
    }
    if arguments.files {
        services.push(orange_files::package());
    }
    if arguments.audio {
        services.push(orange_audio::package());
    }
    for directory in &arguments.plugins {
        let loaded = orange_native::load_directory(Path::new(directory))
            .map_err(|error| format!("{directory}: {error}"))?;
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
    // Without their own option, package data and persistent variables live beside the
    // stored packages.
    let beside = |given: &Option<String>, name: &str| {
        given.as_ref().map(PathBuf::from).or_else(|| {
            arguments
                .store
                .as_ref()
                .map(|store| Path::new(store).join(name))
        })
    };
    let options = HostOptions {
        package_dir: arguments.store.as_ref().map(Into::into),
        base_nodes: arguments.base_nodes,
        plugin_data_dir: beside(&arguments.plugin_data, "data"),
        state_dir: beside(&arguments.state, "state"),
        plugin_http: Some(std::sync::Arc::new(PluginHttp)),
        secrets: Some(std::sync::Arc::new(orange_vault::OsVault::new(
            VAULT_SERVICE,
        ))),
        ..HostOptions::default()
    };
    let (mut host, events) =
        Host::with_services(options, services).map_err(|error| error.to_string())?;
    for (file, error) in host.not_restored() {
        eprintln!("stored package {file} was not installed: {error}");
    }
    for path in &arguments.packages {
        let bytes = read(path)?;
        // A package gets the grants it asks for that this command also gives.
        let asks = host
            .permissions(&bytes)
            .map_err(|error| format!("{path}: {error}"))?;
        let given = Grants {
            log: asks.log && arguments.grants.log,
            files: asks.files && arguments.grants.files,
            http: asks.http && arguments.grants.http,
        };
        for (name, asked, granted) in [
            ("log", asks.log, given.log),
            ("files", asks.files, given.files),
            ("http", asks.http, given.http),
        ] {
            if asked && !granted {
                println!("{path}: asks for `{name}`, not granted (--grant-{name})");
            }
        }
        let key = host
            .install(&bytes, given)
            .map_err(|error| format!("{path}: {error}"))?;
        println!("installed {} {}", key.id.as_str(), key.version.as_str());
    }
    Ok(Started {
        host,
        events,
        plugin_files,
    })
}

pub(crate) fn packages(host: &Host) {
    for package in host.packages() {
        println!(
            "{}\t{}\t{}\t{}\t{}",
            package.id,
            package.version,
            match package.kind {
                PackageKind::Base => "base",
                PackageKind::Native => "native",
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
}

pub(crate) fn nodes(host: &Host) {
    for info in host.nodes() {
        let definition = &info.definition.definition;
        println!(
            "{}@{}\t{}\t{}\t{}",
            definition.id.as_str(),
            definition.version,
            info.label(),
            info.category(),
            info.provider
                .map_or("-", |provider| provider.package.as_str()),
        );
    }
}

fn bind_one(host: &Host, path: &str) -> Result<bool, String> {
    let failed = |error: &dyn std::fmt::Display| format!("{path}: {error}");
    let bound = host.bind(&text(path)?).map_err(|error| failed(&error))?;
    // A file a person edits is kept readable: one field per line.
    let bound = serde_json::from_str::<serde_json::Value>(&bound)
        .and_then(|graph| serde_json::to_string_pretty(&graph))
        .map_err(|error| failed(&error))?;
    orange_host::save_document(Path::new(path), &bound).map_err(|error| failed(&error))?;
    let diagnostics = host.check(&bound).map_err(|error| failed(&error))?;
    if diagnostics.is_empty() {
        println!("{path}: bound, ok");
        return Ok(true);
    }
    println!("{path}: bound, {} diagnostics", diagnostics.len());
    let names = Names::of(host, &bound);
    for diagnostic in diagnostics {
        println!("  {}", describe(&diagnostic, &names));
    }
    Ok(false)
}

pub(crate) fn bind(host: &Host, graphs: &[String]) -> Done {
    let mut failed = false;
    for path in graphs {
        match bind_one(host, path) {
            Ok(clean) => failed |= !clean,
            Err(message) => {
                eprintln!("{message}");
                failed = true;
            }
        }
    }
    if failed { Err(Stop::Reported) } else { Ok(()) }
}

pub(crate) fn export(host: &Host, words: &[String]) -> Done {
    let [graph, output] = words else {
        return usage("export needs a graph file and an output file");
    };
    let (bundle, missing) = host
        .export(&text(graph)?)
        .map_err(|error| format!("{graph}: {error}"))?;
    write(output, bundle.to_bytes())?;
    println!("{output}: the graph and {} packages", bundle.packages.len());
    for requirement in missing {
        println!(
            "not included: {} {} (not in the store)",
            requirement.package, requirement.version
        );
    }
    Ok(())
}

pub(crate) fn import(host: &mut Host, arguments: &Arguments) -> Done {
    let [bundle, output] = &arguments.graphs[..] else {
        return usage("import needs a bundle file and an output graph file");
    };
    let imported = orange_host::Bundle::from_bytes(&read(bundle)?)
        .map_err(|error| format!("{bundle}: {error}"))?;
    // Packages are installed only on request: importing runs and grants nothing.
    if arguments.apply {
        for archive in &imported.packages {
            match host.install(archive, arguments.grants) {
                Ok(key) => println!("installed {} {}", key.id.as_str(), key.version.as_str()),
                Err(error) => eprintln!("a package was not installed: {error}"),
            }
        }
    }
    for requirement in host.requirements(&imported.document).unwrap_or_default() {
        println!(
            "{} {}\t{}",
            requirement.package,
            requirement.version,
            if requirement.available {
                "available"
            } else {
                "missing"
            }
        );
    }
    orange_host::save_document(Path::new(output), &imported.document)
        .map_err(|error| format!("{output}: {error}"))?;
    println!(
        "{output}: saved ({} packages in the bundle)",
        imported.packages.len()
    );
    Ok(())
}

/// Prints the values of pure outputs without running the graph: the outputs named as
/// node and port pairs, or the document's own exports.
pub(crate) fn preview(host: &mut Host, words: &[String]) -> Done {
    let [path, pairs @ ..] = words else {
        return usage("preview needs a graph file");
    };
    if pairs.len() % 2 != 0 {
        return usage("preview takes outputs as pairs: a node, then one of its ports");
    }
    let graph = text(path)?;
    let names = Names::of(host, &graph);
    let mut ports = Vec::new();
    for pair in pairs.chunks(2) {
        let node = names.find_node(&pair[0])?;
        let port = pair[1]
            .parse()
            .map_err(|_| format!("`{}` is not a port name", pair[1]))?;
        ports.push((node, port));
    }
    let previewed = match host.preview(&graph, &ports) {
        Ok(previewed) => previewed,
        Err(error) => {
            report(path, &error, &names);
            return Err(Stop::Reported);
        }
    };
    if previewed.is_empty() {
        println!("{path}: no outputs named and the document exports none");
    }
    for output in previewed {
        let (node, port) = (names.node(&output.node), output.port);
        match output.value {
            Ok(value) => println!("{node} {port} = {}", plain(&value)),
            Err((source, fault)) => {
                println!(
                    "{node} {port}: fault {} at {}",
                    fault.code,
                    names.node(&source)
                );
            }
        }
    }
    Ok(())
}

/// `enable` and `disable`.
pub(crate) fn switch(host: &mut Host, arguments: &Arguments) -> Done {
    let command = &arguments.command;
    let enable = command == "enable";
    let [id] = &arguments.graphs[..] else {
        return usage(format!("{command} needs a package ID"));
    };
    if arguments.store.is_none() {
        return usage(format!("{command} needs --store, where the choice is kept"));
    }
    if !host.packages().any(|package| package.id == id) {
        return Err(Stop::Failed(format!("no package {id}")));
    }
    host.set_enabled(id, enable)
        .map_err(|error| error.to_string())?;
    println!("{} {id}", if enable { "enabled" } else { "disabled" });
    Ok(())
}

pub(crate) fn remove(host: &mut Host, words: &[String]) -> Done {
    let [id, version] = words else {
        return usage("remove needs a package ID and a version");
    };
    let installed = host.packages().any(|package| {
        package.kind == PackageKind::Installed && package.id == id && package.version == version
    });
    let key = installed
        .then(|| {
            Some(PackageKey {
                id: id.parse().ok()?,
                version: version.parse().ok()?,
            })
        })
        .flatten()
        .ok_or_else(|| format!("{id} {version} is not installed"))?;
    host.uninstall(&key).map_err(|error| error.to_string())?;
    println!("removed {id} {version}");
    Ok(())
}

fn share(host: &mut Host, id: &str, adapter: &str, settings: &[u8]) -> Result<(), String> {
    let id = id.parse().map_err(|_| "not a connection id".to_owned())?;
    let settings = orange_host::settings_from_json(settings).map_err(|error| error.to_string())?;
    host.share_connection(id, adapter, settings)
        .map_err(|error| error.to_string())
}

/// `check` and `run`: nothing runs unless every graph asked for can.
pub(crate) fn graphs(started: &mut Started, arguments: &Arguments) -> Done {
    let host = &mut started.host;
    for [id, adapter, path] in &arguments.shared {
        read(path)
            .and_then(|bytes| share(host, id, adapter, &bytes))
            .map_err(|message| format!("--share {id}: {message}"))?;
    }
    let check = arguments.command == "check";
    let mut failed = false;
    let mut flows = Vec::new();
    for path in &arguments.graphs {
        let text = match text(path) {
            Ok(text) => text,
            Err(message) => {
                eprintln!("{message}");
                failed = true;
                continue;
            }
        };
        let names = Names::of(host, &text);
        let outcome = if check {
            host.check(&text).and_then(|diagnostics| {
                if diagnostics.is_empty() {
                    println!("{path}: ok");
                    Ok(())
                } else {
                    Err(HostError::Invalid(diagnostics))
                }
            })
        } else {
            host.open(&text).map(|flow| {
                if arguments.watch {
                    let _ = host.observe(flow, true);
                }
                println!("[{flow}] opened {path}");
                flows.push(Opened {
                    flow,
                    path: path.clone(),
                    names: Names::of(host, &text),
                });
            })
        };
        if let Err(error) = outcome {
            report(path, &error, &names);
            failed = true;
        }
    }
    if failed {
        // Flows already opened are stopped when the host is dropped.
        return Err(Stop::Reported);
    }
    if !check {
        run(host, &started.events, &mut flows, arguments.duration);
    }
    Ok(())
}
