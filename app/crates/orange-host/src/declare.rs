//! What a document declares besides its nodes: its variables and its connections, and
//! the types chosen for generic nodes. Like the operations in `edit`, each one is one
//! model transaction on a [`Draft`].
use super::*;
use orange_model::DocCommand;
use orange_model::document::{ConnectionDecl, LocalTypeDecl, VariableDecl};
use orange_model::id::{ConnectionId, TypeDeclId, TypeExpr, VariableId};
use orange_sdk::{SECRET_PREFIX, SettingText, SettingToggle};
use serde_json::{Value as Json, json};

use draft::refused;

/// One variable of a document. Its `id` is `None` for one not declared yet.
#[derive(Debug, Clone, PartialEq)]
pub struct Variable {
    pub id: Option<String>,
    pub name: String,
    /// Its type, such as `core/i64@1`.
    pub ty: String,
    pub scope: VarScope,
    /// Whether its value is kept between runs.
    pub persistent: bool,
    /// Its value before anything writes it, as a person types it (`7`, `some text`);
    /// `None` leaves it unset until written.
    pub initial: Option<String>,
}

/// One connection of a document. Its `id` is `None` for one not declared yet.
#[derive(Debug, Clone, PartialEq)]
pub struct Connection {
    pub id: Option<String>,
    pub name: String,
    /// The adapter, such as `org.orange.net/http`.
    pub adapter: String,
    pub scope: ConnectionScope,
    /// The adapter's settings, a JSON object. Secrets are never written here: a setting
    /// that holds one is a `credential://<name>` reference to the host's store.
    pub settings: Json,
}

/// What a document declares, in its own order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Declarations {
    pub variables: Vec<Variable>,
    pub connections: Vec<Connection>,
}

/// A connection adapter as an editor offers it: by name and purpose, not by kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterInfo {
    /// Its kind, such as `org.orange.net/http`, which a connection names.
    pub id: String,
    /// What it is called, such as "Web service (HTTP)"; made from its kind when the
    /// adapter gives no name.
    pub label: String,
    /// One sentence on what it is for; empty when the adapter gives none.
    pub purpose: String,
}

/// The name a connection's secret is stored under in the credential store:
/// `connection/<connection ID>/<setting>`, such as `connection/…/basic/password`.
pub fn secret_name(connection: &str, setting: &str) -> String {
    let clean = |text: &str| -> String {
        text.trim_matches('/')
            .chars()
            .map(|c| {
                let c = c.to_ascii_lowercase();
                if c.is_ascii_lowercase()
                    || c.is_ascii_digit()
                    || matches!(c, '.' | '_' | '-' | '/')
                {
                    c
                } else {
                    '-'
                }
            })
            .collect()
    };
    format!("connection/{}/{}", clean(connection), clean(setting))
}

/// `settings` with the setting at `path` (names joined by `/`) set to `value`,
/// making the objects on the way.
fn put_at(settings: &mut Json, path: &str, value: Json) {
    let mut at = settings;
    let mut names = path.split('/').filter(|name| !name.is_empty()).peekable();
    while let Some(name) = names.next() {
        if !at.is_object() {
            *at = json!({});
        }
        let Json::Object(fields) = at else {
            return;
        };
        if names.peek().is_none() {
            let _ = fields.insert(name.to_owned(), value);
            return;
        }
        at = fields.entry(name.to_owned()).or_insert_with(|| json!({}));
    }
}

/// Words that mark a setting as a secret, which a document holds only as a reference.
const SECRET_WORDS: [&str; 6] = ["password", "secret", "token", "bearer", "api_key", "apikey"];

/// The first setting, at any depth, whose name says it is a secret and whose value is
/// not a `credential://` reference.
fn plain_secret(settings: &Json, path: &str) -> Option<String> {
    match settings {
        Json::Object(fields) => fields.iter().find_map(|(name, value)| {
            let here = format!("{path}/{name}");
            let lower = name.to_ascii_lowercase();
            let secret = SECRET_WORDS.iter().any(|word| lower.contains(word));
            match value {
                Json::String(text) if secret && !text.starts_with("credential://") => Some(here),
                _ => plain_secret(value, &here),
            }
        }),
        Json::Array(items) => items
            .iter()
            .enumerate()
            .find_map(|(index, item)| plain_secret(item, &format!("{path}/{index}"))),
        _ => None,
    }
}

