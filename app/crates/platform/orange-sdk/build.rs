//! Records the compiler and target this SDK is built with. A plugin library and the
//! application that loads it exchange Rust types directly, which is only sound when
//! both were built by the same compiler for the same target; `ABI` carries this text
//! so the loader can refuse anything else.
use std::process::Command;

fn main() {
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let version = Command::new(rustc)
        .arg("-vV")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .unwrap_or_default();
    let field = |name: &str| {
        version
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .map_or("unknown", str::trim)
            .to_owned()
    };
    println!(
        "cargo::rustc-env=ORANGE_SDK_RUSTC={} {}",
        field("release:"),
        field("commit-hash:")
    );
    println!(
        "cargo::rustc-env=ORANGE_SDK_TARGET={}",
        std::env::var("TARGET").unwrap_or_else(|_| "unknown".into())
    );
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed=RUSTC");
}
