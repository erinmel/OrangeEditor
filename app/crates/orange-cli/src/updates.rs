//! The `updates` command: reading update feeds and installing newer releases.
use crate::*;

/// Largest release archive `updates --apply` downloads.
pub(crate) const MAX_RELEASE_BYTES: u64 = 256 * 1024 * 1024;

/// Downloads a package's update feed. When the package names a signing key, the feed's
/// signature is downloaded too and the feed is refused unless the key signed it.
pub(crate) fn feed(source: &orange_host::UpdateSource) -> Result<orange_host::UpdateFeed, String> {
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
pub(crate) fn updates(
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
