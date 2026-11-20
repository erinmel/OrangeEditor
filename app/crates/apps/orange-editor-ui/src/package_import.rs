//! Install archives through the host while the editor and its flows stay open.
use super::*;
use crate::controls::Action;
use orange_host::Grants;

#[derive(Debug)]
pub(super) struct PackageImport {
    name: String,
    archive: Vec<u8>,
    requested: Grants,
    approved: Grants,
}

impl Editor {
    pub(super) fn package_removal_dialog(&mut self, ctx: &egui::Context, host: &mut Host) {
        let Some(key) = &self.pending_package_removal else {
            return;
        };
        let modal = egui::Modal::new(egui::Id::unique("orange-remove-package")).show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.heading(t("remove-package"));
            ui.label(tf(
                "remove-package-hint",
                &[("package", key.to_string().into())],
            ));
            controls::dialog_actions(
                ui,
                Action::Destructive,
                true,
                egui::Button::new((icons::text(icons::DELETE), t("remove-package"))),
            )
        });
        let (cancel, remove) = &modal.inner;
        if remove.clicked()
            && let Some(key) = self.pending_package_removal.take()
        {
            match host.uninstall(&key) {
                Ok(()) => {
                    self.previews.invalidate();
                    self.check(host);
                    self.say(tf(
                        "package-removed",
                        &[("package", key.to_string().into())],
                    ));
                }
                Err(error) => self.say_at(
                    NoteLevel::Error,
                    tf(
                        "package-remove-failed",
                        &[("error", error.to_string().into())],
                    ),
                ),
            }
            self.panels.log = true;
            self.log_events = true;
        } else if cancel.clicked() || modal.should_close() {
            self.pending_package_removal = None;
        }
    }

    /// Reviews the permissions an archive requests before installing it.
    pub fn review_package(
        &mut self,
        host: &Host,
        name: String,
        archive: Vec<u8>,
    ) -> Result<(), HostError> {
        let requested = host.permissions(&archive)?;
        self.pending_package = Some(PackageImport {
            name,
            archive,
            requested,
            approved: Grants::default(),
        });
        Ok(())
    }

    pub(super) fn package_import_dialog(&mut self, ctx: &egui::Context, host: &mut Host) {
        let Some(import) = &mut self.pending_package else {
            return;
        };
        let modal = egui::Modal::new(egui::Id::unique("orange-import-package")).show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.heading(t("import-package"));
            ui.label(&import.name);
            if import.requested != Grants::default() {
                ui.separator();
                ui.label(t("package-permissions"));
                for (requested, approved, label) in [
                    (
                        import.requested.log,
                        &mut import.approved.log,
                        "package-grant-log",
                    ),
                    (
                        import.requested.files,
                        &mut import.approved.files,
                        "package-grant-files",
                    ),
                    (
                        import.requested.http,
                        &mut import.approved.http,
                        "package-grant-http",
                    ),
                ] {
                    if requested {
                        ui.checkbox(approved, t(label));
                    }
                }
            } else {
                ui.label(t("package-no-permissions"));
            }
            controls::dialog_actions(
                ui,
                Action::Primary,
                true,
                egui::Button::new(t("install-package")),
            )
        });
        let (cancel, install) = &modal.inner;
        if install.clicked() {
            if let Some(import) = self.pending_package.take() {
                match host.install(&import.archive, import.approved) {
                    Ok(key) => self.say(tf(
                        "package-imported",
                        &[("name", key.id.to_string().into())],
                    )),
                    Err(error) => self.say_at(
                        NoteLevel::Error,
                        tf(
                            "package-import-failed",
                            &[("error", error.to_string().into())],
                        ),
                    ),
                }
                self.previews.invalidate();
                self.check(host);
                self.panels.log = true;
                self.log_events = true;
            }
        } else if cancel.clicked() || modal.should_close() {
            self.pending_package = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approving_an_archive_hot_installs_it_while_a_flow_runs() {
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
        let archive = orange_host::pack(manifest, &files).unwrap();
        let (mut host, _) = Host::new(orange_host::HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.open_untitled(&mut host, orange_host::new_document().unwrap());
        editor.add_node(&mut host, "core/on-manual").unwrap();
        editor.run(&mut host);
        let flow = editor.document.as_ref().unwrap().flow.as_ref().unwrap().0;
        editor
            .review_package(&host, "remember.orangeplug".to_owned(), archive)
            .unwrap();
        assert!(
            host.packages()
                .all(|package| package.id != "org.example.tools")
        );

        let ctx = egui::Context::default();
        let mut frame = |events| {
            ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    editor.package_import_dialog(ui.ctx(), &mut host);
                },
            )
        };
        let mut install = None;
        for _ in 0..3 {
            let output = frame(Vec::new());
            install = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == t("install-package") => {
                        Some(text.pos + text.galley.size() * 0.5)
                    }
                    _ => None,
                })
                .or(install);
            output.drop_without_applying_deltas();
        }
        let pos = install.expect("install button");
        frame(vec![egui::Event::PointerMoved(pos)]).drop_without_applying_deltas();
        for pressed in [true, false] {
            frame(vec![egui::Event::PointerButton {
                pos,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: egui::Modifiers::NONE,
            }])
            .drop_without_applying_deltas();
        }
        assert!(editor.pending_package.is_none());
        assert!(
            host.search_nodes("remember", "en")
                .iter()
                .any(|node| node.definition == "org.example.tools/remember")
        );
        assert!(host.info(flow).is_some());

        editor.pending_package_removal = host.packages().find_map(|package| {
            (package.id == "org.example.tools").then(|| package.manifest.unwrap().key().clone())
        });
        let mut frame = |events| {
            ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    editor.package_removal_dialog(ui.ctx(), &mut host);
                },
            )
        };
        let mut button = None;
        for _ in 0..3 {
            let output = frame(vec![]);
            button = output
                .shapes
                .iter()
                .rev()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == t("remove-package") => {
                        Some(text.pos + text.galley.size() * 0.5)
                    }
                    _ => None,
                })
                .or(button);
            output.drop_without_applying_deltas();
        }
        let pos = button.expect("remove package button");
        frame(vec![egui::Event::PointerMoved(pos)]).drop_without_applying_deltas();
        for pressed in [true, false] {
            frame(vec![egui::Event::PointerButton {
                pos,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: egui::Modifiers::NONE,
            }])
            .drop_without_applying_deltas();
        }
        assert!(editor.pending_package_removal.is_none());
        assert!(
            host.packages()
                .all(|package| package.id != "org.example.tools")
        );
        assert!(
            host.search_nodes("remember", "en")
                .iter()
                .all(|node| node.definition != "org.example.tools/remember")
        );
        assert!(host.info(flow).is_some());
        editor.stop(&mut host);
    }
}
