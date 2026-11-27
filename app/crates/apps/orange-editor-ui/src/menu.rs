//! The right-click menu, the editor's command centre: what it offers depends on what
//! was clicked (the canvas, a card, a wire or a port) and each entry names its
//! shortcut. Commands go through the same editor methods as the toolbar and the keys,
//! so [`Editor::run_command`] drives them without a window.
use super::*;
use orange_host::intrinsic;

/// What a right-click was on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuTarget {
    /// The empty canvas.
    Canvas,
    /// A card. The commands act on the selection, which then includes it.
    Node(NodeId),
    /// A wire, by its two ends, output first.
    Wire(canvas::WireEnds),
    /// A port of a card.
    Port {
        node: NodeId,
        port: String,
        input: bool,
    },
}

/// A command of the right-click menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Command {
    AddNode,
    Properties,
    Rename,
    Paste,
    SelectAll,
    Group,
    Arrange,
    Fit,
    Copy,
    Cut,
    Duplicate,
    MakeFunction,
    ExportFunction,
    ExpandFunction,
    OpenFunction,
    Delete,
    InsertNode,
    AddWaypoint,
    AddSignalReceiver,
    AddSignalSender,
    RenameEvent,
    RemoveWaypoint,
    DeleteWire,
    Disconnect,
    WatchValue,
}

/// One entry of the menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuEntry {
    pub command: Command,
    pub label: &'static str,
    /// The keys that do the same, empty when none do.
    pub shortcut: &'static str,
    pub icon: char,
    pub enabled: bool,
}

/// The open menu: what was clicked and where, in graph units and on the screen.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Menu {
    target: MenuTarget,
    at: [f32; 2],
    screen: egui::Pos2,
    /// Drawn at least once: a click elsewhere after that closes it.
    shown: bool,
}

/// How far a duplicate lands from what it copies, in graph units.
const DUPLICATE_OFFSET: f32 = 40.0;
/// Room between a wireless node and the subscriber or emitter spawned beside it.
const SPAWN_GAP: f32 = 48.0;
/// The height kept free for a spawned card before it is moved below a card in the way.
const SPAWN_ROOM: f32 = 220.0;
/// Room between a spawned card and the card above it.
const STACK_GAP: f32 = 24.0;
/// The width of the right-click menu.
const MENU_WIDTH: f32 = 220.0;

impl Editor {
    /// Opens the right-click menu for `target`, clicked at `at` (graph units) and drawn
    /// at `screen`. A card not in the selection becomes the selection.
    pub fn open_menu(&mut self, target: MenuTarget, at: [f32; 2], screen: egui::Pos2) {
        if let MenuTarget::Node(node) = &target
            && !self.canvas.selection().contains(node)
        {
            self.canvas.select(Some(*node));
        }
        self.quick = None;
        self.menu = Some(Menu {
            target,
            at,
            screen,
            shown: false,
        });
    }

    /// What the open menu offers; empty when it is closed.
    pub fn menu_entries(&self) -> Vec<MenuEntry> {
        self.menu
            .as_ref()
            .map(|menu| self.entries_for(&menu.target))
            .unwrap_or_default()
    }

    /// Whether the right-click menu is open.
    pub fn is_menu_open(&self) -> bool {
        self.menu.is_some()
    }

