//! Protocol-specific controls for the bundled network adapters.
use super::{Typed, set_text, text_at, text_fields};
use crate::controls::{Dropdown as _, Surface};
use crate::i18n::t;
use orange_host::{Host, SettingText};
use serde_json::Value as Json;

pub(crate) const HTTP: &str = "org.orange.net/http";
const WS: &str = "org.orange.net/websocket";
const MQTT: &str = "org.orange.net/mqtt";
const AMQP: &str = "org.orange.net/amqp";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Auth {
    None,
    Guest,
    Basic,
    Bearer,
    ApiKey,
    OAuthClient,
    OAuthRefresh,
    Password,
}

fn modes(adapter: &str) -> &'static [Auth] {
    use Auth::*;
    match adapter {
        HTTP => &[None, Basic, Bearer, ApiKey, OAuthClient, OAuthRefresh],
        WS => &[None, Basic, Bearer, ApiKey],
        MQTT => &[None, Password],
        AMQP => &[Guest, Password],
        _ => &[],
    }
}

pub(super) fn uses_auth(adapter: &str) -> bool {
    !modes(adapter).is_empty()
}

impl Auth {
    fn label(self) -> &'static str {
        t(match self {
            Self::None => "auth-none",
            Self::Guest => "auth-guest",
            Self::Basic => "auth-basic",
            Self::Bearer => "auth-bearer",
            Self::ApiKey => "auth-api-key",
            Self::OAuthClient => "auth-oauth-client",
            Self::OAuthRefresh => "auth-oauth-refresh",
            Self::Password => "auth-password",
        })
    }

    fn required(self) -> &'static [&'static str] {
        match self {
            Self::None | Self::Guest => &[],
            Self::Basic => &["basic/user", "basic/password"],
            Self::Bearer => &["bearer"],
            Self::ApiKey => &["api_key/token"],
            Self::OAuthClient => &["oauth/token_url", "oauth/client_id", "oauth/client_secret"],
            Self::OAuthRefresh => &["oauth/token_url", "oauth/client_id", "oauth/refresh_token"],
            Self::Password => &["username"],
        }
    }

    fn shows(self, field: &str) -> bool {
        self.required().contains(&field)
            || matches!(
                (self, field),
                (Self::Password, "password")
                    | (Self::ApiKey, "api_key/header")
                    | (Self::OAuthClient | Self::OAuthRefresh, "oauth/scope")
                    | (Self::OAuthRefresh, "oauth/client_secret")
            )
    }
}

fn infer(adapter: &str, settings: &Json) -> Option<Auth> {
    let default = *modes(adapter).first()?;
    Some(if matches!(adapter, MQTT | AMQP) {
        if settings.get("username").is_some() || settings.get("password").is_some() {
            Auth::Password
        } else {
            default
        }
    } else if settings.get("oauth").is_some() && adapter == HTTP {
        if text_at(settings, "oauth/refresh_token").is_some() {
            Auth::OAuthRefresh
        } else {
            Auth::OAuthClient
        }
    } else if settings.get("basic").is_some() {
        Auth::Basic
    } else if settings.get("bearer").is_some() {
        Auth::Bearer
    } else if settings.get("api_key").is_some() {
        Auth::ApiKey
    } else {
        default
    })
}

fn auth_field(path: &str) -> bool {
    matches!(path, "bearer" | "username" | "password")
        || path.starts_with("basic/")
        || path.starts_with("api_key/")
        || path.starts_with("oauth/")
}

fn choose(auth: Auth, settings: &mut Json, secrets: &mut Typed) {
    for root in [
        "bearer", "basic", "api_key", "oauth", "username", "password",
    ] {
        let keep = match root {
            "basic" => auth == Auth::Basic,
            "api_key" => auth == Auth::ApiKey,
            "oauth" => matches!(auth, Auth::OAuthClient | Auth::OAuthRefresh),
            setting => auth.shows(setting),
        };
        if !keep {
            set_text(settings, root, None);
        }
    }
    if auth == Auth::OAuthClient {
        set_text(settings, "oauth/refresh_token", None);
    }
    secrets.values.retain(|path, _| auth.shows(path));
    secrets.revealed.clear();
}

fn localized(adapter: &str, field: &SettingText) -> SettingText {
    let label = match field.setting.as_str() {
        "base_url" => "connection-base-url",
        "url" => match adapter {
            WS => "connection-websocket-url",
            MQTT | AMQP => "connection-broker-url",
            _ => "connection-address",
        },
        "basic/user" | "username" => "connection-username",
        "password" if adapter == MQTT => "connection-password-optional",
        "basic/password" | "password" => "connection-password",
        "bearer" => "connection-token",
        "api_key/header" => "connection-key-header",
        "api_key/token" => "connection-key-value",
        "oauth/token_url" => "connection-oauth-url",
        "oauth/client_id" => "connection-oauth-id",
        "oauth/client_secret" => "connection-oauth-secret",
        "oauth/refresh_token" => "connection-oauth-refresh",
        "oauth/scope" => "connection-oauth-scope",
        _ => return field.clone(),
    };
    SettingText {
        label: t(label).to_owned(),
        ..field.clone()
    }
}

pub(super) fn advanced_texts(texts: &[SettingText]) -> Vec<SettingText> {
    texts
        .iter()
        .filter(|field| !auth_field(&field.setting) && !endpoint(field))
        .cloned()
        .collect()
}

