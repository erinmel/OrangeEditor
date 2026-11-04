//! The package store: install, update, remove, revoke and roll back as registry transactions
//! over immutable [`RegistrySnapshot`] generations (04, ADR-07, 12 §8).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use orange_model::catalog::{
    NodeDefinition, ProviderDescriptor, ProviderKey, RegistryBuilder, RegistryError,
    RegistrySnapshot, TypeArgs,
};
use orange_model::document::DefinitionRef;
use orange_model::id::DefinitionId;
use thiserror::Error;

use crate::journal::{Journal, JournalEntry, JournalError, MemoryJournal, Recovery};
use crate::lifecycle::PackageState;
use crate::manifest::{
    HostApiRange, HostApiVersion, PackageKey, PackageManifest, ProviderSlot, SlotKind,
};

/// A staged package: its validated manifest, the node definitions it ships and the argument
/// sets its generic providers support. Archive unpacking is host work; definition files
/// decode with [`NodeDefinition::from_json`].
#[derive(Debug, Clone, PartialEq)]
pub struct PackageContents {
    manifest: PackageManifest,
    definitions: Vec<NodeDefinition>,
    specializations: BTreeMap<(String, DefinitionRef), Vec<TypeArgs>>,
}

impl PackageContents {
    /// Pairs a manifest with the definitions it declares under `contributions.nodes`.
    pub fn new(manifest: PackageManifest, definitions: Vec<NodeDefinition>) -> Self {
        Self {
            manifest,
            definitions,
            specializations: BTreeMap::new(),
        }
    }

    /// Restricts the package's `provider` for `definition` to the `supported` argument sets
    /// (11 §5.5), replacing any earlier list for that pair. The pair must be a declared
    /// `node:` provider contribution. Without a list the provider delegates to constraints;
    /// an inferred instantiation outside a list is `E-NO-SPECIALIZATION`.
    pub fn with_specializations(
        mut self,
        provider: impl Into<String>,
        definition: DefinitionRef,
        supported: Vec<TypeArgs>,
    ) -> Self {
        self.specializations
            .insert((provider.into(), definition), supported);
        self
    }

    /// The argument sets `provider` supports for `definition`, if the package lists them.
    pub fn specializations(
        &self,
        provider: &str,
        definition: &DefinitionRef,
    ) -> Option<&[TypeArgs]> {
        self.specializations
            .get(&(provider.to_owned(), definition.clone()))
            .map(Vec::as_slice)
    }

    /// The validated manifest.
    pub fn manifest(&self) -> &PackageManifest {
        &self.manifest
    }

    /// The shipped node definitions.
    pub fn definitions(&self) -> &[NodeDefinition] {
        &self.definitions
    }
}

/// What the store knows about one package version. A removed or revoked record is the
/// version's tombstone: it keeps the manifest and exact provider keys so a reinstall with a
/// different digest is refused and diagnostics can name what existing locks refer to.
#[derive(Debug, Clone, PartialEq)]
pub struct PackageRecord {
    manifest: Arc<PackageManifest>,
    state: PackageState,
    providers: Vec<ProviderKey>,
    generation: u64,
}

impl PackageRecord {
    /// The version's manifest.
    pub fn manifest(&self) -> &PackageManifest {
        &self.manifest
    }

    /// Active, Removed or Revoked.
    pub fn state(&self) -> PackageState {
        self.state
    }

    /// Exact provider keys the version published, sorted.
    pub fn providers(&self) -> &[ProviderKey] {
        &self.providers
    }

