//! Install archives through the host while the editor and its flows stay open.
use super::*;
use crate::controls::Action;
use orange_host::Grants;
#[cfg(test)]
use orange_host::intrinsic;

#[derive(Debug)]
pub(super) struct PackageImport {
    name: String,
    archive: Vec<u8>,
    requested: Grants,
    approved: Grants,
}

impl Editor {
    fn draft_uses_package(&self, id: &str, version: Option<&str>) -> bool {
        self.document.as_ref().is_some_and(|document| {
            document
                .draft
                .document()
                .bodies()
                .flat_map(|body| body.nodes.values())
                .filter_map(|node| node.provider.as_ref())
                .any(|provider| {
                    provider.package == id && version.is_none_or(|v| provider.version == v)
                })
        })
    }

    pub(super) fn package_in_use(&self, host: &Host, id: &str, version: Option<&str>) -> bool {
        host.package_in_use(id, version) || self.draft_uses_package(id, version)
    }

    /// Turns package `id` (shown as `name`) off or on in the running host. New
    /// documents follow at once. The host refuses to turn off packages in use.
    pub(super) fn switch_package(&mut self, host: &mut Host, id: &str, name: &str, on: bool) {
        if !on && self.package_in_use(host, id, None) {
            self.say_at(NoteLevel::Error, t("package-in-use").to_owned());
            return;
        }
        match host.set_enabled(id, on) {
            Ok(()) => {
                self.previews.invalidate();
                self.check(host);
                self.say(tf(
                    if on {
                        "package-turned-on"
                    } else {
                        "package-turned-off"
                    },
                    &[("package", name.to_owned().into())],
                ));
            }
            Err(error) => self.say_at(
                NoteLevel::Error,
                tf(
                    "package-switch-failed",
                    &[("error", error.to_string().into())],
                ),
            ),
        }
    }

    pub(super) fn package_removal_dialog(&mut self, ctx: &egui::Context, host: &mut Host) {
        let Some(key) = &self.pending_package_removal else {
            return;
        };
        let in_use = self.package_in_use(host, key.id.as_str(), Some(key.version.as_str()));
        let modal = orange_ui_controls::modal(ctx, egui::Id::unique("orange-remove-package")).show(
            ctx,
            |ui| {
                ui.set_max_width(420.0);
                ui.heading(t("remove-package"));
                ui.label(tf(
                    "remove-package-hint",
                    &[("package", key.to_string().into())],
                ));
                if in_use {
                    ui.label(t("package-in-use"));
                }
                controls::dialog_actions(
                    ui,
                    Action::Destructive,
                    !in_use,
                    egui::Button::new((icons::text(icons::DELETE), t("remove-package"))),
                )
            },
        );
        let (cancel, remove) = &modal.inner;
        if !in_use
            && remove.clicked()
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
        let modal = orange_ui_controls::modal(ctx, egui::Id::unique("orange-import-package")).show(
            ctx,
            |ui| {
                ui.set_max_width(420.0);
                ui.heading(t("import-package"));
                ui.label(&import.name);
                grant_choices(ui, import.requested, &mut import.approved);
                controls::dialog_actions(
                    ui,
                    Action::Primary,
                    true,
                    egui::Button::new(t("install-package")),
                )
            },
        );
        let (cancel, install) = &modal.inner;
        if install.clicked() {
            if let Some(import) = self.pending_package.take() {
                self.install_approved_package(host, &import.archive, import.approved);
                self.previews.invalidate();
                self.check(host);
                self.panels.log = true;
                self.log_events = true;
            }
        } else if cancel.clicked() || modal.should_close() {
            self.pending_package = None;
        }
    }

