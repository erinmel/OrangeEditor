//! Commands that keep secrets in the operating system's credential store: `secret`,
//! `oauth login` and `feed key` / `feed sign`.
use crate::*;

/// Name under which the operating system's credential store lists this program's
/// secrets.
pub(crate) const VAULT_SERVICE: &str = "OrangeEditor";

/// `secret set|remove|check <name>`: manages one credential in the operating system's
/// store. The secret itself is read from standard input, never from the command line,
/// where the shell's history and the process list would keep it.
pub(crate) fn secret(words: &[String]) -> ExitCode {
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

pub(crate) fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let (mut out, mut at) = (Vec::with_capacity(bytes.len()), 0);
    while let Some(&byte) = bytes.get(at) {
        let escaped = (byte == b'%')
            .then(|| text.get(at + 1..at + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escaped {
            Some(value) => {
                out.push(value);
                at += 3;
            }
            None => {
                out.push(if byte == b'+' { b' ' } else { byte });
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// How long `oauth login` waits for the browser to come back.
pub(crate) const LOGIN_WAIT: Duration = Duration::from_secs(300);

/// `oauth login <settings.json> <name>`: signs a person in with the authorization-code
/// flow and PKCE and keeps the refresh token in the credential store under `<name>`,
/// for a connection's `oauth.refresh_token`. The settings file holds `authorize_url`,
/// `token_url`, `client_id` and optionally `scope`, `client_secret` (text or a
/// `credential://` reference) and `redirect_port` (default 8765). The browser returns
/// to this program on `127.0.0.1`; no token is printed or written to a file.
pub(crate) fn oauth_login(settings: &str, name: &str) -> Result<(), String> {
    use orange_host::SecretStore;
    use orange_net::oauth;
    use std::io::{Read, Write};
    if !orange_host::is_secret_name(name) {
        return Err(format!("{name}: the credential name is not valid"));
    }
    let vault = orange_vault::OsVault::new(VAULT_SERVICE);
    let file = String::from_utf8(read(settings)?).map_err(|_| format!("{settings}: not text"))?;
    let file: serde_json::Value =
        serde_json::from_str(&file).map_err(|_| format!("{settings}: not JSON"))?;
    // A value is text, or a number such as the port.
    let field = |key: &str| -> Result<Option<String>, String> {
        match &file[key] {
            serde_json::Value::Null => Ok(None),
            serde_json::Value::String(text) => Ok(Some(text.clone())),
            serde_json::Value::Number(number) => Ok(Some(number.to_string())),
            _ => Err(format!("{settings}: `{key}` must be text")),
        }
    };
    let required =
        |key: &str| field(key)?.ok_or_else(|| format!("{settings}: `{key}` is required"));
    let (authorize_url, token_url, client_id) = (
        required("authorize_url")?,
        required("token_url")?,
        required("client_id")?,
    );
    if !oauth::is_endpoint(&authorize_url) || !oauth::is_endpoint(&token_url) {
        return Err("the URLs must be https://, or http:// on this machine".into());
    }
    let client_secret = match field("client_secret")? {
        Some(secret) => Some(match secret.strip_prefix(orange_host::SECRET_PREFIX) {
            Some(stored) => vault
                .get(stored)
                .map_err(|error| format!("{stored}: {error}"))?,
            None => secret,
        }),
        None => None,
    };
    let port: u16 = match field("redirect_port")? {
        Some(port) => port.parse().map_err(|_| "`redirect_port` is not a port")?,
        None => 8765,
    };

    let (verifier, state) = (oauth::random_token()?, oauth::random_token()?);
    let redirect = format!("http://127.0.0.1:{port}/callback");
    let challenge = oauth::pkce_challenge(&verifier);
    let mut parameters = vec![
        ("response_type", "code"),
        ("client_id", client_id.as_str()),
        ("redirect_uri", redirect.as_str()),
        ("state", state.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
    ];
    let scope = field("scope")?;
    if let Some(scope) = &scope {
        parameters.push(("scope", scope));
    }
    let listener = std::net::TcpListener::bind(("127.0.0.1", port))
        .map_err(|error| format!("port {port} cannot be listened on: {:?}", error.kind()))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    println!(
        "Open this address in a browser and sign in:\n{}",
        oauth::authorization_url(&authorize_url, &parameters)
    );

    let started = Instant::now();
    let code = loop {
        if started.elapsed() > LOGIN_WAIT {
            return Err("nobody signed in within five minutes".into());
        }
        let Ok((mut stream, _)) = listener.accept() else {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        };
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut buffer = [0u8; 8192];
        let count = stream.read(&mut buffer).unwrap_or(0);
        let request = String::from_utf8_lossy(buffer.get(..count).unwrap_or_default()).into_owned();
        let query = request
            .strip_prefix("GET /callback?")
            .and_then(|rest| rest.split([' ', '\r', '\n']).next())
            .unwrap_or_default();
        let value = |key: &str| {
            query
                .split('&')
                .filter_map(|pair| pair.split_once('='))
                .find(|(name, _)| *name == key)
                .map(|(_, value)| percent_decode(value))
        };
        // Anything that is not this sign-in's own return is answered and ignored.
        let outcome = match (value("state"), value("code"), value("error")) {
            (Some(returned), Some(code), _) if returned == state => Some(Ok(code)),
            (Some(returned), _, Some(_)) if returned == state => {
                Some(Err("the sign-in was refused".to_owned()))
            }
            _ => None,
        };
        let page = match &outcome {
            Some(Ok(_)) => "Signed in. You can close this page.",
            Some(Err(_)) => "The sign-in was refused. You can close this page.",
            None => "This address is not expecting you.",
        };
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
            page.len()
        );
        if let Some(outcome) = outcome {
            break outcome?;
        }
    };

    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", redirect.as_str()),
        ("client_id", client_id.as_str()),
        ("code_verifier", verifier.as_str()),
    ];
    if let Some(secret) = &client_secret {
        form.push(("client_secret", secret));
    }
    let answer = oauth::token_request(&token_url, &form, None)?;
    let refresh = answer
        .refresh_token
        .ok_or("the server issued no refresh token; ask it for offline access")?;
    vault
        .set(name, &refresh)
        .map_err(|error| format!("{name}: {error}"))?;
    println!(
        "stored the refresh token as {name}\nuse it in a connection's settings as: \"oauth\": {{\"token_url\": \"{token_url}\", \"client_id\": \"{client_id}\", \"refresh_token\": \"credential://{name}\"}}"
    );
    Ok(())
}

/// `feed key <name>` makes a signing key and keeps its private half in the credential
/// store under `<name>`; `feed sign <feed.json> <name>` writes `<feed.json>.sig`. Both
/// print the public key a manifest's `update.key` takes. The private key is never
/// printed or written to a file.
pub(crate) fn feed_command(words: &[String]) -> ExitCode {
    use orange_host::SecretStore;
    let vault = orange_vault::OsVault::new(VAULT_SERVICE);
    let done = match &words.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["key", name] => {
            // An existing key is never replaced: feeds signed with it would stop verifying.
            match vault.get(name) {
                Ok(_) => Err(format!("{name} already holds a key; choose another name")),
                Err(orange_host::SecretError::NotFound) => {
                    orange_net::new_feed_key().and_then(|(private, public)| {
                        vault
                            .set(name, &private)
                            .map(|()| {
                                format!("stored the private key as {name}\nupdate.key: {public}")
                            })
                            .map_err(|error| error.to_string())
                    })
                }
                Err(error) => Err(error.to_string()),
            }
        }
        ["sign", feed, name] => vault
            .get(name)
            .map_err(|error| format!("{name}: {error}"))
            .and_then(|private| {
                let bytes = read(feed)?;
                orange_host::UpdateFeed::from_json(&bytes)
                    .map_err(|error| format!("{feed}: {error}"))?;
                let (signature, public) = orange_net::sign_feed(&bytes, &private)?;
                let output = format!("{feed}.sig");
                std::fs::write(&output, &signature)
                    .map_err(|error| format!("{output}: {error}"))?;
                Ok(format!("{output}: signed\nupdate.key: {public}"))
            }),
        _ => {
            eprintln!("feed needs `key <name>` or `sign <feed.json> <name>`\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match done {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