    /// What the menu offers for `target`, in order.
    pub fn entries_for(&self, target: &MenuTarget) -> Vec<MenuEntry> {
        let chosen = self.canvas.selection().len();
        let open = self.document.is_some();
        let entry = |command, label, shortcut, icon, enabled| MenuEntry {
            command,
            label,
            shortcut,
            icon,
            enabled: open && enabled,
        };
        match target {
            MenuTarget::Canvas => vec![
                entry(
                    Command::AddNode,
                    t("add-node-ellipsis"),
                    "Tab",
                    icons::ADD,
                    true,
                ),
                entry(Command::Paste, t("paste"), "Ctrl+V", icons::PASTE, true),
                entry(
                    Command::SelectAll,
                    t("select-all"),
                    "Ctrl+A",
                    icons::SELECT_ALL,
                    true,
                ),
                entry(
                    Command::Group,
                    t("group-selection"),
                    "Ctrl+G",
                    icons::GROUP,
                    chosen > 0,
                ),
                entry(
                    Command::Arrange,
                    t("arrange-selection"),
                    "Ctrl+L",
                    icons::ARRANGE,
                    chosen > 1,
                ),
                entry(Command::Fit, t("fit-to-view"), "Home", icons::FIT, true),
            ],
            MenuTarget::Node(node) => {
                let mut entries = vec![
                    entry(
                        Command::Properties,
                        t("properties"),
                        "",
                        icons::PROPERTIES,
                        true,
                    ),
                    entry(Command::Rename, t("rename-node"), "", icons::EDIT, true),
                    entry(Command::Copy, t("copy"), "Ctrl+C", icons::COPY, true),
                    entry(Command::Cut, t("cut"), "Ctrl+X", icons::CUT, true),
                    entry(
                        Command::Duplicate,
                        t("duplicate"),
                        "Ctrl+D",
                        icons::DUPLICATE,
                        true,
                    ),
                    entry(Command::Group, t("group"), "Ctrl+G", icons::GROUP, true),
                    entry(
                        Command::MakeFunction,
                        t("make-function-ellipsis"),
                        "Ctrl+E",
                        icons::FUNCTION,
                        true,
                    ),
                ];
                if let Some(graph) = self.graph()
                    && let Some(card) = graph.nodes.iter().find(|card| card.id == *node)
                {
                    if matches!(
                        card.definition.as_str(),
                        intrinsic::EMIT | palette::BRIDGE_OUT
                    ) {
                        entries.insert(
                            1,
                            entry(
                                Command::AddSignalReceiver,
                                t("signal-add-receiver"),
                                "",
                                icons::SIGNAL,
                                true,
                            ),
                        );
                    }
                    if card.definition == intrinsic::ON_EVENT {
                        let has_emitter = card.settings["event"].as_str().is_some_and(|event| {
                            graph
                                .nodes
                                .iter()
                                .any(|node| palette::signal(node) == Some((true, event)))
                        });
                        entries.insert(
                            1,
                            entry(
                                Command::AddSignalSender,
                                t(if has_emitter {
                                    "signal-add-another-sender"
                                } else {
                                    "signal-add-sender"
                                }),
                                "",
                                icons::SIGNAL,
                                true,
                            ),
                        );
                    }
                    if let Some((_, id)) = palette::signal(card)
                        && self.declarations().is_some_and(|d| {
                            d.events.iter().any(|event| event.id.to_string() == id)
                        })
                    {
                        entries.insert(
                            1,
                            entry(
                                Command::RenameEvent,
                                t("event-rename"),
                                "",
                                icons::EDIT,
                                true,
                            ),
                        );
                    }
                }
                if self.call_function(*node).is_some() {
                    entries.push(entry(
                        Command::OpenFunction,
                        t("open-function"),
                        "",
                        icons::FUNCTION,
                        true,
                    ));
                    entries.push(entry(
                        Command::ExpandFunction,
                        t("expand-function"),
                        "",
                        icons::FUNCTION,
                        true,
                    ));
                    entries.push(entry(
                        Command::ExportFunction,
                        t("export-function"),
                        "",
                        icons::PACKAGES,
                        true,
                    ));
                }
                entries.extend([
                    entry(
                        Command::Arrange,
                        t("arrange"),
                        "Ctrl+L",
                        icons::ARRANGE,
                        chosen > 1,
                    ),
                    entry(Command::Delete, t("delete"), "Del", icons::DELETE, true),
                ]);
                entries
            }
            MenuTarget::Wire(wire) => vec![
                entry(Command::InsertNode, t("insert-node"), "", icons::ADD, true),
                entry(
                    Command::AddWaypoint,
                    t("wire-reroute"),
                    "",
                    icons::ARRANGE,
                    true,
                ),
                entry(
                    Command::RemoveWaypoint,
                    t("remove-reroute"),
                    "",
                    icons::DISCONNECT,
                    self.canvas.waypoints.contains_key(wire),
                ),
                entry(
                    Command::DeleteWire,
                    t("delete-wire"),
                    "",
                    icons::DELETE,
                    true,
                ),
            ],
            MenuTarget::Port { node, port, input } => {
                let connected = self
                    .port_view(*node, port, *input)
                    .is_some_and(|port| port.connected);
                vec![
                    entry(
                        Command::Disconnect,
                        t("disconnect"),
                        "",
                        icons::DISCONNECT,
                        connected,
                    ),
                    entry(
                        Command::WatchValue,
                        t("watch-value"),
                        "",
                        icons::WATCH,
                        true,
                    ),
                ]
            }
        }
    }

    pub(crate) fn set_waypoint(
        &mut self,
        host: &mut Host,
        wire: &canvas::WireEnds,
        at: Option<[f32; 2]>,
    ) -> bool {
        let Some(draft) = self.editing() else {
            return false;
        };
        let ((source, out), (target, input)) = wire;
        let result = draft.set_wire_waypoint((*source, out), (*target, input), at);
        self.edited(host, result)
    }