    /// The registry generation that last changed this record.
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

/// Why a store transaction did not publish. The previous generation stays current.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum TransactionError {
    /// The exact version is already active.
    #[error("package `{0}` is already active")]
    AlreadyActive(PackageKey),
    /// The store has no record of the version.
    #[error("package `{0}` is not installed")]
    NotInstalled(PackageKey),
    /// The version exists but is not in a state the transaction accepts.
    #[error("package `{key}` is {state:?}")]
    WrongState {
        /// The package version.
        key: PackageKey,
        /// Its current state.
        state: PackageState,
    },
    /// An update must move between two versions of one package.
    #[error("cannot update `{from}` to `{to}`")]
    UpdateTarget {
        /// The installed version.
        from: PackageKey,
        /// The offered version.
        to: PackageKey,
    },
    /// The version's content digest was revoked.
    #[error("package `{0}` was revoked")]
    Revoked(PackageKey),
    /// The version was installed before with another content digest; versions are immutable.
    #[error("package `{0}` was installed with a different content digest")]
    DigestConflict(PackageKey),
    /// The running host API is outside the package's declared range.
    #[error("host API {}.{} is outside the package range", .host.major, .host.minor)]
    HostApi {
        /// The package's range.
        required: HostApiRange,
        /// The running host API.
        host: HostApiVersion,
    },
    /// An exact dependency is not active.
    #[error("dependency `{0}` is not active")]
    MissingDependency(PackageKey),
    /// A shipped definition is not listed in `contributions.nodes`.
    #[error("definition `{}@{}` is not declared by the manifest", .0.id, .0.version)]
    UndeclaredDefinition(DefinitionRef),
    /// A node listed in `contributions.nodes` has no shipped definition.
    #[error("declared node `{0}` has no definition")]
    MissingDefinition(DefinitionId),
    /// A shipped definition differs from the one already registered under its exact
    /// version; changing a contract needs a new definition version (04).
    #[error("definition `{}@{}` differs from the registered one", .0.id, .0.version)]
    DefinitionChanged(DefinitionRef),
    /// A node slot names a definition that is neither shipped nor registered.
    #[error("provider slot names unknown definition `{}@{}`", .0.id, .0.version)]
    UnknownSlotDefinition(DefinitionRef),
    /// Specializations name a provider and definition that no `node:` contribution of the
    /// manifest pairs.
    #[error("provider `{provider}` declares no slot for `{}@{}`", .definition.id, .definition.version)]
    UndeclaredSpecialization {
        /// The package-local provider ID.
        provider: String,
        /// The specialized definition.
        definition: DefinitionRef,
    },
    /// The registry does not publish this slot kind yet.
    #[error("slot kind {kind:?} (`{descriptor}`) is not publishable yet")]
    UnsupportedSlot {
        /// The slot kind.
        kind: SlotKind,
        /// The slot descriptor.
        descriptor: String,
    },
    /// Another active version depends on the one being removed.
    #[error("package `{package}` is required by `{dependent}`")]
    DependedOn {
        /// The version being removed.
        package: PackageKey,
        /// An active version that depends on it.
        dependent: PackageKey,
    },
    /// No earlier generation is available to restore.
    #[error("no earlier registry generation to roll back to")]
    NothingToRollBack,
    /// Restored contents do not match the recovered committed set: a version is missing,
    /// not committed active, or has another digest.
    #[error("restored package `{0}` does not match the last committed generation")]
    RecoveryMismatch(PackageKey),
    /// Registry publication rejected the next generation.
    #[error(transparent)]
    Registry(#[from] RegistryError),
    /// The journal could not record a step.
    #[error(transparent)]
    Journal(#[from] JournalError),
}

/// One committed state: a published registry generation and the package records it implies.
#[derive(Debug, Clone)]
struct Committed {
    snapshot: Arc<RegistrySnapshot>,
    packages: BTreeMap<PackageKey, PackageRecord>,
}

impl Committed {
    fn active(&self) -> Vec<PackageKey> {
        self.packages
            .iter()
            .filter(|(_, record)| record.state.is_active())
            .map(|(key, _)| key.clone())
            .collect()
    }
}

/// Installed package versions over a sequence of immutable registry generations.
///
/// Every successful transaction builds the next [`RegistrySnapshot`] from the current one
/// with [`RegistryBuilder::from_snapshot`], journals the commit, then makes it current. A
/// failed transaction publishes nothing: the previous snapshot stays current and readers
/// holding any earlier snapshot keep a complete view. The store never edits a document:
/// existing locks keep their exact provider, and a removed one resolves as unavailable.
#[derive(Debug)]
pub struct PackageStore<J = MemoryJournal> {
    host_api: HostApiVersion,
    current: Committed,
    /// Earlier committed states, oldest first; [`rollback`](Self::rollback) restores the last.
    history: Vec<Committed>,
    /// The last failed attempt per version; never kept for a version that is active.
    quarantined: BTreeMap<PackageKey, TransactionError>,
    /// The content digest of every version ever published. Rollback never removes an entry:
    /// a version names one immutable build for good.
    published: BTreeMap<PackageKey, String>,
    /// Revoked content digests, which no transaction or rollback can reinstate.
    revoked: BTreeSet<String>,
    journal: J,
}

impl<J: Journal> PackageStore<J> {
    /// A store whose first generation is `base`, such as the core or bundled snapshot.
    pub fn new(base: Arc<RegistrySnapshot>, host_api: HostApiVersion, journal: J) -> Self {
        Self {
            host_api,
            current: Committed {
                snapshot: base,
                packages: BTreeMap::new(),
            },
            history: Vec::new(),
            quarantined: BTreeMap::new(),
            published: BTreeMap::new(),
            revoked: BTreeSet::new(),
            journal,
        }
    }

    /// Rebuilds a store after a restart from `recovery` and the staged contents of exactly
    /// the versions its last commit made active. Published digests and revocations are
    /// restored, so removed versions keep their identity and revoked digests stay blocked;
    /// removed versions have no record, since their manifests are not reloaded. Active
    /// versions are reinstalled in dependency order and journaled to `journal`. The rebuilt
    /// store has no rollback history.
    pub fn restore(
        base: Arc<RegistrySnapshot>,
        host_api: HostApiVersion,
        journal: J,
        recovery: &Recovery,
        contents: Vec<PackageContents>,
    ) -> Result<Self, TransactionError> {
        let mut store = Self::new(base, host_api, journal);
        store.published = recovery.published.clone();
        store.revoked = recovery.revoked.clone();
        let active = recovery.committed.as_deref().unwrap_or_default();
        let mut pending = Vec::with_capacity(contents.len());
        for item in contents {
            let key = item.manifest.key();
            let expected = store.published.get(key).map(String::as_str);
            if !active.contains(key) || expected != Some(item.manifest.content_digest()) {
                return Err(TransactionError::RecoveryMismatch(key.clone()));
            }
            pending.push(item);
        }
        if let Some(missing) = active
            .iter()
            .find(|key| !pending.iter().any(|item| item.manifest.key() == *key))
        {
            return Err(TransactionError::RecoveryMismatch(missing.clone()));
        }
        while !pending.is_empty() {
            let ready = pending.iter().position(|item| {
                item.manifest.dependencies().iter().all(|dependency| {
                    store
                        .current
                        .packages
                        .keys()
                        .any(|key| key.id == dependency.id && key.version == dependency.version)
                })
            });
            // With no installable version left, install the first to report its error.
            let item = pending.remove(ready.unwrap_or(0));
            store.install(item)?;
        }
        store.history.clear();
        Ok(store)
    }

    /// The content digest a version was published with, including removed versions.
    pub fn published_digest(&self, key: &PackageKey) -> Option<&str> {
        self.published.get(key).map(String::as_str)
    }

    /// Whether `content_digest` was revoked.
    pub fn is_revoked(&self, content_digest: &str) -> bool {
        self.revoked.contains(content_digest)
    }

    /// The current registry generation.
    pub fn current(&self) -> &Arc<RegistrySnapshot> {
        &self.current.snapshot
    }

    /// The record of one package version, including tombstones.
    pub fn package(&self, key: &PackageKey) -> Option<&PackageRecord> {
        self.current.packages.get(key)
    }

    /// Every recorded package version, sorted by key.
    pub fn packages(&self) -> impl Iterator<Item = &PackageRecord> {
        self.current.packages.values()
    }

    /// Why the last attempt to install `key` was quarantined, if it was.
    pub fn quarantined(&self, key: &PackageKey) -> Option<&TransactionError> {
        self.quarantined.get(key)
    }

    /// Whether [`rollback`](Self::rollback) has a generation to restore.
    pub fn can_roll_back(&self) -> bool {
        !self.history.is_empty()
    }

    /// The journal.
    pub fn journal(&self) -> &J {
        &self.journal
    }

    /// Installs a new package version and publishes its definitions and providers in the
    /// next generation. No new-binding default is chosen: there is no last-installed-wins.
    /// On failure the version is quarantined and the current generation is unchanged.
    pub fn install(
        &mut self,
        contents: PackageContents,
    ) -> Result<Arc<RegistrySnapshot>, TransactionError> {
        self.activate(contents, None)
    }

    /// Installs `contents` as the successor of the active version `from`. Both versions stay
    /// published, so documents locked to `from` keep resolving; new-binding defaults that
    /// selected a provider of `from` move to the provider with the same ID in the new version
    /// when it implements the same definition. Retiring `from` is a separate
    /// [`remove`](Self::remove); moving a document to the new version is an explicit rebind.
    pub fn update(
        &mut self,
        from: &PackageKey,
        contents: PackageContents,
    ) -> Result<Arc<RegistrySnapshot>, TransactionError> {
        let to = contents.manifest.key();
        if from.id != to.id || from.version == to.version {
            return Err(TransactionError::UpdateTarget {
                from: from.clone(),
                to: to.clone(),
            });
        }
        self.require_active(from)?;
        self.activate(contents, Some(from.clone()))
    }

    /// Selects `provider` for new bindings of `definition` (12 §8). Existing document locks
    /// are never changed.
    pub fn select_default(
        &mut self,
        definition: DefinitionRef,
        provider: ProviderKey,
    ) -> Result<Arc<RegistrySnapshot>, TransactionError> {
        let snapshot = RegistryBuilder::from_snapshot(&self.current.snapshot)
            .new_binding_default(definition, provider)
            .build()?;
        let packages = self.current.packages.clone();
        self.commit(snapshot, packages, None)
    }

    /// Hides an active version: its providers leave the next generation and any default
    /// selecting them is cleared. The record stays as a Removed tombstone. Documents are not
    /// rebound; nodes locked to it resolve as provider-unavailable. Its definitions stay
    /// registered as passive metadata. Refused while another active version depends on it.
    pub fn remove(&mut self, key: &PackageKey) -> Result<Arc<RegistrySnapshot>, TransactionError> {
        self.require_active(key)?;
        if let Some(dependent) =
            self.current.packages.values().find(|record| {
                record.state.is_active()
                    && record.manifest.dependencies().iter().any(|dependency| {
                        dependency.id == key.id && dependency.version == key.version
                    })
            })
        {
            return Err(TransactionError::DependedOn {
                package: key.clone(),
                dependent: dependent.manifest.key().clone(),
            });
        }
        self.retire(std::slice::from_ref(key), PackageState::Removed, None)
    }

    /// Revokes the content digest of a published version. Revocation is immediate policy
    /// enforcement on the bytes (04), so every active or removed version published with that
    /// digest leaves the registry and becomes Revoked, the digest can no longer be installed,
    /// and earlier generations are discarded so rollback cannot reinstate it. Dependents are
    /// not protected. A version known only from [`restore`](Self::restore) is revoked by
    /// digest without a record.
    pub fn revoke(&mut self, key: &PackageKey) -> Result<Arc<RegistrySnapshot>, TransactionError> {
        let digest = self
            .published
            .get(key)
            .cloned()
            .ok_or_else(|| TransactionError::NotInstalled(key.clone()))?;
        if let Some(record) = self.current.packages.get(key)
            && !record.state.can_become(PackageState::Revoked)
        {
            return Err(TransactionError::WrongState {
                key: key.clone(),
                state: record.state,
            });
        }
        let targets: Vec<PackageKey> = self
            .current
            .packages
            .iter()
            .filter(|(_, record)| {
                record.manifest.content_digest() == digest
                    && record.state.can_become(PackageState::Revoked)
            })
            .map(|(key, _)| key.clone())
            .collect();
        let snapshot = self.retire(&targets, PackageState::Revoked, Some(&digest))?;
        self.history.clear();
        Ok(snapshot)
    }

    /// Restores the previous generation and its default selections. The restored snapshot
    /// is the same `Arc` that was current before; plans built against the abandoned
    /// generation see a different generation and are stale. Version identity is not rolled
    /// back: a version the restored generation lacks stays recorded as a Removed tombstone
    /// with its digest, and a quarantine entry is dropped for any version made active again.
    /// Already committed external effects are not undone.
    pub fn rollback(&mut self) -> Result<Arc<RegistrySnapshot>, TransactionError> {
        let mut previous = self
            .history
            .pop()
            .ok_or(TransactionError::NothingToRollBack)?;
        let generation = previous.snapshot.generation();
        let mut retired = Vec::new();
        let mut kept = Vec::new();
        for (key, record) in &self.current.packages {
            if previous.packages.contains_key(key) {
                continue;
            }
            kept.push(key.clone());
            let tombstone = if record.state.is_active() {
                retired.push(key.clone());
                PackageRecord {
                    state: PackageState::Removed,
                    generation,
                    ..record.clone()
                }
            } else {
                record.clone()
            };
            previous.packages.insert(key.clone(), tombstone);
        }
        let reactivated: Vec<PackageKey> = previous
            .packages
            .iter()
            .filter(|(key, record)| {
                record.state.is_active()
                    && self
                        .current
                        .packages
                        .get(*key)
                        .is_some_and(|current| !current.state.is_active())
            })
            .map(|(key, _)| key.clone())
            .collect();

        let journaled = retired
            .iter()
            .try_for_each(|key| self.transition(key, PackageState::Removed))
            .and_then(|()| self.journal_commit(&previous, None));
        if let Err(error) = journaled {
            previous.packages.retain(|key, _| !kept.contains(key));
            self.history.push(previous);
            return Err(error.into());
        }
        self.current = previous;
        for key in &reactivated {
            self.quarantined.remove(key);
            // The commit is durable; a lost activation record cannot undo it.
            let _ = self.transition(key, PackageState::Active);
        }
        Ok(Arc::clone(&self.current.snapshot))
    }

    fn require_active(&self, key: &PackageKey) -> Result<&PackageRecord, TransactionError> {
        let record = self
            .current
            .packages
            .get(key)
            .ok_or_else(|| TransactionError::NotInstalled(key.clone()))?;
        if record.state.is_active() {
            Ok(record)
        } else {
            Err(TransactionError::WrongState {
                key: key.clone(),
                state: record.state,
            })
        }
    }

    fn transition(
        &mut self,
        package: &PackageKey,
        state: PackageState,
    ) -> Result<(), JournalError> {
        self.journal.append(JournalEntry::Transition {
            package: package.clone(),
            state,
        })
    }

    /// Journals `next` as committed: its active set plus the store's identity state, with
    /// `next`'s versions published and `revoke` added to the revoked digests. On success the
    /// store adopts that identity state.
    fn journal_commit(
        &mut self,
        next: &Committed,
        revoke: Option<&str>,
    ) -> Result<(), JournalError> {
        let mut published = self.published.clone();
        for (key, record) in &next.packages {
            published
                .entry(key.clone())
                .or_insert_with(|| record.manifest.content_digest().to_owned());
        }
        let mut revoked = self.revoked.clone();
        revoked.extend(revoke.map(str::to_owned));
        self.journal.append(JournalEntry::Committed {
            generation: next.snapshot.generation(),
            active: next.active(),
            published: published.clone(),
            revoked: revoked.clone(),
        })?;
        self.published = published;
        self.revoked = revoked;
        Ok(())
    }

    /// Journals `packages` as committed with `snapshot`, then makes them current.
    fn commit(
        &mut self,
        snapshot: Arc<RegistrySnapshot>,
        packages: BTreeMap<PackageKey, PackageRecord>,
        revoke: Option<&str>,
    ) -> Result<Arc<RegistrySnapshot>, TransactionError> {
        let next = Committed { snapshot, packages };
        self.journal_commit(&next, revoke)?;
        let previous = std::mem::replace(&mut self.current, next);
        self.history.push(previous);
        Ok(Arc::clone(&self.current.snapshot))
    }

    fn activate(
        &mut self,
        contents: PackageContents,
        from: Option<PackageKey>,
    ) -> Result<Arc<RegistrySnapshot>, TransactionError> {
        let key = contents.manifest.key().clone();
        if let Some(record) = self.current.packages.get(&key)
            && record.state.is_active()
        {
            return Err(TransactionError::AlreadyActive(key));
        }
        match self.attempt(&key, contents, from) {
            Ok(snapshot) => {
                self.quarantined.remove(&key);
                Ok(snapshot)
            }
            Err(error) => {
                // The attempt already failed; a journal error here cannot change the outcome.
                let _ = self.transition(&key, PackageState::Quarantined);
                self.quarantined.insert(key, error.clone());
                Err(error)
            }
        }
    }

    fn attempt(
        &mut self,
        key: &PackageKey,
        contents: PackageContents,
        from: Option<PackageKey>,
    ) -> Result<Arc<RegistrySnapshot>, TransactionError> {
        self.transition(key, PackageState::Discovered)?;
        self.transition(key, PackageState::Staged)?;
        let manifest = Arc::new(contents.manifest);
        let plan = self.validate(&manifest, contents.definitions, contents.specializations)?;
        self.transition(key, PackageState::Validated)?;
        // Permissions are not interpreted yet; the install request stands for approval.
        self.transition(key, PackageState::Granted)?;
        self.transition(key, PackageState::Activating)?;

        let current = &self.current.snapshot;
        let mut builder = RegistryBuilder::from_snapshot(current);
        for definition in plan.definitions {
            builder = builder.definition(definition);
        }
        let mut specializations = plan.specializations;
        let descriptors: Vec<ProviderDescriptor> = plan
            .providers
            .into_iter()
            .map(|(id, implements)| ProviderDescriptor {
                specializations: specializations.remove(&id).unwrap_or_default(),
                key: ProviderKey {
                    package: key.id.as_str().to_owned(),
                    version: key.version.as_str().to_owned(),
                    content_digest: manifest.content_digest().to_owned(),
                    id,
                },
                implements,
            })
            .collect();
        if let Some(from) = &from
            && let Some(old) = self.current.packages.get(from)
        {
            builder = retarget_defaults(builder, current, old, &descriptors);
        }
        let providers = descriptors
            .iter()
            .map(|provider| provider.key.clone())
            .collect();
        let snapshot = descriptors
            .into_iter()
            .fold(builder, RegistryBuilder::provider)
            .build()?;

        let mut packages = self.current.packages.clone();
        packages.insert(
            key.clone(),
            PackageRecord {
                manifest,
                state: PackageState::Active,
                generation: snapshot.generation(),
                providers,
            },
        );
        let snapshot = self.commit(snapshot, packages, None)?;
        // The commit is durable and names the version active; a lost activation record
        // cannot undo it, and recovery reads the commit first.
        let _ = self.transition(key, PackageState::Active);
        Ok(snapshot)
    }

    /// Contribution validation before any registry work: digest identity, host API,
    /// dependencies, declared definitions, provider slots and their specializations.
    fn validate(
        &self,
        manifest: &PackageManifest,
        definitions: Vec<NodeDefinition>,
        specialized: BTreeMap<(String, DefinitionRef), Vec<TypeArgs>>,
    ) -> Result<Plan, TransactionError> {
        let key = manifest.key();
        if self.revoked.contains(manifest.content_digest()) {
            return Err(TransactionError::Revoked(key.clone()));
        }
        if self
            .published
            .get(key)
            .is_some_and(|digest| digest != manifest.content_digest())
        {
            return Err(TransactionError::DigestConflict(key.clone()));
        }
        if !manifest.host_api().accepts(self.host_api) {
            return Err(TransactionError::HostApi {
                required: manifest.host_api(),
                host: self.host_api,
            });
        }
        for dependency in manifest.dependencies() {
            let dependency = PackageKey {
                id: dependency.id.clone(),
                version: dependency.version.clone(),
            };
            if !self
                .current
                .packages
                .get(&dependency)
                .is_some_and(|record| record.state.is_active())
            {
                return Err(TransactionError::MissingDependency(dependency));
            }
        }

        let declared: BTreeSet<&DefinitionId> = manifest.nodes().iter().collect();
        let shipped: BTreeSet<&DefinitionId> = definitions
            .iter()
            .map(|definition| &definition.definition.id)
            .collect();
        if let Some(definition) = definitions
            .iter()
            .find(|definition| !declared.contains(&definition.definition.id))
        {
            return Err(TransactionError::UndeclaredDefinition(
                definition.definition.clone(),
            ));
        }
        if let Some(missing) = declared.difference(&shipped).next() {
            return Err(TransactionError::MissingDefinition((*missing).clone()));
        }

        let current = &self.current.snapshot;
        let mut config_versions = BTreeMap::new();
        let mut new_definitions = Vec::new();
        for definition in definitions {
            let reference = definition.definition.clone();
            match current.definition(&reference) {
                Some(registered) if *registered == definition => {}
                Some(_) => return Err(TransactionError::DefinitionChanged(reference)),
                None => {
                    config_versions.insert(reference, definition.config_version);
                    new_definitions.push(definition);
                }
            }
        }

        let mut providers: BTreeMap<String, BTreeSet<(DefinitionRef, u32)>> = BTreeMap::new();
        for contribution in manifest.providers() {
            let definition = match &contribution.slot {
                ProviderSlot::Node(definition) => definition,
                ProviderSlot::Other { kind, descriptor } => {
                    return Err(TransactionError::UnsupportedSlot {
                        kind: *kind,
                        descriptor: descriptor.clone(),
                    });
                }
            };
            let config_version = config_versions
                .get(definition)
                .copied()
                .or_else(|| {
                    current
                        .definition(definition)
                        .map(|registered| registered.config_version)
                })
                .ok_or_else(|| TransactionError::UnknownSlotDefinition(definition.clone()))?;
            providers
                .entry(contribution.id.clone())
                .or_default()
                .insert((definition.clone(), config_version));
        }
        let mut specializations: BTreeMap<String, BTreeMap<DefinitionRef, Vec<TypeArgs>>> =
            BTreeMap::new();
        for ((provider, definition), supported) in specialized {
            let declared = providers.get(&provider).is_some_and(|implements| {
                implements
                    .iter()
                    .any(|(implemented, _)| *implemented == definition)
            });
            if !declared {
                return Err(TransactionError::UndeclaredSpecialization {
                    provider,
                    definition,
                });
            }
            specializations
                .entry(provider)
                .or_default()
                .insert(definition, supported);
        }
        Ok(Plan {
            definitions: new_definitions,
            providers,
            specializations,
        })
    }

    /// Takes `keys` out of the next generation and records them in `state` (Removed or
    /// Revoked), adding `revoke` to the revoked digests.
    fn retire(
        &mut self,
        keys: &[PackageKey],
        state: PackageState,
        revoke: Option<&str>,
    ) -> Result<Arc<RegistrySnapshot>, TransactionError> {
        let mut records = Vec::with_capacity(keys.len());
        for key in keys {
            let record = self
                .current
                .packages
                .get(key)
                .ok_or_else(|| TransactionError::NotInstalled(key.clone()))?;
            records.push((key, record.clone()));
        }
        let providers: Vec<ProviderKey> = records
            .iter()
            .filter(|(_, record)| record.state.is_active())
            .flat_map(|(_, record)| record.providers.iter().cloned())
            .collect();
        let snapshot = if records.iter().any(|(_, record)| record.state.is_active()) {
            providers
                .into_iter()
                .fold(
                    RegistryBuilder::from_snapshot(&self.current.snapshot),
                    RegistryBuilder::remove_provider,
                )
                .build()?
        } else {
            Arc::clone(&self.current.snapshot)
        };
        for key in keys {
            self.transition(key, state)?;
        }
        let mut packages = self.current.packages.clone();
        for (key, record) in records {
            packages.insert(
                key.clone(),
                PackageRecord {
                    state,
                    generation: snapshot.generation(),
                    ..record
                },
            );
        }
        self.commit(snapshot, packages, revoke)
    }
}

/// Validated additions for one installation.
struct Plan {
    /// Shipped definitions not yet registered.
    definitions: Vec<NodeDefinition>,
    /// Implemented definitions per package-local provider ID.
    providers: BTreeMap<String, BTreeSet<(DefinitionRef, u32)>>,
    /// Supported argument sets per provider ID and definition, where the package lists them.
    specializations: BTreeMap<String, BTreeMap<DefinitionRef, Vec<TypeArgs>>>,
}

/// Moves new-binding defaults from providers of `old` to the same-ID providers in `new`
/// that implement the defaulted definition.
fn retarget_defaults(
    mut builder: RegistryBuilder,
    current: &RegistrySnapshot,
    old: &PackageRecord,
    new: &[ProviderDescriptor],
) -> RegistryBuilder {
    for contribution in old.manifest.providers() {
        let ProviderSlot::Node(definition) = &contribution.slot else {
            continue;
        };
        let Some(selected) = current.new_binding_default(definition) else {
            continue;
        };
        if !old.providers.contains(selected) {
            continue;
        }
        let successor = new.iter().find(|provider| {
            provider.key.id == selected.id
                && provider
                    .implements
                    .iter()
                    .any(|(implemented, _)| implemented == definition)
        });
        if let Some(successor) = successor {
            builder = builder.new_binding_default(definition.clone(), successor.key.clone());
        }
    }
    builder
}