impl Host {
    /// The variables and connections a document declares.
    pub fn declarations(&self, draft: &Draft) -> Declarations {
        let document = draft.document();
        let variables = document
            .variables()
            .values()
            .map(|variable| Variable {
                id: Some(variable.id.to_string()),
                name: variable.name.clone(),
                ty: variable.ty.to_string(),
                scope: variable.scope,
                persistent: variable.persistent,
                initial: variable
                    .init
                    .as_ref()
                    .and_then(|literal| self.value_of(literal).ok())
                    .map(|value| plain(&value)),
            })
            .collect();
        let connections = document
            .connections()
            .values()
            .map(|connection| Connection {
                id: Some(connection.id.to_string()),
                name: connection.name.clone(),
                adapter: connection.adapter.clone(),
                scope: connection.scope,
                settings: Json::Object(connection.settings.clone()),
            })
            .collect();
        Declarations {
            variables,
            connections,
        }
    }

    /// The connection adapters this host has, such as `org.orange.net/http`, sorted.
    pub fn adapters(&self) -> Vec<String> {
        self.adapters.keys().cloned().collect()
    }

    /// Every connection adapter this host has, from every package, with its name and
    /// purpose, sorted by name.
    pub fn adapter_list(&self) -> Vec<AdapterInfo> {
        let mut list: Vec<AdapterInfo> = self
            .adapters
            .iter()
            .map(|(id, adapter)| {
                let about = adapter.about();
                let label = if about.label.trim().is_empty() {
                    words(id.rsplit('/').next().unwrap_or(id))
                } else {
                    about.label
                };
                AdapterInfo {
                    id: id.clone(),
                    label,
                    purpose: about.purpose,
                }
            })
            .collect();
        list.sort_by_key(|adapter| adapter.label.to_lowercase());
        list
    }

    /// The text settings of `adapter` an editor can show as fields, secrets among
    /// them. Empty for an adapter that has none or that this host lacks.
    pub fn adapter_texts(&self, adapter: &str) -> Vec<SettingText> {
        self.adapters
            .get(adapter)
            .map(|found| found.texts())
            .unwrap_or_default()
    }

    /// The true-or-false settings of `adapter` an editor can show as switches.
    pub fn adapter_toggles(&self, adapter: &str) -> Vec<SettingToggle> {
        self.adapters
            .get(adapter)
            .map(|found| found.toggles())
            .unwrap_or_default()
    }

    /// The nodes of a document that use connection `id`, in document order.
    pub fn connection_users(&self, draft: &Draft, id: &str) -> Vec<NodeId> {
        draft
            .document()
            .bodies()
            .flat_map(|body| body.nodes.values())
            .filter(|node| {
                node.config
                    .get("connection")
                    .and_then(Json::as_str)
                    .is_some_and(|named| named == id)
            })
            .map(|node| node.id)
            .collect()
    }

    /// Opens a connection of `adapter` with `settings` and lets it go, to check them:
    /// the settings are valid and every credential they name is in the store. Nothing
    /// is sent by an adapter that connects on first use. The error says why it would
    /// not open, never with a secret's value.
    pub fn test_connection(
        &self,
        adapter: &str,
        settings: &serde_json::Map<String, Json>,
    ) -> Result<(), String> {
        let found = self
            .adapters
            .get(adapter)
            .ok_or_else(|| format!("no adapter `{adapter}` is available"))?;
        let (settings, _) = with_secrets(self.options.secrets.as_ref(), settings)?;
        found
            .open(&settings)
            .map(drop)
            .map_err(|_| "The adapter could not open with these settings. Check the connection and credentials.".into())
    }

    /// The settings of `adapter` an editor can offer as a list, with the values this
    /// machine has now (such as its audio devices). Empty for an adapter that offers
    /// none or that this host does not have. It may ask the operating system, so a front
    /// end calls it when a person opens the list, not every frame.
    pub fn adapter_choices(&self, adapter: &str) -> Vec<SettingChoices> {
        self.adapters
            .get(adapter)
            .map(|found| found.choices())
            .unwrap_or_default()
    }

    /// The number settings of `adapter` an editor can show as fields, such as an audio
    /// input's threshold. Empty for an adapter that has none or that this host lacks.
    pub fn adapter_fields(&self, adapter: &str) -> Vec<SettingField> {
        self.adapters
            .get(adapter)
            .map(|found| found.fields())
            .unwrap_or_default()
    }

    /// Whether [`Host::probe`] has a live reading to show for `adapter`. Opens nothing.
    pub fn adapter_probes(&self, adapter: &str) -> bool {
        self.adapters
            .get(adapter)
            .is_some_and(|found| found.probes())
    }

    /// A live reading of what a connection of `adapter` with `settings` would see, such
    /// as an input device's level, for an editor to show while a person sets it up.
    /// It runs apart from every flow and stops when dropped. `None` when the adapter
    /// has nothing to show; an error says why it cannot start.
    pub fn probe(
        &self,
        adapter: &str,
        settings: &serde_json::Map<String, serde_json::Value>,
    ) -> Option<Result<Box<dyn Probe>, String>> {
        self.adapters.get(adapter)?.probe(settings)
    }