    /// Runs `command` of the open menu on what was right-clicked, and closes the menu.
    /// Returns whether it did something; a command the menu does not offer, or offers
    /// greyed out, does nothing.
    pub fn run_command(&mut self, host: &mut Host, command: Command) -> bool {
        let Some(menu) = self.menu.take() else {
            return false;
        };
        let offered = self
            .entries_for(&menu.target)
            .iter()
            .any(|entry| entry.command == command && entry.enabled);
        if !offered {
            return false;
        }
        match (command, &menu.target) {
            (Command::AddNode, _) => {
                self.open_quick_add(None, menu.at, menu.screen);
                true
            }
            (Command::Properties, MenuTarget::Node(node)) => {
                self.canvas.select(Some(*node));
                self.panels.properties = true;
                true
            }
            (Command::Rename, MenuTarget::Node(node)) => self.begin_node_rename(*node),
            (Command::RenameEvent, MenuTarget::Node(node)) => self.rename_event_for_node(*node),
            (Command::Paste, _) => match self.clip.clone() {
                Some(clip) => !self.paste(host, &clip, menu.at).is_empty(),
                None => {
                    // The system clipboard answers on a later frame, as a paste event.
                    self.paste_asked = true;
                    true
                }
            },
            (Command::SelectAll, _) => {
                let every: Vec<NodeId> = self
                    .graph()
                    .map(|graph| graph.nodes.iter().map(|node| node.id).collect())
                    .unwrap_or_default();
                self.canvas.select_many(&every);
                true
            }
            (Command::Group, _) => self.group_selection(host, t("group")).is_some(),
            (Command::Arrange, _) => self.arrange_selection(),
            (Command::Fit, _) => {
                self.canvas.fit();
                true
            }
            (Command::Copy, _) => self.copy_to_clip(host, false),
            (Command::Cut, _) => self.copy_to_clip(host, true),
            (Command::Duplicate, _) => self.duplicate(host),
            (Command::MakeFunction, _) => {
                self.preview_function(host);
                true
            }
            (Command::ExpandFunction, MenuTarget::Node(node)) => self.expand_function(host, *node),
            (Command::OpenFunction, MenuTarget::Node(node)) => self.open_function(host, *node),
            (Command::ExportFunction, MenuTarget::Node(node)) => {
                self.begin_function_export(host, *node)
            }
            (Command::Delete, _) => {
                let selection = self.canvas.selection();
                self.remove_nodes(host, &selection)
            }
            (Command::InsertNode, MenuTarget::Wire(wire)) => {
                self.open_insert_node(wire.clone(), menu.at, menu.screen);
                true
            }
            (Command::AddSignalReceiver | Command::AddSignalSender, MenuTarget::Node(node)) => {
                self.add_signal_endpoint(host, *node, command == Command::AddSignalSender)
            }
            (Command::AddWaypoint, MenuTarget::Wire(wire)) => {
                self.set_waypoint(host, wire, Some(menu.at))
            }
            (Command::RemoveWaypoint, MenuTarget::Wire(wire)) => {
                self.set_waypoint(host, wire, None)
            }
            (Command::DeleteWire, MenuTarget::Wire(wire)) => self.remove_wire(host, wire),
            (Command::Disconnect, MenuTarget::Port { node, port, input }) => {
                self.disconnect(host, (*node, port), *input)
            }
            (Command::WatchValue, MenuTarget::Port { node, .. }) => {
                // The properties panel shows the values the node's ports had last.
                self.canvas.select(Some(*node));
                self.panels.properties = true;
                true
            }
            _ => false,
        }
    }

