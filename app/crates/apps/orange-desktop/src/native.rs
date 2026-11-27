//! Native plugin libraries (`orange-native`) with full access, at the person's own risk.
//!
//! A library in the plugins folder is machine code that runs inside the application with
//! all of its rights: it can read files, use the network and devices, read the
//! operating system's credential store as the person, and crash the program. Nothing
//! inspects or limits it. So a library loads only after the person allowed full access
//! to that exact file: its name and SHA-256 are kept in `full-access.json` beside the
//! libraries. A changed file is a new file and asks again. Loading runs a library's
//! code, so a library is never loaded to find out what it is: the dialog names the file.
//!
//! Native plugins load when the application starts ([`orange_native`]), so an approval
//! or a revocation takes effect at the next start.
use orange_editor_ui::i18n::{t, tf};
use serde_json::{Value as Json, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The file in the plugins folder that lists the libraries allowed full access.
const APPROVALS: &str = "full-access.json";
const FORMAT: &str = "orange.full-access-plugins";
/// Largest library read to compute its digest.
const MAX_LIBRARY_BYTES: u64 = 512 << 20;

/// A library file in the plugins folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Library {
    /// Its file name, which is what the person is shown.
    pub file: String,
    /// `sha256:<hex>` of its bytes.
    pub digest: String,
    pub path: PathBuf,
}

/// The plugins folder and what was decided about each library in it.
#[derive(Debug, Default)]
pub struct NativePlugins {
    directory: PathBuf,
    /// Approved file names and the digest approved for each.
    approved: BTreeMap<String, String>,
    /// Libraries waiting for the person's decision.
    pub pending: Vec<Library>,
    /// Packages loaded at this start: library file, package ID and version.
    pub loaded: Vec<(String, String, String)>,
    /// Libraries allowed but refused by the loader, with why.
    pub failed: Vec<(String, String)>,
    /// Decisions made in this session that take effect at the next start.
    pub restart_needed: bool,
    /// The dialog is open.
    pub open: bool,
}

/// The libraries of a folder, in file-name order, with their digests. A file that
/// cannot be read is left out.
pub fn scan(directory: &Path) -> Vec<Library> {
    let Ok(paths) = orange_native::libraries(directory) else {
        return Vec::new();
    };
    paths
        .into_iter()
        .filter_map(|path| {
            let size = std::fs::metadata(&path).ok()?.len();
            if size > MAX_LIBRARY_BYTES {
                return None;
            }
            let bytes = std::fs::read(&path).ok()?;
            Some(Library {
                file: path.file_name()?.to_string_lossy().into_owned(),
                digest: orange_net::sha256(&bytes),
                path,
            })
        })
        .collect()
}

fn read_approvals(directory: &Path) -> BTreeMap<String, String> {
    let Ok(text) = std::fs::read_to_string(directory.join(APPROVALS)) else {
        return BTreeMap::new();
    };
    let Ok(file) = serde_json::from_str::<Json>(&text) else {
        return BTreeMap::new();
    };
    if file["format"] != FORMAT || file["format_version"] != 1 {
        return BTreeMap::new();
    }
    file["approved"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            Some((
                entry["file"].as_str()?.to_owned(),
                entry["sha256"].as_str()?.to_owned(),
            ))
        })
        .collect()
}