    /// Declares a variable, or changes the one with `variable.id`, keeping what this
    /// does not set. Returns the variable's ID.
    pub fn declare_variable(
        &self,
        draft: &mut Draft,
        variable: &Variable,
    ) -> Result<String, HostError> {
        let id = match &variable.id {
            Some(id) => id.clone(),
            None => draft.fresh_id("variable"),
        };
        let parsed: VariableId = id
            .parse()
            .map_err(|_| refused(&format!("`{id}` is not a variable ID")))?;
        let init = match &variable.initial {
            None => None,
            Some(typed) => {
                let text = literal(&variable.ty, typed).map_err(HostError::Variable)?;
                let parsed: Literal = serde_json::from_str(&text)
                    .map_err(|error| HostError::Variable(error.to_string()))?;
                let _ = self.value_of(&parsed)?;
                Some(parsed)
            }
        };
        // Start from what is declared, so fields this does not set are kept.
        let mut decl: Json = draft
            .document()
            .variables()
            .get(&parsed)
            .map(serde_json::to_value)
            .transpose()
            .map_err(|error| refused(&error))?
            .unwrap_or_else(|| json!({}));
        decl["id"] = json!(id);
        decl["name"] = json!(variable.name);
        decl["type"] = json!(variable.ty);
        decl["scope"] = serde_json::to_value(variable.scope).map_err(|error| refused(&error))?;
        decl["persistent"] = json!(variable.persistent);
        match init {
            Some(init) => {
                decl["init"] = serde_json::to_value(init).map_err(|error| refused(&error))?;
            }
            None => {
                if let Some(fields) = decl.as_object_mut() {
                    let _ = fields.remove("init");
                }
            }
        }
        let decl: VariableDecl = serde_json::from_value(decl).map_err(|error| refused(&error))?;
        let _ = draft.commit(
            &self.registry_for(draft.document()),
            [DocCommand::UpsertVariable(decl)],
        )?;
        Ok(id)
    }

    /// Declares a record or enum type in the document, or replaces the one with the same
    /// ID: the way a person makes their own choices and structures without a package
    /// (12 §6.5). A new type gets its ID from [`Host::new_type_id`]. Renaming the type or
    /// a member keeps every saved value; removing a variant or field, or changing its
    /// type, shows as problems where values use it. Returns whether anything changed.
    pub fn declare_type(&self, draft: &mut Draft, decl: LocalTypeDecl) -> Result<bool, HostError> {
        draft.commit(
            &self.registry_for(draft.document()),
            [DocCommand::UpsertType(decl)],
        )
    }

    /// An ID for a new type of `draft`, unused by anything in it.
    pub fn new_type_id(&self, draft: &Draft) -> Result<TypeDeclId, HostError> {
        draft
            .fresh_id("type")
            .parse()
            .map_err(|_| refused(&"the new ID is not valid"))
    }

    /// Removes a type the document declares. Variables, ports and values that use it
    /// stay and show as problems until they use another type or are removed.
    pub fn remove_type(&self, draft: &mut Draft, id: TypeDeclId) -> Result<bool, HostError> {
        draft.commit(
            &self.registry_for(draft.document()),
            [DocCommand::RemoveType(id)],
        )
    }

    /// Removes a variable. Nodes that name it stay and show as problems until they
    /// name another one or are removed.
    pub fn remove_variable(&self, draft: &mut Draft, id: &str) -> Result<bool, HostError> {
        let id: VariableId = id
            .parse()
            .map_err(|_| refused(&format!("`{id}` is not a variable ID")))?;
        draft.commit(
            &self.registry_for(draft.document()),
            [DocCommand::RemoveVariable(id)],
        )
    }

    /// Declares a connection, or changes the one with `connection.id`. A setting whose
    /// name says it is a secret (a password, token, bearer or key) must be a
    /// `credential://<name>` reference: a document never holds the secret itself.
    /// Returns the connection's ID.
    pub fn declare_connection(
        &self,
        draft: &mut Draft,
        connection: &Connection,
    ) -> Result<String, HostError> {
        let (id, decl) = self.connection_decl(draft, connection)?;
        let _ = draft.commit(
            &self.registry_for(draft.document()),
            [DocCommand::UpsertConnection(decl)],
        )?;
        Ok(id)
    }