    pub(crate) fn add_signal_endpoint(
        &mut self,
        host: &mut Host,
        node: NodeId,
        sender: bool,
    ) -> bool {
        let Some(graph) = self.graph() else {
            return false;
        };
        let places: std::collections::BTreeMap<_, _> = self.canvas.places().into_iter().collect();
        let Some(&[x, mut y]) = places.get(&node) else {
            return false;
        };
        let side = canvas::NODE_WIDTH + SPAWN_GAP;
        let x = if sender { x - side } else { x + side };
        let mut occupied: Vec<_> = graph
            .nodes
            .iter()
            .filter_map(|card| {
                let [left, top] = places.get(&card.id)?;
                let size = canvas::node_size(card);
                (x < left + size.x && *left < x + canvas::NODE_WIDTH)
                    .then_some((*top, top + size.y))
            })
            .collect();
        occupied.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (top, bottom) in occupied {
            if y + SPAWN_ROOM > top && y < bottom + STACK_GAP {
                y = bottom + STACK_GAP;
            }
        }
        let bridge = graph
            .nodes
            .iter()
            .find(|card| card.id == node)
            .and_then(|card| {
                palette::bridge_channel(card)
                    .map(|channel| (card.definition.clone(), channel.to_owned()))
            });
        let Some(draft) = self.editing() else {
            return false;
        };
        let result = match bridge {
            // A Bridge Out spawns a Bridge In on its channel; an unbound Bridge In gets a
            // Bridge Out on its own.
            Some((definition, channel)) => {
                let spawn = if definition == palette::BRIDGE_OUT {
                    palette::BRIDGE_IN
                } else {
                    palette::BRIDGE_OUT
                };
                // One undo step; a refused bind leaves no node, edge or channel behind.
                host.grouped(draft, |host, draft| {
                    let spawned = host.add_node(draft, spawn, [x, y])?;
                    let _ = host.set_config(
                        draft,
                        spawned,
                        &serde_json::json!({ "channel": channel }),
                    )?;
                    let (out, reading) = if spawn == palette::BRIDGE_IN {
                        (node, spawned)
                    } else {
                        (spawned, node)
                    };
                    let _ = host.connect(draft, (out, "result"), (reading, "value"))?;
                    Ok(spawned)
                })
            }
            None if sender => host.add_signal_sender(draft, node, [x, y]),
            None => host.add_signal_receiver(draft, node, [x, y]),
        };
        match result {
            Ok(endpoint) => {
                let _ = self.edited(host, Ok(true));
                self.canvas.select(Some(endpoint));
                // It rides on the pointer until a click places it.
                self.canvas.follow(endpoint);
                true
            }
            Err(error) => self.edited(host, Err(error)),
        }
    }

    /// Copies the selection into the editor's clip and the system clipboard; with
    /// `cut`, also removes it.
    pub(crate) fn copy_to_clip(&mut self, host: &mut Host, cut: bool) -> bool {
        let Some(clip) = self.copy(host) else {
            return false;
        };
        self.clip = Some(clip.clone());
        self.clip_out = Some(clip);
        if cut {
            let selection = self.canvas.selection();
            let _ = self.remove_nodes(host, &selection);
        }
        true
    }

    /// Copies the selection and pastes it a little below and to the right, selecting
    /// the copies. The clipboard is left as it was.
    pub fn duplicate(&mut self, host: &mut Host) -> bool {
        let selection = self.canvas.selection();
        let Some(clip) = self.copy(host) else {
            return false;
        };
        let corner = self
            .graph()
            .map(|graph| {
                graph
                    .nodes
                    .iter()
                    .filter(|node| selection.contains(&node.id))
                    .filter_map(|node| node.position)
                    .fold([f32::INFINITY; 2], |least, at| {
                        [least[0].min(at[0]), least[1].min(at[1])]
                    })
            })
            .filter(|corner| corner[0].is_finite())
            .unwrap_or_else(|| self.canvas.paste_place());
        let at = [corner[0] + DUPLICATE_OFFSET, corner[1] + DUPLICATE_OFFSET];
        !self.paste(host, &clip, at).is_empty()
    }

    /// Removes one wire, leaving any other at its two ports.
    pub fn remove_wire(&mut self, host: &mut Host, wire: &canvas::WireEnds) -> bool {
        let Some(draft) = self.editing() else {
            return false;
        };
        let result = host.disconnect_wire(draft, (wire.0.0, &wire.0.1), (wire.1.0, &wire.1.1));
        self.edited(host, result)
    }

