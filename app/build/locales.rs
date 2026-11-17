//! Build script shared by every crate that ships translated text.
//!
//! It lists each `locales/<language tag>/*.ftl` file of the crate in
//! `$OUT_DIR/locales.rs` as `CATALOGS: &[(&str, &str)]`, the tag and the file's text,
//! so a new language is a new folder and no code changes. The crate brings the list in
//! with `include!(concat!(env!("OUT_DIR"), "/locales.rs"));`.
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::{env, fs};

fn main() {
    let manifest = env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let out = env::var_os("OUT_DIR").expect("cargo sets OUT_DIR");
    let root = Path::new(&manifest).join("locales");
    println!("cargo::rerun-if-changed={}", root.display());
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    for language in fs::read_dir(&root).into_iter().flatten().flatten() {
        let tag = language.file_name().to_string_lossy().into_owned();
        if !language.path().is_dir() {
            continue;
        }
        for file in fs::read_dir(language.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            let path = file.path();
            if path.extension().is_some_and(|extension| extension == "ftl") {
                files.push((tag.clone(), path));
            }
        }
    }
    files.sort();
    let mut list = String::from(
        "/// Every catalogue in `locales/`, as (language tag, Fluent text).\n\
         pub(crate) const CATALOGS: &[(&str, &str)] = &[\n",
    );
    for (tag, path) in files {
        let _ = writeln!(
            list,
            "    ({tag:?}, include_str!({:?})),",
            path.display().to_string()
        );
    }
    list.push_str("];\n");
    fs::write(Path::new(&out).join("locales.rs"), list).expect("OUT_DIR is writable");
}
