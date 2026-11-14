//! What a document declares besides its nodes: its variables and its connections, and
//! the types chosen for generic nodes. Like the operations in `edit`, each one takes
//! the document's text and returns the edited text through one model transaction.
use super::*;
use orange_model::DocCommand;
use orange_model::document::{ConnectionDecl, VariableDecl};
use orange_model::id::{ConnectionId, TypeExpr, VariableId};
use serde_json::{Value as Json, json};

fn refused(error: &dyn std::fmt::Display) -> HostError {
    HostError::Document(error.to_string())
}

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
    pub fn declarations(&self, json: &str) -> Result<Declarations, HostError> {
        let document = from_json(json).map_err(|error| refused(&error))?;
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
        Ok(Declarations {
            variables,
            connections,
        })
    }

    /// The connection adapters this host has, such as `org.orange.net/http`, sorted.
    pub fn adapters(&self) -> Vec<String> {
        self.adapters.keys().cloned().collect()
    }

    /// Declares a variable, or changes the one with `variable.id`, keeping what this
    /// does not set. Returns the document and the variable's ID.
    pub fn declare_variable(
        &self,
        json: &str,
        variable: &Variable,
    ) -> Result<(String, String), HostError> {
        let mut document = from_json(json).map_err(|error| refused(&error))?;
        let saved: Json = serde_json::from_str(json).map_err(|error| refused(&error))?;
        let id = match &variable.id {
            Some(id) => id.clone(),
            None => edit::fresh_id(&saved, "variable"),
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
        let mut decl: Json = document
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
        match document.apply([DocCommand::UpsertVariable(decl)]) {
            Ok(_) => Ok((to_json(&document).map_err(|error| refused(&error))?, id)),
            Err(orange_model::TxError::Empty) => Ok((json.to_owned(), id)),
            Err(error) => Err(refused(&error)),
        }
    }

    /// Removes a variable. Nodes that name it stay and show as problems until they
    /// name another one or are removed.
    pub fn remove_variable(&self, json: &str, id: &str) -> Result<String, HostError> {
        let mut document = from_json(json).map_err(|error| refused(&error))?;
        let id: VariableId = id
            .parse()
            .map_err(|_| refused(&format!("`{id}` is not a variable ID")))?;
        let _ = document
            .apply([DocCommand::RemoveVariable(id)])
            .map_err(|error| refused(&error))?;
        to_json(&document).map_err(|error| refused(&error))
    }

    /// Declares a connection, or changes the one with `connection.id`. A setting whose
    /// name says it is a secret (a password, token, bearer or key) must be a
    /// `credential://<name>` reference: a document never holds the secret itself.
    /// Returns the document and the connection's ID.
    pub fn declare_connection(
        &self,
        json: &str,
        connection: &Connection,
    ) -> Result<(String, String), HostError> {
        if !connection.settings.is_object() {
            return Err(refused(&"a connection's settings are a JSON object"));
        }
        if let Some(at) = plain_secret(&connection.settings, "") {
            return Err(refused(&format!(
                "the setting `{at}` holds a secret: write it as credential://<name> and keep the secret in the store"
            )));
        }
        let mut document = from_json(json).map_err(|error| refused(&error))?;
        let saved: Json = serde_json::from_str(json).map_err(|error| refused(&error))?;
        let id = match &connection.id {
            Some(id) => id.clone(),
            None => edit::fresh_id(&saved, "connection"),
        };
        let parsed: ConnectionId = id
            .parse()
            .map_err(|_| refused(&format!("`{id}` is not a connection ID")))?;
        let mut decl: Json = document
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
        match document.apply([DocCommand::UpsertConnection(decl)]) {
            Ok(_) => Ok((to_json(&document).map_err(|error| refused(&error))?, id)),
            Err(orange_model::TxError::Empty) => Ok((json.to_owned(), id)),
            Err(error) => Err(refused(&error)),
        }
    }

    /// Removes a connection. Nodes that use it show as problems until they use another.
    pub fn remove_connection(&self, json: &str, id: &str) -> Result<String, HostError> {
        let mut document = from_json(json).map_err(|error| refused(&error))?;
        let id: ConnectionId = id
            .parse()
            .map_err(|_| refused(&format!("`{id}` is not a connection ID")))?;
        let _ = document
            .apply([DocCommand::RemoveConnection(id)])
            .map_err(|error| refused(&error))?;
        to_json(&document).map_err(|error| refused(&error))
    }

    /// Chooses the type of one type parameter of a generic node (a pin, 11 §5.5), such
    /// as `core/i64@1` for `T`; `None` takes the choice off so wires decide again.
    pub fn choose_type(
        &self,
        json: &str,
        node: NodeId,
        param: &str,
        ty: Option<&str>,
    ) -> Result<String, HostError> {
        let mut document = from_json(json).map_err(|error| refused(&error))?;
        let mut pins = document
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
        match document.apply([DocCommand::SetTypePins { node, pins }]) {
            Ok(_) => to_json(&document).map_err(|error| refused(&error)),
            Err(orange_model::TxError::Empty) => Ok(json.to_owned()),
            Err(error) => Err(refused(&error)),
        }
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