    /// Draws the open menu where it was opened. A click on an entry runs it; a click
    /// elsewhere or Escape closes it.
    pub(crate) fn menu_popup(&mut self, ctx: &egui::Context, host: &mut Host) {
        let Some(menu) = &self.menu else {
            return;
        };
        let entries = self.entries_for(&menu.target);
        let title = match menu.target {
            MenuTarget::Canvas => t("canvas"),
            MenuTarget::Node(_) => t("node"),
            MenuTarget::Wire(_) => t("connection"),
            MenuTarget::Port { .. } => t("port"),
        };
        let (screen, shown_before) = (menu.screen, menu.shown);
        let mut chosen = None;
        let shown = orange_ui_controls::anchored_popup(
            ctx,
            egui::Id::unique("orange-context-menu"),
            screen,
            MENU_WIDTH,
            |ui| {
                ui.strong(title);
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height((ctx.content_rect().height() - 64.0).max(28.0))
                    .show(ui, |ui| {
                        let mut previous_group = None;
                        for entry in &entries {
                            let group = match entry.command {
                                Command::Group
                                | Command::MakeFunction
                                | Command::OpenFunction
                                | Command::ExpandFunction
                                | Command::ExportFunction
                                | Command::Arrange => 1,
                                Command::Fit | Command::SelectAll => 2,
                                Command::Delete | Command::DeleteWire | Command::Disconnect => 3,
                                _ => 0,
                            };
                            if previous_group.is_some_and(|previous| previous != group) {
                                ui.separator();
                            }
                            previous_group = Some(group);
                            let action = match entry.command {
                                Command::AddNode => Some("add"),
                                Command::Paste => Some("paste"),
                                Command::SelectAll => Some("select-all"),
                                Command::Group => Some("group"),
                                Command::Arrange => Some("arrange"),
                                Command::Fit => Some("fit"),
                                Command::Copy => Some("copy"),
                                Command::Cut => Some("cut"),
                                Command::Duplicate => Some("duplicate"),
                                Command::MakeFunction => Some("function"),
                                Command::Delete | Command::DeleteWire => Some("delete"),
                                _ => None,
                            };
                            let shortcut = action.map_or_else(
                                || entry.shortcut.to_owned(),
                                |action| self.canvas.keys.label(action),
                            );
                            let shortcut = if shortcut == t("unassigned") {
                                ""
                            } else {
                                &shortcut
                            };
                            if orange_ui_controls::menu_item(
                                ui,
                                icons::centered(entry.icon),
                                entry.label,
                                shortcut,
                                entry.enabled,
                            )
                            .clicked()
                            {
                                chosen = Some(entry.command);
                            }
                        }
                    });
            },
        );
        let escape = ctx.input(|input| input.key_pressed(egui::Key::Escape));
        if let Some(command) = chosen {
            let _ = self.run_command(host, command);
        } else if escape || (shown_before && shown.response.clicked_elsewhere()) {
            self.menu = None;
        } else if let Some(menu) = &mut self.menu {
            menu.shown = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::graph;
    use orange_host::HostOptions;

    #[test]
    fn keyboard_and_context_delete_only_the_selected_signal_endpoint() {
        let (mut host, _) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.open_untitled(&mut host, orange_host::new_document().unwrap());
        let sender = editor
            .add_node_at(&mut host, orange_host::preset::SIGNAL, [0.0, 0.0])
            .unwrap();
        let first = editor
            .graph()
            .unwrap()
            .nodes
            .iter()
            .find(|node| node.id != sender)
            .unwrap()
            .id;
        assert!(editor.add_signal_endpoint(&mut host, sender, false));
        let second = editor.canvas.selected().unwrap();

        editor.open_menu(MenuTarget::Node(first), [0.0, 0.0], egui::Pos2::ZERO);
        assert!(editor.run_command(&mut host, Command::AddSignalSender));
        let other_sender = editor.canvas.selected().unwrap();
        assert_eq!(editor.graph().unwrap().nodes.len(), 4);
        let wait_a = editor
            .add_node_at(&mut host, intrinsic::WAIT, [740.0, 0.0])
            .unwrap();
        let wait_b = editor
            .add_node_at(&mut host, intrinsic::WAIT, [740.0, 200.0])
            .unwrap();
        assert!(editor.connect(&mut host, (first, "then"), (wait_a, "in")));
        assert!(editor.connect(&mut host, (second, "then"), (wait_b, "in")));

        editor.open_menu(MenuTarget::Node(first), [0.0, 0.0], egui::Pos2::ZERO);
        assert!(editor.run_command(&mut host, Command::Delete));
        assert!(
            editor
                .graph()
                .unwrap()
                .nodes
                .iter()
                .all(|node| node.id != first)
        );
        assert_eq!(editor.graph().unwrap().nodes.len(), 5);
        assert_eq!(editor.graph().unwrap().wires.len(), 1);
        assert_eq!(editor.graph().unwrap().wires[0].from.0, second);

        editor.canvas.select(Some(sender));
        let ctx = egui::Context::default();
        for events in [
            vec![],
            vec![egui::Event::Key {
                key: egui::Key::Delete,
                physical_key: None,
                modifiers: egui::Modifiers::NONE,
                pressed: true,
                repeat: false,
            }],
        ] {
            ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    editor.ui(ui, &mut host);
                },
            )
            .drop_without_applying_deltas();
        }
        let graph = editor.graph().unwrap();
        assert_eq!(graph.nodes.len(), 4);
        assert_eq!(graph.wires.len(), 1);
        assert!(graph.nodes.iter().any(|node| node.id == second));
        assert!(graph.nodes.iter().any(|node| node.id == other_sender));
        assert_eq!(
            editor
                .document
                .as_ref()
                .unwrap()
                .draft
                .document()
                .events()
                .len(),
            1
        );
        editor.undo(&mut host);
        assert!(
            editor
                .graph()
                .unwrap()
                .nodes
                .iter()
                .any(|node| node.id == sender)
        );
    }

