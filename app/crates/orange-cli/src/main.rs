//! `orange`: check and run graph documents from a terminal.
//!
//! This is the smallest front end of `orange-host`. It exists so a graph can be run and
//! controlled without an editor, and as the reference for how any front end attaches.
mod commands;
mod console;
mod credentials;
mod names;
mod updates;

use commands::*;
use console::*;
use credentials::*;
use names::*;
use orange_host::{
    FlowEvent, FlowId, Grants, Host, HostError, HostEvent, HostOptions, NodeId, StopMode,
};
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};
use updates::*;

const USAGE: &str = "\
usage:
  orange check <graph.json>... [--package <file.orangeplug>]... [--grant-log] [--store <dir>]
  orange run   <graph.json>... [--package <file.orangeplug>]... [--grant-log] [--store <dir>] [--for <ms>] [--watch]
  orange bind  <graph.json>... [--package <file.orangeplug>]... [--store <dir>]
  orange preview <graph.json> [<node> <port>]... [--package <file.orangeplug>]... [--store <dir>]
  orange pack  <package-dir> <out.orangeplug>
  orange nodes [--package <file.orangeplug>]... [--store <dir>]
  orange packages [--store <dir>]
  orange enable <package-id> --store <dir>
  orange disable <package-id> --store <dir>
  orange remove <package-id> <version> --store <dir>
  orange updates [--apply] [--signed-only] [--grant-log] --store <dir>
  orange publish <graph.json> <function> <package-id> <version> <node-name> <out.orangeplug>
  orange export <graph.json> <out.orangegraph> --store <dir>
  orange import <in.orangegraph> <graph.json> [--apply] [--store <dir>]
  orange devices
  orange secret set <name> | remove <name> | check <name>
  orange feed key <name> | sign <feed.json> <name>
  orange oauth login <settings.json> <name>

`preview` prints the values of outputs that follow from constants and pure nodes,
without running anything: the outputs named, or those the document exports.

Nodes and variables are shown and accepted by name. A node is called by its `name`
field if the graph gives it one, otherwise by what its kind is called; a variable by
its declared name.

With `--watch`, `run` also prints each variable and each value on a wire when it
changes.

`run` starts every graph as its own flow, numbered from 1 in argument order, and prints
what they do. While it runs, type commands on standard input:

  pause <flow> | resume <flow>      hold a flow, or let it continue
  stop <flow>  | kill <flow>        stop gracefully, or at once
  trigger <flow> [node]             fire a manual entry, by name or ID; the only one if not given
  reload <flow>                     read the flow's file again and apply the edit live
  set <flow> <variable> <value>     set a run variable, by name or ID, to a value such as 7, true or some text
  quit                              stop everything and exit

`bind` completes a graph written by hand: every package node that names only its
`definition_id`, `config` and `inputs` gets the provider, package lock and signature a
graph must record, and the file is saved with the previous version kept as `.bak`.

`pack` builds a package from a directory holding `manifest.json` and the package's
files (`nodes/<name>.json`, `graphs/<name>.json`, a component), filling in its digest.

`--share <id> <adapter> <settings.json>` gives every graph that declares connection
`<id>` as shared the same session, with settings read from the file. The base network
package is built in: adapters `org.orange.net/http`, `org.orange.net/websocket` and
`org.orange.net/mqtt`.

`--store <dir>` keeps installed packages in a directory: packages given with
`--package` are saved there, and later commands with the same `--store` have them
without `--package`.

`packages` lists the base, native and stored packages and whether each is in use.
`disable <id>` leaves a package out from the next command on and `enable <id>` brings
it back; both need `--store`, where the choice is kept. `remove` takes one out of the
store. `updates` reads the update feed each package names and lists newer releases;
with `--apply` it downloads each, checks its digest against the feed and installs it.
A package that names a signing key is updated only from a feed that key signed; with
`--signed-only` a package that names no key is not updated at all. A
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
`--no-audio` the audio package and `--no-files` the file package.

The base file package is built in: a connection with adapter `org.orange.files/folder`
and a `path` setting names a directory, and the file nodes read, write, list and watch
files inside it and nowhere else.

`publish` turns a function of a graph (named by its name or ID) into a package with
one node made of it, which any graph can then use after `--package`.

`export` writes a graph together with the stored packages it uses as one file.
`import` saves the graph from such a file and lists the packages it needs and whether
each is available; with `--apply` it first installs the packages the file carries.

`--state <dir>` (or `state` in the store) keeps the variables a graph declares
`persistent`: they are saved as they change and a later run starts with them. The
files there are JSON and can be edited while nothing runs.

`secret set <name>` reads a credential from standard input and keeps it in the
operating system's credential store; a connection's settings name it as
`credential://<name>` (for example a `bearer` setting of `credential://api/token`), so graph files
hold no secrets. `secret check` says whether one is stored without showing it.