fn endpoint(field: &SettingText) -> bool {
    matches!(field.setting.as_str(), "url" | "base_url")
}

/// Reports changes that are separate from the saved settings, including a new
/// mode with no fields filled and changes to masked drafts.
pub(super) fn fields(
    ui: &mut egui::Ui,
    adapter: &str,
    texts: &[SettingText],
    settings: &mut Json,
    secrets: &mut Typed,
    selected: &mut Option<Auth>,
    host: &Host,
) -> bool {
    let Some(mut auth) = selected.or_else(|| infer(adapter, settings)) else {
        return text_fields(ui, texts, settings, secrets, host);
    };
    let before = auth;
    let endpoints: Vec<_> = texts
        .iter()
        .filter(|field| endpoint(field))
        .map(|field| localized(adapter, field))
        .collect();
    text_fields(ui, &endpoints, settings, secrets, host);
    let caption = ui.label(t("authentication"));
    let _ = egui::ComboBox::from_id_salt("connection-authentication")
        .selected_text(auth.label())
        .width(ui.available_width())
        .show_select(ui, Surface::Dialog, |ui| {
            for mode in modes(adapter) {
                let _ = ui.selectable_value(&mut auth, *mode, mode.label());
            }
        })
        .response
        .labelled_by(caption.id);
    if auth != before {
        choose(auth, settings, secrets);
        *selected = Some(auth);
    }
    if auth == Auth::Guest {
        ui.weak(t("connection-amqp-guest-hint"));
    }
    let credentials: Vec<_> = texts
        .iter()
        .filter(|field| auth.shows(&field.setting))
        .map(|field| SettingText {
            required: auth.required().contains(&field.setting.as_str())
                || (adapter == AMQP && field.setting == "password"),
            ..localized(adapter, field)
        })
        .collect();
    let secret_changed = text_fields(ui, &credentials, settings, secrets, host);
    auth != before || secret_changed
}

pub(super) fn validate(
    adapter: &str,
    settings: &Json,
    secrets: &Typed,
    selected: Option<Auth>,
) -> Result<(), &'static str> {
    let schemes = match adapter {
        HTTP => ["http", "https"],
        WS => ["ws", "wss"],
        MQTT => ["mqtt", "mqtts"],
        AMQP => ["amqp", "amqps"],
        _ => return Ok(()),
    };
    let key = if adapter == HTTP { "base_url" } else { "url" };
    let address = text_at(settings, key)
        .and_then(Json::as_str)
        .ok_or("connection-url-required")?;
    let (scheme, rest) = address.split_once("://").ok_or("connection-url-invalid")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if !schemes.contains(&scheme)
        || authority.is_empty()
        || authority.contains('@')
        || address.contains('#')
        || address.chars().any(char::is_whitespace)
        || (adapter == HTTP && address.contains('?'))
    {
        return Err("connection-url-invalid");
    }

    let has = |key: &str| {
        secrets
            .values
            .get(key)
            .is_some_and(|value| !value.is_empty())
            || text_at(settings, key)
                .and_then(Json::as_str)
                .is_some_and(|value| !value.is_empty())
    };
    if let Some(mode) = selected.or_else(|| infer(adapter, settings))
        && (mode.required().iter().any(|key| !has(key))
            || (adapter == AMQP && mode == Auth::Password && !has("password")))
    {
        return Err("connection-auth-required");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn switching_auth_removes_inactive_settings_and_unsaved_secrets() {
        let mut settings = json!({"base_url": "https://example.test", "timeout_ms": 1000,
            "basic": {"user": "user", "password": "credential://example/password"}});
        let mut secrets = Typed::default();
        secrets
            .values
            .insert("basic/password".into(), "draft".into());
        assert_eq!(infer(HTTP, &settings), Some(Auth::Basic));
        choose(Auth::Bearer, &mut settings, &mut secrets);
        assert!(secrets.values.is_empty());
        assert_eq!(
            settings,
            json!({"base_url": "https://example.test", "timeout_ms": 1000})
        );
        assert_eq!(
            validate(HTTP, &settings, &secrets, Some(Auth::Bearer)),
            Err("connection-auth-required")
        );
        secrets.values.insert("bearer".into(), "draft".into());
        assert_eq!(
            validate(HTTP, &settings, &secrets, Some(Auth::Bearer)),
            Ok(())
        );
        assert_eq!(
            validate(HTTP, &json!({}), &Typed::default(), None),
            Err("connection-url-required")
        );
    }

    #[test]
    fn protocols_offer_and_validate_their_own_authentication() {
        let secrets = Typed::default();
        assert!(!modes(WS).contains(&Auth::OAuthClient));
        assert!(modes(HTTP).contains(&Auth::OAuthRefresh));
        let mqtt = json!({"url": "mqtts://broker.test", "username": "user"});
        assert_eq!(validate(MQTT, &mqtt, &secrets, None), Ok(()));
        let amqp = json!({"url": "amqps://broker.test", "username": "user"});
        assert_eq!(
            validate(AMQP, &amqp, &secrets, None),
            Err("connection-auth-required")
        );
        assert_eq!(infer(AMQP, &json!({})), Some(Auth::Guest));
    }
}
