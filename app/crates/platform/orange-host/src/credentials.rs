//! Local authority for graph credential references.
use super::*;
use serde_json::{Map, Value as Json, json};
use sha2::{Digest, Sha256};

const APPROVAL_PREFIX: &str = "approval/";
const APPROVED: &str = "approved-v1";

pub(super) struct ResolvedSettings(pub Map<String, Json>);

impl Drop for ResolvedSettings {
    fn drop(&mut self) {
        use zeroize::Zeroize as _;
        fn erase(value: &mut Json) {
            match value {
                Json::String(text) => text.zeroize(),
                Json::Object(object) => object.values_mut().for_each(erase),
                Json::Array(items) => items.iter_mut().for_each(erase),
                _ => {}
            }
        }
        self.0.values_mut().for_each(erase);
    }
}

pub(super) fn open_session(
    store: Option<&Arc<dyn SecretStore>>,
    adapter: &dyn ConnectionAdapter,
    settings: &Map<String, Json>,
) -> Result<Arc<dyn Session>, String> {
    let renewal_field =
        effects::guard(|| adapter.renewal_field()).map_err(|fault| fault.message.to_string())?;
    let (resolved, renew, has_secrets) = with_secrets(store, settings, renewal_field)?;
    effects::guard(|| adapter.open_renewing(&resolved.0, renew))
        .map_err(|fault| fault.message.to_string())?
        .map_err(|error| {
            if has_secrets {
                "the adapter rejected the connection or credentials".into()
            } else {
                error
            }
        })
}

pub(super) fn references(
    adapter: &dyn ConnectionAdapter,
    settings: &Map<String, Json>,
) -> Result<bool, String> {
    fn walk(value: &Json, path: &str, fields: &BTreeSet<String>) -> Result<bool, String> {
        if path.strip_prefix("/headers/").is_some_and(|header| {
            [
                "authorization",
                "proxy-authorization",
                "cookie",
                "x-api-key",
            ]
            .iter()
            .any(|name| header.eq_ignore_ascii_case(name))
        }) {
            return Err("use the adapter's authentication settings for sensitive headers".into());
        }
        if let Some(name) = value
            .as_str()
            .and_then(|text| text.strip_prefix(SECRET_PREFIX))
        {
            return if fields.contains(path)
                && is_secret_name(name)
                && !name.starts_with(APPROVAL_PREFIX)
            {
                Ok(true)
            } else {
                Err("credential references belong in an adapter-declared secret field".into())
            };
        }
        if fields.contains(path) {
            return Err("store the adapter's secret in the credential vault".into());
        }
        let mut found = false;
        if let Json::Object(object) = value {
            for (key, item) in object {
                let key = key.replace('~', "~0").replace('/', "~1");
                found |= walk(item, &format!("{path}/{key}"), fields)?;
            }
        } else if let Json::Array(items) = value {
            for (index, item) in items.iter().enumerate() {
                found |= walk(item, &format!("{path}/{index}"), fields)?;
            }
        }
        Ok(found)
    }
    let fields = effects::guard(|| adapter.texts())
        .map_err(|fault| fault.message.to_string())?
        .into_iter()
        .filter(|field| field.secret)
        .map(|field| format!("/{}", field.setting.trim_matches('/')))
        .collect();
    let mut found = false;
    for (key, item) in settings {
        let key = key.replace('~', "~0").replace('/', "~1");
        found |= walk(item, &format!("/{key}"), &fields)?;
    }
    Ok(found)
}

fn approval_key(graph: GraphId, decl: &ConnectionDecl) -> String {
    use std::fmt::Write as _;
    let mut profile = json!([graph, decl.id, decl.adapter, decl.settings]);
    profile.sort_all_objects();
    let digest = Sha256::digest(profile.to_string().as_bytes());
    let mut key = String::from(APPROVAL_PREFIX);
    for byte in digest {
        let _ = write!(key, "{byte:02x}");
    }
    key
}

impl Host {
    /// Authorizes the current graph-scoped profile to use its credential references.
    /// Call after the operator or embedding application approves the exact settings.
    /// The approval is local to the vault and does not enter the graph or an export.
    pub fn approve_connection_credentials(&self, draft: &Draft, id: &str) -> Result<(), HostError> {
        self.approve_credentials(draft, id, true)
    }

    /// Authorizes this profile only for the current host, including hosts with
    /// read-only credential stores.
    pub fn approve_connection_credentials_for_session(
        &self,
        draft: &Draft,
        id: &str,
    ) -> Result<(), HostError> {
        self.approve_credentials(draft, id, false)
    }

