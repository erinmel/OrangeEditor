//! The package side of a host: installing, listing, switching and removing packages,
//! and checking the graphs a package is made of.
use super::*;

impl Host {
    /// Installs a `.orangeplug` archive and makes its nodes available to documents
    /// opened afterwards. Running flows keep the providers they were compiled with.
    ///
    /// Each contributed node `<package>/<name>` is described by the archive file
    /// `nodes/<name>.json`. A package provider becomes the choice for new nodes only
    /// where no provider is chosen yet; an existing choice is never replaced here.
    /// `grants` are the host capabilities the user approved for the package's code.
    ///
    /// With [`HostOptions::package_dir`] the archive is then saved. If saving fails the
    /// package stays installed for this session and the error says it was not stored.
    pub fn install(&mut self, archive: &[u8], grants: Grants) -> Result<PackageKey, HostError> {
        self.install_expected(archive, grants, None)
    }

    /// What a package asks to be granted, read from the `permissions` section of its
    /// manifest without installing anything: what an install dialog shows the user.
    /// [`install`](Self::install) refuses a grant the package did not ask for, so a
    /// package never holds more than it declared and the user approved.
    pub fn permissions(&self, archive: &[u8]) -> Result<Grants, HostError> {
        let mut files = MemorySink::new();
        let verified = read_archive(archive, &ArchiveLimits::default(), &mut files)
            .map_err(|error| HostError::Archive(error.to_string()))?;
        asked(verified.manifest())
    }

    /// Installs a package like [`install`](Self::install), but only if its content
    /// digest is `content_digest` (`sha256:<64 hex>`): the check that a downloaded
    /// archive is the one an update feed announced.
    pub fn install_release(
        &mut self,
        archive: &[u8],
        grants: Grants,
        content_digest: &str,
    ) -> Result<PackageKey, HostError> {
        self.install_expected(archive, grants, Some(content_digest))
    }

