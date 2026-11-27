//! Stores, checks or removes one secret in the operating system's store:
//! `secret set <name> <value>`, `secret check <name>`, `secret remove <name>`. It is
//! for trying the store on a machine; the `orange secret` command is the real tool and
//! never takes a secret as an argument.
use orange_sdk::SecretStore;
use orange_vault::OsVault;

fn main() -> std::process::ExitCode {
    let words: Vec<String> = std::env::args().skip(1).collect();
    let vault = OsVault::new("orange-editor-example");
    let done = match &words.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["set", name, value] => vault.set(name, value).map(|()| "stored".to_owned()),
        ["check", name] => vault
            .get(name)
            .map(|value| format!("stored, {} bytes", value.len())),
        ["remove", name] => vault.remove(name).map(|()| "removed".to_owned()),
        _ => {
            eprintln!("usage: secret set <name> <value> | check <name> | remove <name>");
            return std::process::ExitCode::from(2);
        }
    };
    match done {
        Ok(message) => {
            println!("{message}");
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
