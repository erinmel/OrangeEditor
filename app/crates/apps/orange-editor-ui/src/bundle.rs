//! Sharing a graph with the packages it needs: one file that carries the document and
//! the archives of the packages it is locked to. Opening one installs nothing until the
//! person approves each missing package's grants, and the document opens unchanged:
//! a node whose package stays missing keeps its settings and wires.
use super::*;
use crate::controls::Action;
#[cfg(test)]
use orange_host::intrinsic;
use orange_host::{Bundle, Grants};

/// The file name extension of a shared graph.
pub const BUNDLE_EXTENSION: &str = "orangebundle";

/// One package a shared graph carries.
#[derive(Debug)]
pub(super) struct CarriedPackage {
    pub(super) id: String,
    pub(super) version: String,
    pub(super) archive: Vec<u8>,
    /// This host already has this exact version.
    pub(super) installed: bool,
    pub(super) requested: Grants,
    pub(super) approved: Grants,
}

/// A shared graph waiting for the person to choose what to install.
#[derive(Debug)]
pub(super) struct BundleImport {
    pub(super) name: String,
    pub(super) document: String,
    pub(super) packages: Vec<CarriedPackage>,
    /// Packages the graph needs that neither this host nor the file has, as
    /// `id version`.
    pub(super) not_included: Vec<String>,
}

impl Editor {
    /// Asks the application to save the open graph together with the stored archives of
    /// the packages it uses. Packages this host holds no archive for (built in, or
    /// installed without a package directory) are named in the log: whoever opens the
    /// file needs them separately.
    pub fn share_with_packages(&mut self, host: &Host) {
        let Some(document) = &self.document else {
            return;
        };
        let text = match document.draft.text() {
            Ok(text) => text,
            Err(error) => {
                self.say_at(NoteLevel::Error, error.to_string());
                return;
            }
        };
        let stem = std::path::Path::new(&document.path)
            .file_stem()
            .map_or_else(
                || "graph".into(),
                |stem| stem.to_string_lossy().into_owned(),
            );
        match host.export(&text) {
            Ok((bundle, missing)) => {
                for requirement in missing {
                    self.say_at(
                        NoteLevel::Warning,
                        tf(
                            "bundle-not-included",
                            &[
                                ("package", requirement.package.into()),
                                ("version", requirement.version.into()),
                            ],
                        ),
                    );
                }
                self.request = Some(FileRequest::ExportBundle {
                    name: format!("{stem}.{BUNDLE_EXTENSION}"),
                    bundle: bundle.to_bytes(),
                });
            }
            Err(error) => self.say_at(
                NoteLevel::Error,
                tf(
                    "bundle-export-failed",
                    &[("error", error.to_string().into())],
                ),
            ),
        }
    }

    /// Reads a shared graph and shows what it carries before anything is installed.
    pub fn review_bundle(
        &mut self,
        host: &Host,
        name: String,
        bytes: &[u8],
    ) -> Result<(), HostError> {
        let bundle = Bundle::from_bytes(bytes)?;
        let requirements = host.requirements(&bundle.document)?;
        let mut packages = Vec::new();
        for archive in bundle.packages {
            let key = host.archive_key(&archive)?;
            let (id, version) = (key.id.to_string(), key.version.to_string());
            let installed = requirements.iter().any(|requirement| {
                requirement.package == id && requirement.version == version && requirement.available
            }) || host
                .packages()
                .any(|package| package.id == id && package.version == version);
            packages.push(CarriedPackage {
                requested: host.permissions(&archive)?,
                approved: Grants::default(),
                id,
                version,
                archive,
                installed,
            });
        }
        let not_included = requirements
            .iter()
            .filter(|requirement| !requirement.available)
            .filter(|requirement| {
                !packages.iter().any(|package| {
                    package.id == requirement.package && package.version == requirement.version
                })
            })
            .map(|requirement| format!("{} {}", requirement.package, requirement.version))
            .collect();
        self.pending_bundle = Some(BundleImport {
            name,
            document: bundle.document,
            packages,
            not_included,
        });
        Ok(())
    }

    /// Installs the carried packages this host lacks, with the grants the person
    /// ticked, then opens the graph as a new, unsaved document. A package that fails to
    /// install is reported and the graph still opens, keeping its nodes.
    pub(super) fn install_bundle(&mut self, host: &mut Host, import: BundleImport) {
        for package in import.packages.iter().filter(|package| !package.installed) {
            self.install_approved_package(host, &package.archive, package.approved);
        }
        self.previews.invalidate();
        self.open_untitled(host, import.document);
        self.panels.log = true;
        self.log_events = true;
    }