    fn id(n: u32) -> NodeId {
        format!("10000000-0000-4000-8000-{n:012x}").parse().unwrap()
    }

    fn opened() -> (Editor, Host) {
        let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        let text = host.bind(&graph()).unwrap();
        editor.open(&mut host, "menu.json", text);
        (editor, host)
    }

    fn commands(editor: &Editor) -> Vec<Command> {
        editor
            .menu_entries()
            .iter()
            .map(|entry| entry.command)
            .collect()
    }

    #[test]
    fn a_named_card_can_be_renamed_undone_and_reset() {
        let (mut editor, mut host) = opened();
        let node = id(2);
        let original = editor
            .graph()
            .unwrap()
            .nodes
            .iter()
            .find(|view| view.id == node)
            .unwrap()
            .name
            .clone();
        let wires = editor.graph().unwrap().wires.clone();
        editor.open_menu(MenuTarget::Node(node), [0.0; 2], egui::Pos2::ZERO);
        assert!(editor.run_command(&mut host, Command::Rename));
        assert_eq!(editor.renaming_node.as_ref().unwrap().node, node);
        assert!(editor.rename_node(&mut host, node, "  New label  "));
        assert!(editor.text().unwrap().contains("New label"));
        assert_eq!(editor.graph().unwrap().wires, wires);
        editor.undo(&mut host);
        assert_eq!(
            editor
                .graph()
                .unwrap()
                .nodes
                .iter()
                .find(|view| view.id == node)
                .unwrap()
                .name,
            original
        );
        editor.redo(&mut host);
        assert!(editor.rename_node(&mut host, node, "  "));
        assert!(!editor.text().unwrap().contains("New label"));
    }

    #[test]
    fn each_target_offers_its_commands_and_canvas_and_card_entries_name_shortcuts() {
        let (mut editor, mut host) = opened();
        editor.open_menu(MenuTarget::Canvas, [0.0, 0.0], egui::Pos2::ZERO);
        assert_eq!(
            commands(&editor),
            [
                Command::AddNode,
                Command::Paste,
                Command::SelectAll,
                Command::Group,
                Command::Arrange,
                Command::Fit
            ]
        );
        // With nothing selected, grouping is offered greyed out.
        let entries = editor.menu_entries();
        let group = entries.iter().find(|entry| entry.command == Command::Group);
        assert!(!group.unwrap().enabled);
        assert!(entries.iter().all(|entry| !entry.shortcut.is_empty()));
        // A right-clicked card becomes the selection and gets the card commands.
        editor.open_menu(MenuTarget::Node(id(2)), [0.0, 0.0], egui::Pos2::ZERO);
        assert_eq!(editor.canvas.selection(), [id(2)]);
        assert!(commands(&editor).starts_with(&[
            Command::Properties,
            Command::Rename,
            Command::Copy,
            Command::Cut,
            Command::Duplicate
        ]));
        assert!(commands(&editor).contains(&Command::MakeFunction));
        assert!(editor.menu_entries().iter().all(|entry| matches!(
            entry.command,
            Command::Properties | Command::Rename
        ) || !entry.shortcut.is_empty()));
        editor.panels.properties = false;
        assert!(editor.run_command(&mut host, Command::Properties));
        assert!(editor.panels.properties);
        assert_eq!(editor.canvas.selected(), Some(id(2)));
        let wire = ((id(1), "then".to_owned()), (id(2), "in".to_owned()));
        editor.open_menu(MenuTarget::Wire(wire), [0.0, 0.0], egui::Pos2::ZERO);
        assert_eq!(
            commands(&editor),
            [
                Command::InsertNode,
                Command::AddWaypoint,
                Command::RemoveWaypoint,
                Command::DeleteWire
            ]
        );
        // An unwired port cannot be disconnected but can be watched.
        let port = MenuTarget::Port {
            node: id(2),
            port: "value".into(),
            input: true,
        };
        let entries = editor.entries_for(&port);
        assert!(!entries[0].enabled && entries[1].enabled);
    }