`oauth login <settings.json> <name>` signs you in to a service in a browser
(authorization code with PKCE) and keeps the refresh token in the credential store; an
HTTP connection then uses it as `oauth.refresh_token`. The settings file holds
`authorize_url`, `token_url`, `client_id` and optionally `scope`, `client_secret` and
`redirect_port`.

`feed key <name>` makes a key for signing update feeds, keeps its private half in the
credential store and prints the `update.key` to put in a package's manifest; `feed sign
<feed.json> <name>` writes the signature to serve beside the feed as `<feed.json>.sig`.

`--grant-log` lets installed package code write log lines, `--grant-files` lets it
read and write files in its own data directory (`--plugin-data <dir>`, or `data` in the
store), and `--grant-http` lets it send HTTPS requests. A package receives a grant
only if its manifest asks for it (`permissions`), and an install says what a package
asked for and was not given. A grant is given to the
packages installed by that command and is kept with them in the store. `--for` stops after a time.";

pub(crate) struct Arguments {
    pub command: String,
    pub graphs: Vec<String>,
    pub packages: Vec<String>,
    pub shared: Vec<[String; 3]>,
    pub grants: Grants,
    pub duration: Option<Duration>,
    pub store: Option<String>,
    pub base_nodes: bool,
    pub network: bool,
    pub audio: bool,
    pub files: bool,
    pub apply: bool,
    pub signed_only: bool,
    pub plugins: Vec<String>,
    pub plugin_data: Option<String>,
    pub watch: bool,
    pub state: Option<String>,
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
        files: true,
        apply: false,
        signed_only: false,
        plugins: Vec::new(),
        plugin_data: None,
        watch: false,
        state: None,
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
            "--state" => parsed.state = Some(words.next().ok_or("--state needs a directory")?),
            "--plugin-data" => {
                parsed.plugin_data = Some(words.next().ok_or("--plugin-data needs a directory")?);
            }
            "--no-base" => parsed.base_nodes = false,
            "--no-net" => parsed.network = false,
            "--no-audio" => parsed.audio = false,
            "--no-files" => parsed.files = false,
            "--apply" => parsed.apply = true,
            "--signed-only" => parsed.signed_only = true,
            "--plugins" => parsed
                .plugins
                .push(words.next().ok_or("--plugins needs a directory")?),
            "--store" => parsed.store = Some(words.next().ok_or("--store needs a directory")?),
            "--watch" => parsed.watch = true,
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

fn report(path: &str, error: &HostError, names: &Names) {
    eprintln!("{path}: {error}");
    if let HostError::Invalid(diagnostics) = error {
        for diagnostic in diagnostics {
            eprintln!("  {}", describe(diagnostic, names));
        }
    }
}

fn command(words: &[String]) -> Result<ExitCode, Stop> {
    let (name, rest) = match words {
        [name, rest @ ..] => (name.as_str(), rest),
        [] => return Err(Stop::Usage("missing command".into())),
    };
    // Commands that need no host.
    match name {
        "publish" => return publish(rest).map(|()| ExitCode::SUCCESS),
        "oauth" => return oauth(rest).map(|()| ExitCode::SUCCESS),
        "pack" => return pack(rest).map(|()| ExitCode::SUCCESS),
        "devices" => return devices().map(|()| ExitCode::SUCCESS),
        "feed" => return Ok(feed_command(rest)),
        "secret" => return Ok(secret(rest)),
        "check" | "run" | "nodes" | "packages" | "remove" | "updates" | "enable" | "disable"
        | "bind" | "export" | "import" | "preview" => {}
        _ => return Err(Stop::Usage(format!("unknown command `{name}`"))),
    }
    let arguments = arguments().map_err(Stop::Usage)?;
    let mut started = start(&arguments)?;
    let host = &mut started.host;
    match name {
        "packages" => packages(host),
        "nodes" => nodes(host),
        "bind" => bind(host, &arguments.graphs)?,
        "preview" => preview(host, &arguments.graphs)?,
        "export" => export(host, &arguments.graphs)?,
        "import" => import(host, &arguments)?,
        "enable" | "disable" => switch(host, &arguments)?,
        "remove" => remove(host, &arguments.graphs)?,
        "updates" => {
            return Ok(updates(host, &arguments, &started.plugin_files));
        }
        _ => graphs(&mut started, &arguments)?,
    }
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    let words: Vec<String> = std::env::args().skip(1).collect();
    match command(&words) {
        Ok(code) => code,
        Err(Stop::Usage(message)) => {
            eprintln!("{message}\n{USAGE}");
            ExitCode::from(2)
        }
        Err(Stop::Failed(message)) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
        Err(Stop::Reported) => ExitCode::FAILURE,
    }
}