    /// The declaration [`declare_connection`](Self::declare_connection) writes, with
    /// its ID, checked but not applied.
    pub(crate) fn connection_decl(
        &self,
        draft: &Draft,
        connection: &Connection,
    ) -> Result<(String, ConnectionDecl), HostError> {
        if !connection.settings.is_object() {
            return Err(refused(&"a connection's settings are a JSON object"));
        }
        if let Some(at) = plain_secret(&connection.settings, "") {
            return Err(refused(&format!(
                "the setting `{at}` holds a secret: write it as credential://<name> and keep the secret in the store"
            )));
        }
        let id = match &connection.id {
            Some(id) => id.clone(),
            None => draft.fresh_id("connection"),
        };
        let parsed: ConnectionId = id
            .parse()
            .map_err(|_| refused(&format!("`{id}` is not a connection ID")))?;
        let mut decl: Json = draft
            .document()
            .connections()
            .get(&parsed)
            .map(serde_json::to_value)
            .transpose()
            .map_err(|error| refused(&error))?
            .unwrap_or_else(|| json!({}));
        decl["id"] = json!(id);
        decl["name"] = json!(connection.name);
        decl["adapter"] = json!(connection.adapter);
        decl["scope"] = serde_json::to_value(connection.scope).map_err(|error| refused(&error))?;
        decl["settings"] = connection.settings.clone();
        let decl: ConnectionDecl = serde_json::from_value(decl).map_err(|error| refused(&error))?;
        Ok((id, decl))
    }

    /// Declares a connection like [`declare_connection`](Self::declare_connection),
    /// first storing each of `secrets` (a setting, as names joined by `/`, and the
    /// secret typed for it) in the host's credential store under
    /// [`secret_name`] and writing only its `credential://` reference into the
    /// settings. Nothing is declared when a secret cannot be stored. Returns the
    /// connection's ID.
    pub fn declare_connection_with_secrets(
        &self,
        draft: &mut Draft,
        connection: &Connection,
        secrets: &[(String, String)],
    ) -> Result<String, HostError> {
        let id = match &connection.id {
            Some(id) => id.clone(),
            None => draft.fresh_id("connection"),
        };
        let mut connection = Connection {
            id: Some(id.clone()),
            ..connection.clone()
        };
        if !secrets.is_empty() {
            let store = self.options.secrets.as_ref().ok_or_else(|| {
                HostError::Connection(
                    connection.name.clone(),
                    "the application has no credential store".into(),
                )
            })?;
            for (setting, secret) in secrets {
                let name = secret_name(&id, setting);
                store.replace(&name, secret).map_err(|error| {
                    HostError::Connection(connection.name.clone(), format!("{setting}: {error}"))
                })?;
                put_at(
                    &mut connection.settings,
                    setting,
                    Json::String(format!("{SECRET_PREFIX}{name}")),
                );
            }
        }
        self.declare_connection(draft, &connection)
    }

    /// Removes a connection. Nodes that use it show as problems until they use another.
    pub fn remove_connection(&self, draft: &mut Draft, id: &str) -> Result<bool, HostError> {
        let id: ConnectionId = id
            .parse()
            .map_err(|_| refused(&format!("`{id}` is not a connection ID")))?;
        draft.commit(
            &self.registry_for(draft.document()),
            [DocCommand::RemoveConnection(id)],
        )
    }

    /// Chooses the type of one type parameter of a generic node (a pin, 11 §5.5), such
    /// as `core/i64@1` for `T`; `None` takes the choice off so wires decide again.
    pub fn choose_type(
        &self,
        draft: &mut Draft,
        node: NodeId,
        param: &str,
        ty: Option<&str>,
    ) -> Result<bool, HostError> {
        let mut pins = draft
            .document()
            .node(node)
            .map(|found| found.type_pins.clone())
            .ok_or_else(|| refused(&"the document has no such node"))?;
        match ty {
            Some(ty) => {
                let ty: TypeExpr =
                    serde_json::from_value(json!(ty)).map_err(|error| refused(&error))?;
                let _ = pins.insert(param.to_owned(), ty);
            }
            None => {
                let _ = pins.remove(param);
            }
        }
        draft.commit(
            &self.registry_for(draft.document()),
            [DocCommand::SetTypePins { node, pins }],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_secrets_are_found_at_any_depth() {
        let settings = json!({"url": "https://example.test",
            "auth": {"bearer": "credential://api/token"}});
        assert_eq!(plain_secret(&settings, ""), None);
        let settings = json!({"oauth": {"client_id": "x", "refresh_token": "abc"}});
        assert_eq!(
            plain_secret(&settings, "").as_deref(),
            Some("/oauth/refresh_token")
        );
        let settings = json!({"users": [{"Password": "hunter"}]});
        assert_eq!(
            plain_secret(&settings, "").as_deref(),
            Some("/users/0/Password")
        );
    }
}