    pub(super) fn bundle_import_dialog(&mut self, ctx: &egui::Context, host: &mut Host) {
        let Some(import) = &mut self.pending_bundle else {
            return;
        };
        let missing = import
            .packages
            .iter()
            .filter(|package| !package.installed)
            .count();
        let modal = orange_ui_controls::modal(ctx, egui::Id::unique("orange-import-bundle")).show(
            ctx,
            |ui| {
                ui.set_max_width(460.0);
                ui.heading(t("open-shared-graph"));
                ui.label(&import.name);
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .show(ui, |ui| {
                        for package in &mut import.packages {
                            ui.separator();
                            ui.strong(format!("{} {}", package.id, package.version));
                            if package.installed {
                                ui.weak(t("bundle-package-installed"));
                            } else {
                                let _ = package_import::grant_choices(
                                    ui,
                                    package.requested,
                                    &mut package.approved,
                                );
                            }
                        }
                        if !import.not_included.is_empty() {
                            ui.separator();
                            ui.label(t("bundle-not-carried"));
                            for line in &import.not_included {
                                ui.weak(line);
                            }
                        }
                    });
                ui.separator();
                let open = controls::button(
                    ui,
                    Surface::Dialog,
                    Action::Secondary,
                    true,
                    egui::Button::new(t("bundle-open-only")),
                );
                let label = if missing == 0 {
                    t("bundle-open")
                } else {
                    t("bundle-install-and-open")
                };
                let (cancel, install) =
                    controls::dialog_actions(ui, Action::Primary, true, egui::Button::new(label));
                (open, cancel, install)
            },
        );
        let (open, cancel, install) = &modal.inner;
        if install.clicked() {
            if let Some(import) = self.pending_bundle.take() {
                self.install_bundle(host, import);
            }
        } else if open.clicked() {
            if let Some(import) = self.pending_bundle.take() {
                self.open_untitled(host, import.document);
            }
        } else if cancel.clicked() || modal.should_close() {
            self.pending_bundle = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orange_host::{HostOptions, PackageKind};

    fn remember_archive() -> Vec<u8> {
        let manifest = include_bytes!("../../orange-cli/packages/remember/manifest.json");
        let files = [
            (
                "nodes/remember.json".to_owned(),
                include_bytes!("../../orange-cli/packages/remember/nodes/remember.json").to_vec(),
            ),
            (
                "graphs/remember.json".to_owned(),
                include_bytes!("../../orange-cli/packages/remember/graphs/remember.json").to_vec(),
            ),
        ];
        orange_host::pack(manifest, &files).unwrap()
    }

    fn store(name: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "orange-editor-bundle-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    /// A graph shared with its package opens on a host without it: the card first names
    /// the missing package, the dialog installs it, and the graph then checks clean
    /// with the same nodes and wires.
    #[test]
    fn a_shared_graph_brings_its_package_and_opens_with_its_wires() {
        let sender_store = store("sender");
        let (mut sender, _) = Host::new(HostOptions {
            package_dir: Some(sender_store.clone()),
            ..HostOptions::default()
        })
        .unwrap();
        let _ = sender
            .install(&remember_archive(), Grants::default())
            .unwrap();
        let mut editor = Editor::new();
        editor.open_untitled(&mut sender, orange_host::new_document().unwrap());
        let start = editor.add_node(&mut sender, intrinsic::ON_MANUAL).unwrap();
        let remember = editor
            .add_node(&mut sender, "org.example.tools/remember")
            .unwrap();
        assert!(editor.connect(&mut sender, (start, "then"), (remember, "in")));
        assert!(editor.set_input(&mut sender, (remember, "value"), Some("5")));
        let sent_wires = editor.graph().unwrap().wires.len();
        editor.share_with_packages(&sender);
        let Some(FileRequest::ExportBundle { name, bundle }) = editor.take_request() else {
            panic!("sharing asks where to save the file");
        };
        assert!(name.ends_with(".orangebundle"));

        let (mut receiver, _) = Host::new(HostOptions::default()).unwrap();
        assert!(
            receiver
                .packages()
                .all(|package| package.id != "org.example.tools")
        );
        let mut opened = Editor::new();
        opened.review_bundle(&receiver, name, &bundle).unwrap();
        // Opening without installing keeps the node and its wire, and names the package.
        let pending = opened.pending_bundle.take().unwrap();
        assert_eq!(pending.packages.len(), 1);
        assert!(!pending.packages[0].installed);
        assert!(pending.not_included.is_empty());
        opened.open_untitled(&mut receiver, pending.document.clone());
        let card = opened
            .graph()
            .unwrap()
            .nodes
            .iter()
            .find(|card| card.id == remember)
            .cloned()
            .unwrap();
        assert_eq!(
            card.missing
                .as_ref()
                .map(|missing| missing.package.as_str()),
            Some("org.example.tools")
        );
        assert_eq!(opened.graph().unwrap().wires.len(), sent_wires);

        // Installing what it carries makes the same graph whole.
        opened.install_bundle(&mut receiver, pending);
        assert!(receiver.packages().any(|package| {
            package.id == "org.example.tools" && matches!(package.kind, PackageKind::Installed)
        }));
        let card = opened
            .graph()
            .unwrap()
            .nodes
            .iter()
            .find(|card| card.id == remember)
            .cloned()
            .unwrap();
        assert_eq!(card.missing, None);
        assert_eq!(opened.graph().unwrap().wires.len(), sent_wires);
        let problems = &opened.document.as_ref().unwrap().problems;
        assert!(problems.is_empty(), "{problems:?}");

        // A second look sees it installed and offers nothing more.
        opened
            .review_bundle(&receiver, "again".into(), &bundle)
            .unwrap();
        assert!(opened.pending_bundle.as_ref().unwrap().packages[0].installed);
        let _ = std::fs::remove_dir_all(&sender_store);
    }

    /// The dialog lists the carried package with its grant boxes, and Install and open
    /// installs it and opens the graph.
    #[test]
    fn the_shared_graph_dialog_installs_and_opens() {
        let sender_store = store("dialog");
        let (mut sender, _) = Host::new(HostOptions {
            package_dir: Some(sender_store.clone()),
            ..HostOptions::default()
        })
        .unwrap();
        let _ = sender
            .install(&remember_archive(), Grants::default())
            .unwrap();
        let mut editor = Editor::new();
        editor.open_untitled(&mut sender, orange_host::new_document().unwrap());
        let _ = editor
            .add_node(&mut sender, "org.example.tools/remember")
            .unwrap();
        editor.share_with_packages(&sender);
        let Some(FileRequest::ExportBundle { bundle, .. }) = editor.take_request() else {
            panic!("sharing asks where to save the file");
        };

        let (mut host, _) = Host::new(HostOptions::default()).unwrap();
        let mut opened = Editor::new();
        opened
            .review_bundle(&host, "shared.orangebundle".into(), &bundle)
            .unwrap();
        let context = egui::Context::default();
        let mut frame = |editor: &mut Editor, events| {
            context.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| editor.bundle_import_dialog(ui.ctx(), &mut host),
            )
        };
        let (mut listed, mut install) = (false, None);
        for _ in 0..3 {
            let output = frame(&mut opened, Vec::new());
            for shape in &output.shapes {
                if let egui::Shape::Text(text) = &shape.shape {
                    listed |= text.galley.text() == "org.example.tools 1.0.0";
                    if text.galley.text() == t("bundle-install-and-open") {
                        install = Some(text.pos + text.galley.size() * 0.5);
                    }
                }
            }
            output.drop_without_applying_deltas();
        }
        assert!(listed, "the carried package is listed");
        orange_testkit::ui::click(
            install.expect("install button"),
            egui::PointerButton::Primary,
            |events| {
                frame(&mut opened, events).drop_without_applying_deltas();
            },
        );
        assert!(opened.pending_bundle.is_none());
        assert!(opened.document.is_some());
        assert!(
            host.packages()
                .any(|package| package.id == "org.example.tools")
        );
        let _ = std::fs::remove_dir_all(&sender_store);
    }

    /// Properties of a node whose package is missing names the package and offers to
    /// install one, which asks the application for an archive.
    #[test]
    fn a_missing_package_node_offers_to_install_its_package() {
        let (mut sender, _) = Host::new(HostOptions::default()).unwrap();
        let _ = sender
            .install(&remember_archive(), Grants::default())
            .unwrap();
        let mut editor = Editor::new();
        editor.open_untitled(&mut sender, orange_host::new_document().unwrap());
        let remember = editor
            .add_node(&mut sender, "org.example.tools/remember")
            .unwrap();
        let text = editor.document.as_ref().unwrap().draft.text().unwrap();

        let (mut host, _) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.file_dialogs = true;
        editor.open_untitled(&mut host, text);
        editor.canvas().select(Some(remember));
        let needs = tf(
            "node-needs-package",
            &[
                ("package", "org.example.tools".into()),
                ("version", "1.0.0".into()),
            ],
        );
        let context = egui::Context::default();
        let mut frame = |editor: &mut Editor, events| {
            context.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| editor.properties_panel(ui, &mut host),
            )
        };
        let (mut shown, mut button) = (false, None);
        for _ in 0..3 {
            let output = frame(&mut editor, Vec::new());
            for shape in &output.shapes {
                if let egui::Shape::Text(text) = &shape.shape {
                    shown |= text.galley.text() == needs;
                    if text.galley.text() == t("install-missing-package") {
                        button = Some(text.pos + text.galley.size() * 0.5);
                    }
                }
            }
            output.drop_without_applying_deltas();
        }
        assert!(shown, "the notice names the package and version");
        let pos = button.expect("install button");
        orange_testkit::ui::click(pos, egui::PointerButton::Primary, |events| {
            frame(&mut editor, events).drop_without_applying_deltas();
        });
        assert_eq!(editor.take_request(), Some(FileRequest::ImportPackage));
    }
}