    #[test]
    fn rerouting_is_a_layout_edit_with_undo() {
        let (mut editor, mut host) = opened();
        let wire = ((id(1), "then".to_owned()), (id(2), "in".to_owned()));
        editor.open_menu(
            MenuTarget::Wire(wire.clone()),
            [50.0, 80.0],
            egui::Pos2::ZERO,
        );
        assert!(editor.run_command(&mut host, Command::AddWaypoint));
        assert_eq!(editor.canvas.waypoints.get(&wire), Some(&[50.0, 80.0]));
        editor.undo(&mut host);
        assert!(!editor.canvas.waypoints.contains_key(&wire));
    }

    #[test]
    fn card_commands_duplicate_group_and_delete_the_selection_as_single_edits() {
        let (mut editor, mut host) = opened();
        let count = |editor: &Editor| editor.graph().unwrap().nodes.len();
        editor.open_menu(MenuTarget::Node(id(2)), [0.0, 0.0], egui::Pos2::ZERO);
        assert!(editor.run_command(&mut host, Command::Duplicate));
        assert!(!editor.is_menu_open());
        assert_eq!(count(&editor), 3);
        let copy = editor.canvas.selection();
        assert_eq!(copy.len(), 1);
        assert_ne!(copy[0], id(2));
        editor.open_menu(MenuTarget::Node(copy[0]), [0.0, 0.0], egui::Pos2::ZERO);
        assert!(editor.run_command(&mut host, Command::Group));
        assert_eq!(editor.groups().len(), 1);
        editor.open_menu(MenuTarget::Node(copy[0]), [0.0, 0.0], egui::Pos2::ZERO);
        assert!(editor.run_command(&mut host, Command::Delete));
        assert_eq!(count(&editor), 2);
        editor.undo(&mut host);
        assert_eq!(count(&editor), 3);
        // Copy, then Paste on the canvas, where the menu was opened.
        editor.open_menu(MenuTarget::Node(id(2)), [0.0, 0.0], egui::Pos2::ZERO);
        assert!(editor.run_command(&mut host, Command::Copy));
        editor.open_menu(MenuTarget::Canvas, [500.0, 500.0], egui::Pos2::ZERO);
        assert!(editor.run_command(&mut host, Command::Paste));
        assert_eq!(count(&editor), 4);
        let pasted = editor.canvas.selection()[0];
        let place = editor
            .graph()
            .unwrap()
            .nodes
            .iter()
            .find(|node| node.id == pasted)
            .unwrap()
            .position;
        assert_eq!(place, Some([500.0, 500.0]));
        // A command the open menu does not offer does nothing, and closes it.
        editor.open_menu(MenuTarget::Canvas, [0.0, 0.0], egui::Pos2::ZERO);
        assert!(!editor.run_command(&mut host, Command::DeleteWire));
        assert!(
            !editor.run_command(&mut host, Command::Fit),
            "the menu closed"
        );
    }

    #[test]
    fn a_node_is_inserted_into_a_wire_and_a_wire_or_port_is_taken_off() {
        let (mut editor, mut host) = opened();
        let wire = ((id(1), "then".to_owned()), (id(2), "in".to_owned()));
        editor.open_menu(
            MenuTarget::Wire(wire.clone()),
            [300.0, 0.0],
            egui::Pos2::ZERO,
        );
        assert!(editor.run_command(&mut host, Command::InsertNode));
        assert!(editor.is_quick_add_open());
        let choice = editor
            .quick_choices(&host, "do once")
            .into_iter()
            .find(|choice| choice.definition == intrinsic::DO_ONCE)
            .unwrap();
        let inserted = editor.pick_quick(&mut host, &choice).unwrap();
        let wires = |editor: &Editor| -> Vec<canvas::WireEnds> {
            editor
                .graph()
                .unwrap()
                .wires
                .iter()
                .map(|wire| (wire.from.clone(), wire.to.clone()))
                .collect()
        };
        let now = wires(&editor);
        assert!(!now.contains(&wire), "{now:?}");
        assert!(
            now.iter()
                .any(|(from, to)| *from == wire.0 && to.0 == inserted)
        );
        assert!(
            now.iter()
                .any(|(from, to)| from.0 == inserted && *to == wire.1)
        );
        // Delete wire takes off that one wire.
        let onward = now
            .iter()
            .find(|(from, _)| from.0 == inserted)
            .unwrap()
            .clone();
        editor.open_menu(MenuTarget::Wire(onward), [0.0, 0.0], egui::Pos2::ZERO);
        assert!(editor.run_command(&mut host, Command::DeleteWire));
        assert_eq!(wires(&editor).len(), 1);
        // Disconnect takes off the wires at a port; Watch value shows its node.
        let port = MenuTarget::Port {
            node: id(1),
            port: "then".into(),
            input: false,
        };
        editor.open_menu(port.clone(), [0.0, 0.0], egui::Pos2::ZERO);
        assert!(editor.run_command(&mut host, Command::Disconnect));
        assert!(wires(&editor).is_empty());
        editor.panels.properties = false;
        editor.open_menu(port, [0.0, 0.0], egui::Pos2::ZERO);
        assert!(editor.run_command(&mut host, Command::WatchValue));
        assert!(editor.panels.properties);
        assert_eq!(editor.canvas.selected(), Some(id(1)));
    }