    pub(super) fn install_approved_package(
        &mut self,
        host: &mut Host,
        archive: &[u8],
        grants: Grants,
    ) {
        match host.install(archive, grants) {
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
    }
}

/// The grants a package asks for, each a box the person ticks to allow it; nothing is
/// ticked for them. Says so when the package asks for nothing. Returns whether a box
/// changed.
pub(super) fn grant_choices(ui: &mut egui::Ui, requested: Grants, approved: &mut Grants) -> bool {
    if requested == Grants::default() {
        ui.label(t("package-no-permissions"));
        return false;
    }
    ui.separator();
    ui.label(t("package-permissions"));
    let mut choices = Vec::new();
    for (asked, given, label) in [
        (requested.log, &mut approved.log, "package-grant-log"),
        (requested.files, &mut approved.files, "package-grant-files"),
        (requested.http, &mut approved.http, "package-grant-http"),
        (
            requested.http_local,
            &mut approved.http_local,
            "package-grant-http-local",
        ),
        (
            requested.compute,
            &mut approved.compute,
            "package-grant-compute",
        ),
    ] {
        if asked {
            choices.push((given, t(label)));
        }
    }
    orange_ui_controls::check_list(
        ui,
        Surface::Dialog,
        orange_ui_controls::CheckStyle::Emphasis,
        &mut choices,
    )
    .changed()
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn package_manager_keeps_import_and_search_inline_and_filters_by_id() {
        let (mut host, _) = Host::new(orange_host::HostOptions::default()).unwrap();
        host.install(&remember_archive(), Grants::default())
            .unwrap();
        let mut editor = Editor::new();
        editor.file_dialogs = true;
        editor.package_search = "org.example.tools".to_owned();
        let ctx = egui::Context::default();
        icons::install(&ctx);
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(480.0, 400.0),
                )),
                ..Default::default()
            },
            |ui| editor.package_manager(ui, &mut host),
        );
        let labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some((text.galley.text().to_owned(), text.pos)),
                _ => None,
            })
            .collect();
        output.drop_without_applying_deltas();
        let import = labels
            .iter()
            .find(|(label, _)| label == t("import-package"))
            .unwrap()
            .1;
        let matches: Vec<_> = labels
            .iter()
            .filter(|(label, _)| label == "org.example.tools")
            .collect();
        assert_eq!(
            matches.len(),
            2,
            "the query and its matching package are both visible"
        );
        let search = matches[0].1;
        assert!(search.x > import.x && (search.y - import.y).abs() < 6.0);
        assert!(!labels.iter().any(|(label, _)| label == "org.orange.base"));
    }

    #[test]
    fn a_stopped_open_graph_protects_its_package_nodes() {
        let (mut host, _) = Host::new(orange_host::HostOptions::default()).unwrap();
        let key = host
            .install(&remember_archive(), Grants::default())
            .unwrap();
        let mut editor = Editor::new();
        editor.open_untitled(
            &mut host,
            include_str!("../../orange-cli/graphs/remember-on-trigger.json").to_owned(),
        );
        let id = key.id.as_str();
        let version = key.version.as_str();
        assert!(!host.package_in_use(id, None));
        assert!(editor.package_in_use(&host, id, Some(version)));
        editor.switch_package(&mut host, id, id, false);
        assert!(
            host.packages()
                .any(|package| package.id == id && package.enabled)
        );

        editor.open_untitled(&mut host, orange_host::new_document().unwrap());
        assert!(!editor.package_in_use(&host, id, Some(version)));
    }

    #[test]
    fn the_packages_panel_turns_an_installed_package_off_and_on_while_a_flow_runs() {
        let (mut host, _) = Host::new(orange_host::HostOptions::default()).unwrap();
        let _ = host
            .install(&remember_archive(), Grants::default())
            .unwrap();
        let mut editor = Editor::new();
        editor.open_untitled(&mut host, orange_host::new_document().unwrap());
        editor.add_node(&mut host, intrinsic::ON_MANUAL).unwrap();
        editor.run(&mut host);
        let flow = editor.document.as_ref().unwrap().flow.as_ref().unwrap().0;
        let placeable = |host: &Host| {
            host.search_nodes("remember", "en")
                .iter()
                .any(|node| node.definition == "org.example.tools/remember")
        };
        // Only the installed package has a switch: the base and clock nodes are core.
        let switches = |host: &Host| host.packages().filter(|p| p.switchable).count();
        assert_eq!(switches(&host), 1);

        let ctx = egui::Context::default();
        let using_package = host
            .open(include_str!(
                "../../orange-cli/graphs/remember-on-trigger.json"
            ))
            .unwrap();
        for (blocked, on) in [(true, true), (false, false), (false, true)] {
            let label = t(if blocked || !on {
                "package-enabled"
            } else {
                "package-disabled"
            });
            let mut frame = |events| {
                ctx.run_ui(
                    egui::RawInput {
                        events,
                        ..Default::default()
                    },
                    |ui| editor.package_manager(ui, &mut host),
                )
            };
            let mut toggle = None;
            let mut remove = None;
            for _ in 0..3 {
                let output = frame(Vec::new());
                let found: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == label => {
                            Some(text.pos + text.galley.size() * 0.5)
                        }
                        _ => None,
                    })
                    .collect();
                assert!(found.len() <= 1, "one switch is shown");
                toggle = found.first().copied().or(toggle);
                remove = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == t("remove-package") => {
                            Some(text.pos + text.galley.size() * 0.5)
                        }
                        _ => None,
                    })
                    .or(remove);
                output.drop_without_applying_deltas();
            }
            let pos = toggle.expect("package switch");
            let remove = remove.expect("package removal button");
            assert!(remove.x > pos.x && (remove.y - pos.y).abs() < 2.0);
            orange_testkit::ui::click(pos, egui::PointerButton::Primary, |events| {
                frame(events).drop_without_applying_deltas();
            });
            if blocked {
                orange_testkit::ui::click(remove, egui::PointerButton::Primary, |events| {
                    frame(events).drop_without_applying_deltas()
                });
                assert!(editor.pending_package_removal.is_none());
                host.close(using_package).unwrap();
            }
            let tools = host
                .packages()
                .find(|package| package.id == "org.example.tools")
                .unwrap();
            assert_eq!((tools.active, tools.enabled), (on, on));
            assert_eq!(placeable(&host), on);
            assert!(host.info(flow).is_some());
        }
        editor.stop(&mut host);
    }

    #[test]
    fn approving_an_archive_hot_installs_it_while_a_flow_runs() {
        let archive = remember_archive();
        let (mut host, _) = Host::new(orange_host::HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.open_untitled(&mut host, orange_host::new_document().unwrap());
        editor.add_node(&mut host, intrinsic::ON_MANUAL).unwrap();
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
        orange_testkit::ui::click(pos, egui::PointerButton::Primary, |events| {
            frame(events).drop_without_applying_deltas();
        });
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
        orange_testkit::ui::click(pos, egui::PointerButton::Primary, |events| {
            frame(events).drop_without_applying_deltas();
        });
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