    fn approve_credentials(&self, draft: &Draft, id: &str, persist: bool) -> Result<(), HostError> {
        let id: ConnectionId = id
            .parse()
            .map_err(|_| HostError::Document("invalid connection ID".into()))?;
        let document = draft.document();
        let decl = document
            .connections()
            .get(&id)
            .ok_or_else(|| HostError::Document("the graph has no such connection".into()))?;
        if decl.scope != ConnectionScope::Graph {
            return Ok(());
        }
        let refused = |reason: String| HostError::Connection(decl.name.clone(), reason);
        let adapter = self
            .adapters
            .get(&decl.adapter)
            .ok_or_else(|| refused("the connection adapter is unavailable".into()))?;
        if !references(adapter.as_ref(), &decl.settings).map_err(refused)? {
            return Ok(());
        }
        let store = self
            .options
            .secrets
            .as_ref()
            .ok_or_else(|| refused("the application has no credential vault".into()))?;
        let key = approval_key(document.graph_id(), decl);
        if persist {
            store
                .replace(&key, APPROVED)
                .map_err(|error| refused(error.to_string()))?;
        }
        let _ = self
            .credential_approvals
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key);
        Ok(())
    }

    pub(super) fn check_connection_credentials(
        &self,
        graph: GraphId,
        decl: &ConnectionDecl,
        adapter: &dyn ConnectionAdapter,
    ) -> Result<(), String> {
        if !references(adapter, &decl.settings)? {
            return Ok(());
        }
        let store = self
            .options
            .secrets
            .as_ref()
            .ok_or("the application has no credential vault")?;
        let key = approval_key(graph, decl);
        if self
            .credential_approvals
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&key)
        {
            return Ok(());
        }
        match store.get(&key) {
            Ok(value) if value == APPROVED => Ok(()),
            Ok(_) | Err(SecretError::NotFound) => Err(
                "approve this connection's credential use for these settings before running the graph".into()
            ),
            Err(error) => Err(error.to_string()),
        }
    }
}

/// The settings an adapter receives: every string that is exactly
/// `credential://<name>`, at any depth, replaced by that secret. The result is handed
/// to the adapter and never stored, logged or compared.
fn with_secrets(
    secrets: Option<&Arc<dyn SecretStore>>,
    settings: &serde_json::Map<String, serde_json::Value>,
    renewal_field: Option<&'static str>,
) -> Result<(ResolvedSettings, Renew, bool), String> {
    use serde_json::Value as Json;
    // Each setting that named a credential, as a JSON pointer, with the name.
    fn resolve(
        value: &mut Json,
        at: &mut String,
        store: Option<&Arc<dyn SecretStore>>,
        named: &mut Vec<(String, String)>,
    ) -> Result<(), String> {
        let mut inside = |part: &str, value: &mut Json, at: &mut String| {
            let length = at.len();
            at.push('/');
            at.push_str(&part.replace('~', "~0").replace('/', "~1"));
            let done = resolve(value, at, store, named);
            at.truncate(length);
            done
        };
        match value {
            Json::String(text) => {
                if let Some(name) = text.strip_prefix(SECRET_PREFIX) {
                    let store = store.ok_or_else(|| {
                        format!(
                            "it names credential `{name}` and the application has no credential store"
                        )
                    })?;
                    let secret = store
                        .get(name)
                        .map_err(|error| format!("credential `{name}`: {error}"))?;
                    named.push((at.clone(), name.to_owned()));
                    *text = secret;
                }
                Ok(())
            }
            Json::Array(items) => items
                .iter_mut()
                .enumerate()
                .try_for_each(|(index, item)| inside(&index.to_string(), item, at)),
            Json::Object(fields) => fields
                .iter_mut()
                .try_for_each(|(name, field)| inside(name, field, at)),
            _ => Ok(()),
        }
    }
    let mut resolved = ResolvedSettings(settings.clone());
    let mut named = Vec::new();
    for (key, value) in &mut resolved.0 {
        let mut path = format!("/{}", key.replace('~', "~0").replace('/', "~1"));
        resolve(value, &mut path, secrets, &mut named)?;
    }
    let has_secrets = !named.is_empty();
    let renewable = named
        .into_iter()
        .find(|(path, _)| Some(path.as_str()) == renewal_field);
    let renew = match (secrets, renewable) {
        (Some(store), Some((path, name))) => {
            let store = Arc::clone(store);
            Renew::new(move |setting, current, secret| {
                if setting != path {
                    return Err(SecretError::InvalidName);
                }
                // Multiple sessions may finish a refresh at the same time.
                static ROTATION: Mutex<()> = Mutex::new(());
                let _guard = ROTATION
                    .lock()
                    .map_err(|_| SecretError::Unavailable("rotation failed"))?;
                let expected = zeroize::Zeroizing::new(store.get(&name)?);
                if expected.as_str() != current {
                    return Err(SecretError::Unavailable("the credential has changed"));
                }
                store.replace(&name, secret)
            })
        }
        _ => Renew::none(),
    };
    Ok((resolved, renew, has_secrets))
}

#[cfg(test)]
mod tests {
    use super::*;
    use orange_sdk::SettingText;

    #[derive(Debug)]
    struct PanickingMetadata;

    impl ConnectionAdapter for PanickingMetadata {
        fn open(&self, _: &Map<String, Json>) -> Result<Arc<dyn Session>, String> {
            panic!("unexpected open");
        }
        fn texts(&self) -> Vec<SettingText> {
            panic!("synthetic metadata failure");
        }
        fn renewal_field(&self) -> Option<&'static str> {
            panic!("synthetic renewal failure");
        }
    }

    #[test]
    fn adapter_metadata_panics_are_returned_as_generic_errors() {
        let settings = Map::new();
        let failure = references(&PanickingMetadata, &settings).unwrap_err();
        assert_eq!(failure, "A service provider stopped unexpectedly.");
        let Err(failure) = open_session(None, &PanickingMetadata, &settings) else {
            panic!("expected the metadata fault");
        };
        assert_eq!(failure, "A service provider stopped unexpectedly.");
    }
}