    /// Where `text` is drawn, the leftmost.
    fn drawn_at(shapes: &[egui::epaint::ClippedShape], text: &str) -> Option<egui::Pos2> {
        let mut found: Vec<egui::Pos2> = Vec::new();
        for clipped in shapes {
            let mut stack = vec![&clipped.shape];
            while let Some(shape) = stack.pop() {
                match shape {
                    egui::Shape::Text(drawn) if drawn.galley.text() == text => {
                        found.push(drawn.visual_bounding_rect().center());
                    }
                    egui::Shape::Vec(inner) => stack.extend(inner.iter()),
                    _ => {}
                }
            }
        }
        found.into_iter().min_by(|a, b| a.x.total_cmp(&b.x))
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn button(at: egui::Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: at,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn a_right_click_opens_the_menu_of_what_is_under_it_and_the_search_takes_arrow_keys() {
        let (mut editor, mut host) = opened();
        editor.panels.nodes = false;
        editor.panels.log = false;
        editor.panels.properties = false;
        let context = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
        let frame = |editor: &mut Editor, host: &mut Host, events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            };
            let output = context.run_ui(input, |ui| editor.ui(ui, host));
            let shapes = output.shapes.clone();
            output.drop_without_applying_deltas();
            shapes
        };
        let secondary = egui::PointerButton::Secondary;
        let right_click = |at| {
            [
                vec![egui::Event::PointerMoved(at)],
                vec![button(at, secondary, true)],
                vec![button(at, secondary, false)],
            ]
        };
        let _ = frame(&mut editor, &mut host, Vec::new());
        let shapes = frame(&mut editor, &mut host, Vec::new());
        let card = drawn_at(&shapes, "go").expect("the trigger card shows its name");
        for events in right_click(card) {
            let _ = frame(&mut editor, &mut host, events);
        }
        let target = |editor: &Editor| editor.menu.as_ref().map(|menu| menu.target.clone());
        assert_eq!(target(&editor), Some(MenuTarget::Node(id(1))));
        let shapes = frame(&mut editor, &mut host, Vec::new());
        assert!(drawn_at(&shapes, "Duplicate").is_some());
        let duplicate = editor.canvas.keys.label("duplicate");
        assert!(
            drawn_at(&shapes, &duplicate).is_some(),
            "entries name their shortcut, {duplicate} here"
        );
        // Escape closes it; a right-click on empty canvas opens the canvas menu.
        let _ = frame(&mut editor, &mut host, vec![key(egui::Key::Escape)]);
        assert!(!editor.is_menu_open());
        for events in right_click(egui::pos2(1200.0, 680.0)) {
            let _ = frame(&mut editor, &mut host, events);
        }
        assert_eq!(target(&editor), Some(MenuTarget::Canvas));
        let shapes = frame(&mut editor, &mut host, Vec::new());
        let add = drawn_at(&shapes, "Add node…").expect("the canvas menu shows Add node…");
        let primary = egui::PointerButton::Primary;
        let _ = frame(&mut editor, &mut host, vec![egui::Event::PointerMoved(add)]);
        let _ = frame(
            &mut editor,
            &mut host,
            vec![button(add, primary, true), button(add, primary, false)],
        );
        assert!(editor.is_quick_add_open(), "Add node… opens the search");
        // In the search, the arrows move and Enter adds the one they are on.
        let _ = frame(&mut editor, &mut host, Vec::new());
        let _ = frame(
            &mut editor,
            &mut host,
            vec![egui::Event::Text("trigger".into())],
        );
        let second = editor.quick_choices(&host, "trigger")[1].definition.clone();
        let before: Vec<NodeId> = editor.graph().unwrap().nodes.iter().map(|n| n.id).collect();
        let _ = frame(&mut editor, &mut host, vec![key(egui::Key::ArrowDown)]);
        let _ = frame(&mut editor, &mut host, vec![key(egui::Key::Enter)]);
        assert!(!editor.is_quick_add_open());
        let graph = editor.graph().unwrap();
        let added = graph.nodes.iter().find(|node| !before.contains(&node.id));
        assert_eq!(
            added.map(|node| node.definition.as_str()),
            Some(second.as_str())
        );
    }
}
