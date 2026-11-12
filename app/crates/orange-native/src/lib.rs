//! Loads native plugin libraries: dynamic libraries built on `orange-sdk` that export a
//! [`ServicePackage`] with `orange_sdk::export_plugin!`.
//!
//! An application calls [`load`] or [`load_directory`] when it starts and passes the
//! packages to its host with the ones it was built with. From there a loaded package
//! is an ordinary service package: its nodes, triggers and connection kinds work like
//! built-in ones.
//!
//! # What this does not protect against
//!
//! A native plugin is machine code running inside the application with all of its
//! rights: files, network, devices, memory. Nothing here limits or inspects it. It is
//! installed at the user's own risk, like any program they choose to run.
//!
//! The library and the application exchange Rust types directly, and Rust has no
//! stable binary interface. [`load`] therefore refuses a library unless it reports the
//! same SDK revision, compiler and target as the application ([`orange_sdk::ABI`]).
//! That check cannot see dependency versions or features: a plugin must be built
//! against the application's lockfile, or behavior is undefined. A panic inside a
//! plugin ends the application. A loaded library stays loaded until the process ends.
// Product code reports failures as typed errors; each remaining panic is a documented
// internal invariant (10, "Panic policy").
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

use libloading::Library;
use orange_sdk::{ABI, ABI_SYMBOL, PACKAGE_SYMBOL, ServicePackage};
use std::ffi::{CStr, c_char, c_void};
use std::path::{Path, PathBuf};

/// Why a library was not loaded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum LoadError {
    /// The operating system could not load the file as a library.
    #[error("the library could not be opened: {0}")]
    Open(String),
    /// The library does not export the plugin symbols.
    #[error("the library is not an OrangeEditor plugin")]
    NotAPlugin,
    /// The library was built with another SDK revision, compiler or target.
    #[error("the plugin was built for `{found}`, and this application needs `{expected}`")]
    Abi {
        /// What this application was built with.
        expected: String,
        /// What the library reports.
        found: String,
    },
    /// The library's package function returned nothing.
    #[error("the plugin returned no package")]
    NoPackage,
    /// The directory could not be read.
    #[error("the plugin directory could not be read: {0}")]
    Directory(String),
}

type AbiFn = unsafe extern "C" fn() -> *const c_char;
type PackageFn = unsafe extern "C" fn() -> *mut c_void;

/// Loads one plugin library and returns the package it exports.
///
/// Loading runs the library's initializers, and the returned package's code is the
/// library's: see the crate documentation for what that implies.
pub fn load(path: &Path) -> Result<ServicePackage, LoadError> {
    // SAFETY: loading a library runs its initializers, which this crate cannot make
    // safe; the caller chose to run this file as a plugin (crate documentation).
    #[allow(unsafe_code)]
    let library =
        unsafe { Library::new(path) }.map_err(|error| LoadError::Open(error.to_string()))?;

    // SAFETY: the symbol is only read as a function pointer here. Its signature is
    // fixed by `export_plugin!` and takes no arguments, so calling a function of that
    // name from a library that is not a plugin is the residual risk of loading it.
    #[allow(unsafe_code)]
    let found = unsafe {
        let abi = library
            .get::<AbiFn>(ABI_SYMBOL)
            .map_err(|_| LoadError::NotAPlugin)?;
        let text = abi();
        if text.is_null() {
            return Err(LoadError::NotAPlugin);
        }
        // `export_plugin!` returns a pointer to a NUL-terminated constant.
        CStr::from_ptr(text).to_owned()
    };
    if found.as_c_str() != ABI {
        return Err(LoadError::Abi {
            expected: ABI.to_string_lossy().into_owned(),
            found: found.to_string_lossy().into_owned(),
        });
    }

    // SAFETY: the ABI text matched, so the library was built with this SDK revision,
    // compiler and target, and `orange_plugin_package` returns `Box<ServicePackage>`
    // as a raw pointer whose ownership passes to the caller.
    #[allow(unsafe_code)]
    let package = unsafe {
        let package = library
            .get::<PackageFn>(PACKAGE_SYMBOL)
            .map_err(|_| LoadError::NotAPlugin)?;
        let raw = package().cast::<ServicePackage>();
        if raw.is_null() {
            return Err(LoadError::NoPackage);
        }
        *Box::from_raw(raw)
    };
    // The package holds code and data of the library for as long as it lives, and
    // flows may keep parts of it beyond that. The library is never unloaded.
    std::mem::forget(library);
    Ok(package)
}

/// One library of a directory and what loading it gave.
pub type Loaded = (PathBuf, Result<ServicePackage, LoadError>);

/// Loads every library in a directory, in file-name order: files with the platform's
/// library extension (`.dll`, `.so`, `.dylib`). Other files are ignored. Each library
/// is reported with its own result, so one bad plugin does not hide the others.
pub fn load_directory(directory: &Path) -> Result<Vec<Loaded>, LoadError> {
    let entries = std::fs::read_dir(directory)
        .map_err(|error| LoadError::Directory(format!("{:?}", error.kind())))?;
    let mut libraries: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension == std::env::consts::DLL_EXTENSION)
        })
        .collect();
    libraries.sort();
    Ok(libraries
        .into_iter()
        .map(|path| {
            let loaded = load(&path);
            (path, loaded)
        })
        .collect())
}