    /// Registers the types an archive declares, each a file `types/<name>.json` that
    /// defines `<package id>/<name>@1`:
    /// `{"kind": "record", "fields": [{"id": "x", "type": "core/f64@1", "optional": false}]}`
    /// or `{"kind": "enum", "variants": [{"id": "left"}, {"id": "at", "payload": "<type>"}]}`.
    /// Returns whether a generation with new types was published.
    fn publish_archive_types(&mut self, archive: &[u8]) -> Result<bool, HostError> {
        let mut files = MemorySink::new();
        let verified = read_archive(archive, &ArchiveLimits::default(), &mut files)
            .map_err(|error| HostError::Archive(error.to_string()))?;
        let id = verified.manifest().key().id.as_str().to_owned();
        let mut declarations = Vec::new();
        for (path, bytes) in files.files() {
            let Some(name) = path
                .as_str()
                .strip_prefix("types/")
                .and_then(|file| file.strip_suffix(".json"))
            else {
                continue;
            };
            verified
                .verify_file(path, bytes)
                .map_err(|error| HostError::Archive(error.to_string()))?;
            let refused =
                |what: &str| HostError::Definition(format!("`{}`: {what}", path.as_str()));
            let file: serde_json::Value =
                serde_json::from_slice(bytes).map_err(|error| refused(&error.to_string()))?;
            let text = |value: &serde_json::Value, key: &str| {
                value[key]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| refused(&format!("`{key}` must be text")))
            };
            let list = |key: &str| {
                file[key]
                    .as_array()
                    .ok_or_else(|| refused(&format!("`{key}` must be a list")))
            };
            declarations.push(match file["kind"].as_str() {
                Some("record") => TypeDecl::Record {
                    name: name.to_owned(),
                    fields: list("fields")?
                        .iter()
                        .map(|field| {
                            let optional = field["optional"].as_bool().unwrap_or(false);
                            Ok((text(field, "id")?, text(field, "type")?, optional))
                        })
                        .collect::<Result<_, HostError>>()?,
                },
                Some("enum") => TypeDecl::Enum {
                    name: name.to_owned(),
                    variants: list("variants")?
                        .iter()
                        .map(|variant| {
                            let payload = match &variant["payload"] {
                                serde_json::Value::Null => None,
                                _ => Some(text(variant, "payload")?),
                            };
                            Ok((text(variant, "id")?, payload))
                        })
                        .collect::<Result<_, HostError>>()?,
                },
                _ => return Err(refused("`kind` must be `record` or `enum`")),
            });
        }
        if declarations.is_empty() {
            return Ok(false);
        }
        let mut types = self.store.current().types().extended();
        for declaration in &declarations {
            register_type(&mut types, &id, declaration).map_err(HostError::Definition)?;
        }
        let types = types
            .build()
            .map_err(|error| HostError::Definition(error.to_string()))?;
        let _ = self
            .store
            .publish_types(Arc::new(types))
            .map_err(|error| HostError::Install(error.to_string()))?;
        Ok(true)
    }

    /// Installs an archive: its types first, then its nodes. A failure after the types
    /// were published takes them back, so a refused package leaves nothing behind.
    pub(super) fn install_archive(
        &mut self,
        archive: &[u8],
        grants: Grants,
        expected: Option<&str>,
    ) -> Result<PackageKey, HostError> {
        let published = self.publish_archive_types(archive)?;
        let installed = self.install_archive_nodes(archive, grants, expected);
        if installed.is_err() && published {
            let _ = self.store.rollback();
        }
        installed
    }

    pub(super) fn install_expected(
        &mut self,
        archive: &[u8],
        grants: Grants,
        expected: Option<&str>,
    ) -> Result<PackageKey, HostError> {
        let key = self.install_archive(archive, grants, expected)?;
        if let Some(directory) = &self.options.package_dir {
            persist::save(directory, archive, grants, &key).map_err(HostError::Storage)?;
        }
        // Installing a package is asking for it: it is enabled again.
        self.set_enabled(key.id.as_str(), true)?;
        Ok(key)
    }

    /// Chooses whether hosts started from now on use package `id`: the base nodes
    /// ([`BASE_PACKAGE`]), a native service package or an installed package. Nothing
    /// changes in this host; [`PackageInfo::enabled`] shows the choice and
    /// [`PackageInfo::active`] what is in use. With [`HostOptions::package_dir`] the
    /// choice is saved there; without one it lasts as long as this host and has no
    /// effect. A document that uses nodes of a package that is left out does not open.
    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> Result<(), HostError> {
        let changed = if enabled {
            self.disabled.remove(id)
        } else {
            self.disabled.insert(id.to_owned())
        };
        if changed && let Some(directory) = &self.options.package_dir {
            persist::set_disabled(directory, &self.disabled).map_err(HostError::Storage)?;
        }
        Ok(())
    }

    /// The packages of this host: the base nodes, the native service packages it was
    /// started with, then every installed package, including those left out because
    /// they are disabled. This is the data of a package manager view.
    pub fn packages(&self) -> impl Iterator<Item = PackageInfo<'_>> {
        let built_in = self.built_in.iter().map(|package| PackageInfo {
            id: &package.id,
            version: &package.version,
            kind: package.kind,
            active: package.active,
            enabled: !self.disabled.contains(&package.id),
            manifest: None,
            name: package.name.as_deref(),
            description: package.description.as_deref(),
            update: package.update.as_ref(),
        });
        let inactive = self.inactive.iter().map(|(id, version)| PackageInfo {
            id,
            version,
            kind: PackageKind::Installed,
            active: false,
            enabled: !self.disabled.contains(id),
            manifest: None,
            name: None,
            description: None,
            update: None,
        });
        let installed = self
            .store
            .packages()
            .filter(|record| record.state() == PackageState::Active)
            .map(|record| {
                let manifest = record.manifest();
                PackageInfo {
                    id: manifest.key().id.as_str(),
                    version: manifest.key().version.as_str(),
                    kind: PackageKind::Installed,
                    active: true,
                    enabled: !self.disabled.contains(manifest.key().id.as_str()),
                    manifest: Some(manifest),
                    name: None,
                    description: None,
                    update: None,
                }
            });
        built_in.chain(installed).chain(inactive)
    }

    /// Removes an installed package. Its nodes can no longer be placed, and a document
    /// that uses them no longer opens; flows already running keep the code they
    /// started with. A package another installed package depends on is refused. With
    /// [`HostOptions::package_dir`] it is also removed from the directory, so it stays
    /// removed when the host next starts. A saved package that was left out because it
    /// is disabled is removed from the directory.
    pub fn uninstall(&mut self, key: &PackageKey) -> Result<(), HostError> {
        let left_out = self
            .inactive
            .iter()
            .position(|(id, version)| id == key.id.as_str() && version == key.version.as_str());
        if let Some(index) = left_out {
            let _ = self.inactive.remove(index);
        } else {
            let _ = self
                .store
                .remove(key)
                .map_err(|error| HostError::Uninstall(error.to_string()))?;
        }
        if let Some(directory) = &self.options.package_dir {
            persist::forget(directory, key).map_err(HostError::Storage)?;
        }
        Ok(())
    }

    /// Saved packages that could not be installed again when this host started, by
    /// archive file name. The others are installed; these stay on disk untouched.
    pub fn not_restored(&self) -> &[(String, HostError)] {
        &self.not_restored
    }

    fn install_archive_nodes(
        &mut self,
        archive: &[u8],
        grants: Grants,
        expected: Option<&str>,
    ) -> Result<PackageKey, HostError> {
        let mut files = MemorySink::new();
        let verified = read_archive(archive, &ArchiveLimits::default(), &mut files)
            .map_err(|error| HostError::Archive(error.to_string()))?;
        let manifest = verified.manifest().clone();
        let key = manifest.key().clone();
        let asked = asked(&manifest)?;
        for (name, given, wanted) in [
            ("log", grants.log, asked.log),
            ("files", grants.files, asked.files),
            ("http", grants.http, asked.http),
        ] {
            if given && !wanted {
                return Err(HostError::Install(format!(
                    "the package does not ask for the `{name}` grant"
                )));
            }
        }

        let content_digest = verified.content_digest().to_owned();
        if expected.is_some_and(|expected| {
            let bare = |digest: &'_ str| digest.strip_prefix("sha256:").map(str::to_owned);
            bare(expected).unwrap_or_else(|| expected.to_owned())
                != bare(&content_digest).unwrap_or_else(|| content_digest.clone())
        }) {
            return Err(HostError::Archive(
                "its content digest is not the expected one".into(),
            ));
        }
        let read = |path: &str| -> Result<&str, HostError> {
            let missing = || HostError::MissingDefinition(path.to_owned());
            let archive_path: ArchivePath = path.parse().map_err(|_| missing())?;
            let bytes = files.file(path).ok_or_else(missing)?;
            verified
                .verify_file(&archive_path, bytes)
                .map_err(|error| HostError::Archive(error.to_string()))?;
            std::str::from_utf8(bytes).map_err(|error| HostError::Definition(error.to_string()))
        };
        let local = |name: &str| name.rsplit('/').next().unwrap_or(name).to_owned();

        let mut definitions = Vec::with_capacity(manifest.nodes().len());
        let mut described = Vec::with_capacity(manifest.nodes().len());
        let mut definition_text = BTreeMap::new();
        for node in manifest.nodes() {
            let name = node.to_string();
            let path = format!("nodes/{}.json", local(&name));
            let text = read(&path)?;
            let (definition, presentation) = NodeDefinition::described(text)
                .map_err(|error| HostError::Definition(error.to_string()))?;
            described.push((definition.definition.clone(), presentation));
            if definition.definition.id != *node {
                return Err(HostError::Definition(format!(
                    "`{path}` does not define `{name}`"
                )));
            }
            let _ = definition_text.insert((name, u64::from(definition.definition.version)), text);
            definitions.push(definition);
        }

        // Prepare every implementation before anything is published, so a refused
        // component or graph leaves the registry untouched.
        let provider_key = |id: &str| ProviderKey {
            package: key.id.as_str().to_owned(),
            version: key.version.as_str().to_owned(),
            content_digest: content_digest.clone(),
            id: id.to_owned(),
        };
        let mut guests: Vec<GuestPackage> = Vec::new();
        let mut graphs: Vec<(graphs::GraphKey, Arc<GraphNode>)> = Vec::new();
        for provider in manifest.providers() {
            if provider.id == GRAPH_PROVIDER {
                let ProviderSlot::Node(definition) = &provider.slot else {
                    continue;
                };
                let name = definition.id.to_string();
                let version = u64::from(definition.version);
                let path = format!("graphs/{}.json", local(&name));
                let text = read(&path)?;
                let contract = definition_text
                    .get(&(name.clone(), version))
                    .ok_or_else(|| HostError::MissingDefinition(name.clone()))?;
                let graph = GraphNode::decode(text, contract)
                    .map_err(|reason| HostError::Graph(name.clone(), reason))?;
                let graph_key = (
                    key.id.as_str().to_owned(),
                    key.version.as_str().to_owned(),
                    name,
                    version,
                );
                graphs.push((graph_key, Arc::new(graph)));
            } else if matches!(manifest.execution(), Some(Execution::WasmComponent { .. }))
                && guests
                    .iter()
                    .all(|guest| guest.provider().id != provider.id)
            {
                let access = GuestAccess {
                    data_dir: self
                        .options
                        .plugin_data_dir
                        .as_ref()
                        .map(|root| root.join(key.id.as_str())),
                    http: self.options.plugin_http.clone(),
                    max_data_bytes: self.options.plugin_data_quota,
                    http_hosts: self.options.plugin_http_hosts.clone(),
                };
                guests.push(
                    GuestPackage::from_archive(
                        &self.engine,
                        &verified,
                        &files,
                        &provider.id,
                        grants,
                    )?
                    .with_access(access),
                );
            }
        }

        let install = |error: &dyn std::fmt::Display| HostError::Install(error.to_string());
        let contents = verified.into_contents(definitions);

        // A graph is checked as it will be used: linked with the graphs it builds on,
        // against a registry that includes this package's own definitions. The check
        // runs on a throwaway store first, because publishing a version is permanent:
        // a package refused here can be corrected and installed under the same version.
        let mut known = self.graphs.clone();
        known.extend(graphs.iter().cloned());
        let bodies: Vec<_> = graphs.iter().map(|(_, graph)| Arc::clone(graph)).collect();
        let mut checked = bodies.is_empty();
        if !checked {
            let mut trial = PackageStore::new(
                Arc::clone(self.store.current()),
                HOST_API_VERSION,
                MemoryJournal::new(),
            );
            // A package the throwaway store cannot take (it knows no installed
            // package, so a declared dependency is missing) is checked after the real
            // installation instead.
            if trial.install(contents.clone()).is_ok() {
                self.check_graphs(key.id.as_str(), &bodies, &known, trial.current())?;
                checked = true;
            }
        }
        // A newer version of an installed package succeeds it: both stay published, so
        // documents locked to the old one still open, and newly placed nodes use the
        // new one.
        let previous = self
            .store
            .packages()
            .filter(|record| record.state() == PackageState::Active)
            .map(|record| record.manifest().key())
            .filter(|active| active.id == key.id && active.version != key.version)
            .max_by(|a, b| orange_packages::precedence(&a.version, &b.version))
            .cloned();
        let _ = match previous {
            Some(previous) => self.store.update(&previous, contents),
            None => self.store.install(contents),
        }
        .map_err(|e| install(&e))?;
        if !checked
            && let Err(error) =
                self.check_graphs(key.id.as_str(), &bodies, &known, self.store.current())
        {
            let _ = self.store.rollback();
            return Err(error);
        }

        for provider in manifest.providers() {
            let ProviderSlot::Node(definition) = &provider.slot else {
                continue;
            };
            let implemented = provider.id == GRAPH_PROVIDER
                || guests
                    .iter()
                    .any(|guest| guest.provider().id == provider.id);
            if implemented
                && self
                    .store
                    .current()
                    .new_binding_default(definition)
                    .is_none()
            {
                let _ = self
                    .store
                    .select_default(definition.clone(), provider_key(&provider.id))
                    .map_err(|e| install(&e))?;
            }
        }
        self.guests.extend(guests);
        self.graphs.extend(graphs);
        self.presentations.extend(described);
        Ok(key)
    }

    pub(super) fn check_graphs(
        &self,
        package: &str,
        bodies: &[Arc<GraphNode>],
        known: &Graphs,
        registry: &Arc<RegistrySnapshot>,
    ) -> Result<(), HostError> {
        let refused = |reason: String| HostError::Graph(package.to_owned(), reason);
        let empty = to_json(&GraphDocument::new(
            SCRATCH_GRAPH
                .parse()
                .map_err(|_| refused("invalid validation graph id".into()))?,
        ))
        .map_err(|error| refused(error.to_string()))?;
        let mut scratch = graphs::scratch(&empty, bodies)
            .ok_or_else(|| refused("could not build a validation document".into()))?;
        graphs::link(&mut scratch, known);
        let document =
            from_json(&scratch.to_string()).map_err(|error| refused(error.to_string()))?;
        let analysis = analyzed(&document, registry);
        if analysis.is_ready_for_lowering(&document, registry) {
            Ok(())
        } else {
            Err(HostError::GraphInvalid(
                package.to_owned(),
                analysis.report().diagnostics.clone(),
            ))
        }
    }
}