impl NativePlugins {
    /// Reads the folder and its approvals without loading anything, creating the folder
    /// so the person can find where libraries go.
    pub fn open(directory: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&directory);
        let approved = read_approvals(&directory);
        Self {
            directory,
            approved,
            ..Self::default()
        }
    }

    /// Whether `library`, exactly as it is now, was allowed full access.
    pub fn is_approved(&self, library: &Library) -> bool {
        self.approved.get(&library.file) == Some(&library.digest)
    }

    /// The libraries allowed full access, as `(file, digest)`.
    #[cfg(test)]
    pub fn approved(&self) -> impl Iterator<Item = (&str, &str)> {
        self.approved
            .iter()
            .map(|(file, digest)| (file.as_str(), digest.as_str()))
    }

    /// Splits the folder's libraries into the approved ones, to load, and the others,
    /// which wait in [`pending`](Self::pending) and are not loaded.
    pub fn sort(&mut self) -> Vec<Library> {
        let (approved, pending) = scan(&self.directory)
            .into_iter()
            .partition(|library| self.is_approved(library));
        self.pending = pending;
        self.open = !self.pending.is_empty();
        approved
    }

    /// Loads the approved libraries and returns their packages for the host.
    pub fn load(&mut self) -> Vec<orange_sdk::ServicePackage> {
        let mut packages = Vec::new();
        for library in self.sort() {
            match orange_native::load(&library.path) {
                Ok(package) => {
                    self.loaded.push((
                        library.file,
                        package.id().to_owned(),
                        package.version().to_owned(),
                    ));
                    packages.push(package);
                }
                Err(error) => self.failed.push((library.file, error.to_string())),
            }
        }
        packages
    }

    /// Allows full access to `library` as it is now, from the next start.
    pub fn approve(&mut self, library: &Library) -> Result<(), String> {
        let _ = self
            .approved
            .insert(library.file.clone(), library.digest.clone());
        self.pending.retain(|pending| pending != library);
        self.restart_needed = true;
        self.save()
    }

    /// Takes full access back from a library file, from the next start.
    pub fn revoke(&mut self, file: &str) -> Result<(), String> {
        if self.approved.remove(file).is_some() {
            self.restart_needed = true;
        }
        self.save()
    }

    fn save(&self) -> Result<(), String> {
        let approved: Vec<Json> = self
            .approved
            .iter()
            .map(|(file, digest)| json!({"file": file, "sha256": digest}))
            .collect();
        let text = json!({"format": FORMAT, "format_version": 1, "approved": approved});
        let target = self.directory.join(APPROVALS);
        std::fs::create_dir_all(&self.directory)
            .and_then(|()| orange_sandbox::replace_file(&target, text.to_string().as_bytes()))
            .map_err(|error| format!("{:?}", error.kind()))
    }

    /// The dialog: the risk in plain words, each library waiting for a decision, and the
    /// ones allowed, which can be taken back.
    pub fn dialog(&mut self, ctx: &eframe::egui::Context) {
        use eframe::egui;
        if !self.open {
            return;
        }
        let mut open = true;
        let mut approve = None;
        let mut revoke = None;
        let _ = egui::Window::new(t("desktop-native-title"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(460.0)
            .show(ctx, |ui| {
                ui.colored_label(ui.visuals().warn_fg_color, t("desktop-native-risk"));
                ui.label(t("desktop-native-vault"));
                ui.weak(tf(
                    "desktop-native-folder",
                    &[("folder", self.directory.display().to_string().into())],
                ));
                for library in &self.pending {
                    ui.separator();
                    ui.strong(&library.file);
                    ui.weak(&library.digest);
                    if ui.button(t("desktop-native-allow")).clicked() {
                        approve = Some(library.clone());
                    }
                }
                if !self.loaded.is_empty() || self.approved.len() > self.pending.len() {
                    ui.separator();
                    ui.label(t("desktop-native-allowed"));
                }
                for (file, id, version) in &self.loaded {
                    ui.horizontal(|ui| {
                        ui.label(format!("{file} — {id} {version}"));
                        if ui.button(t("desktop-native-revoke")).clicked() {
                            revoke = Some(file.clone());
                        }
                    });
                }
                for (file, error) in &self.failed {
                    ui.horizontal(|ui| {
                        ui.label(tf(
                            "desktop-native-failed",
                            &[
                                ("file", file.clone().into()),
                                ("error", error.clone().into()),
                            ],
                        ));
                        if ui.button(t("desktop-native-revoke")).clicked() {
                            revoke = Some(file.clone());
                        }
                    });
                }
                if self.restart_needed {
                    ui.separator();
                    ui.label(t("desktop-native-restart"));
                }
            });
        if let Some(library) = approve
            && let Err(error) = self.approve(&library)
        {
            eprintln!("full-access approval was not saved: {error}");
        }
        if let Some(file) = revoke {
            if let Err(error) = self.revoke(&file) {
                eprintln!("full-access revocation was not saved: {error}");
            }
            self.loaded.retain(|(loaded, ..)| *loaded != file);
            self.failed.retain(|(failed, _)| *failed != file);
        }
        self.open = open;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "orange-native-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    /// Nothing loads without an approval of the exact file; a changed file asks again,
    /// a revoked one waits again, and the decisions survive a restart.
    #[test]
    fn a_library_loads_only_with_full_access_for_its_exact_bytes() {
        let directory = folder("approve");
        let mut plugins = NativePlugins::open(directory.clone());
        let library = directory.join(format!("tool.{}", std::env::consts::DLL_EXTENSION));
        std::fs::write(&library, b"first build").unwrap();
        std::fs::write(directory.join("notes.txt"), b"not a library").unwrap();

        assert!(plugins.sort().is_empty());
        assert_eq!(plugins.pending.len(), 1);
        assert!(plugins.open);
        let pending = plugins.pending[0].clone();
        plugins.approve(&pending).unwrap();
        assert!(plugins.restart_needed);

        let mut restarted = NativePlugins::open(directory.clone());
        assert_eq!(restarted.sort(), vec![pending.clone()]);
        assert!(restarted.pending.is_empty());
        assert!(!restarted.open);

        // A rebuilt library is another file to the person: it asks again.
        std::fs::write(&library, b"second build").unwrap();
        let mut changed = NativePlugins::open(directory.clone());
        assert!(changed.sort().is_empty());
        assert_eq!(changed.pending.len(), 1);
        assert_ne!(changed.pending[0].digest, pending.digest);

        let mut revoking = NativePlugins::open(directory.clone());
        revoking.revoke(&pending.file).unwrap();
        assert_eq!(revoking.approved().count(), 0);
        let mut after = NativePlugins::open(directory.clone());
        assert!(after.sort().is_empty());
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// The dialog opens at start for a waiting library, states the risk, and its button
    /// records the approval for the next start.
    #[test]
    fn the_dialog_names_the_risk_and_its_button_allows_full_access() {
        use eframe::egui;
        let directory = folder("dialog");
        let mut plugins = NativePlugins::open(directory.clone());
        std::fs::write(
            directory.join(format!("tool.{}", std::env::consts::DLL_EXTENSION)),
            b"bytes",
        )
        .unwrap();
        assert!(plugins.load().is_empty());
        assert!(plugins.open);
        let context = egui::Context::default();
        let frame = |plugins: &mut NativePlugins, events| {
            context.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| plugins.dialog(ui.ctx()),
            )
        };
        let (mut risk, mut allow) = (false, None);
        for _ in 0..3 {
            let output = frame(&mut plugins, Vec::new());
            for shape in &output.shapes {
                if let egui::Shape::Text(text) = &shape.shape {
                    risk |= text.galley.text() == t("desktop-native-risk");
                    if text.galley.text() == t("desktop-native-allow") {
                        allow = Some(text.pos + text.galley.size() * 0.5);
                    }
                }
            }
            output.drop_without_applying_deltas();
        }
        assert!(risk, "the risk is stated");
        orange_testkit::ui::click(
            allow.expect("allow button"),
            egui::PointerButton::Primary,
            |events| {
                frame(&mut plugins, events).drop_without_applying_deltas();
            },
        );
        assert!(plugins.pending.is_empty());
        assert!(plugins.restart_needed);
        let mut restarted = NativePlugins::open(directory.clone());
        assert_eq!(restarted.sort().len(), 1);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn an_unreadable_approval_file_approves_nothing() {
        let directory = folder("broken");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(APPROVALS), b"{not json").unwrap();
        std::fs::write(
            directory.join(format!("tool.{}", std::env::consts::DLL_EXTENSION)),
            b"bytes",
        )
        .unwrap();
        let mut plugins = NativePlugins::open(directory.clone());
        assert!(plugins.sort().is_empty());
        assert_eq!(plugins.pending.len(), 1);
        let _ = std::fs::remove_dir_all(&directory);
    }
}
