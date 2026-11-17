//! Inline device selection backed by adapter metadata and document connections.
use super::*;
use orange_host::{Connection, ConnectionScope, ReferenceKind};
#[derive(Debug, Clone)]
pub(crate) struct DevicePicker {
    pub adapter: String,
    pub pointer: String,
    pub connection: Option<Connection>,
    pub selected: Option<String>,
    pub options: Vec<(String, String)>,
    pub default_label: String,
    pub group_label: String,
}
impl Editor {
    pub(crate) fn prepare_devices(&mut self, host: &Host) {
        let Some(document) = &self.document else {
            return;
        };
        let nodes: Vec<_> = document
            .graph
            .nodes
            .iter()
            .filter_map(|node| {
                let pointer = node
                    .references
                    .iter()
                    .find(|(_, kind)| *kind == ReferenceKind::Connection)?
                    .0
                    .clone();
                Some((
                    node.id,
                    node.definition.clone(),
                    pointer.clone(),
                    node.settings
                        .pointer(&pointer)
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                ))
            })
            .collect();
        let declared = self
            .declarations()
            .map(|d| d.connections.clone())
            .unwrap_or_default();
        let adapters: std::collections::HashMap<_, _> = host
            .nodes()
            .filter_map(|node| {
                Some((
                    node.definition.definition.id.to_string(),
                    node.presentation?.connection_adapter.clone()?,
                ))
            })
            .collect();
        self.canvas.device_pickers.clear();
        for (id, definition, pointer, reference) in nodes {
            let Some(adapter) = adapters.get(&definition) else {
                continue;
            };
            let choices = self.adapter_choices(host, adapter, false);
            let Some(devices) = choices.iter().find(|choice| choice.setting == "device") else {
                continue;
            };
            let connection = declared
                .iter()
                .find(|c| c.id == reference && c.adapter == *adapter)
                .cloned();
            let selected = connection
                .as_ref()
                .and_then(|c| c.settings["device"].as_str().map(str::to_owned));
            let options = devices
                .values
                .iter()
                .enumerate()
                .map(|(at, value)| {
                    (
                        value.clone(),
                        devices
                            .labels
                            .get(at)
                            .cloned()
                            .unwrap_or_else(|| "Unavailable device".into()),
                    )
                })
                .collect();
            self.canvas.device_pickers.insert(
                id,
                DevicePicker {
                    adapter: adapter.clone(),
                    pointer,
                    connection,
                    selected,
                    options,
                    default_label: devices.unset.clone(),
                    group_label: if host
                        .connection_label(&definition)
                        .is_some_and(|label| label.to_lowercase().contains("output"))
                    {
                        "Playback devices".into()
                    } else {
                        "Recording devices".into()
                    },
                },
            );
        }
    }
    pub(crate) fn choose_device(&mut self, host: &mut Host, node: NodeId, device: Option<String>) {
        let Some(picker) = self.canvas.device_pickers.get(&node).cloned() else {
            return;
        };
        let name = device
            .as_ref()
            .and_then(|id| picker.options.iter().find(|(key, _)| key == id))
            .map_or_else(|| picker.default_label.clone(), |(_, label)| label.clone());
        let mut connection = picker.connection.unwrap_or(Connection {
            id: None,
            name,
            adapter: picker.adapter,
            scope: ConnectionScope::Graph,
            settings: serde_json::json!({}),
        });
        connection.settings = declarations::with_setting(&connection.settings, "device", device);
        declarations::set_text(&mut connection.settings, "file", None);
        if let Some(id) = self.declare_connection(host, &connection) {
            self.set_setting(
                host,
                node,
                &picker.pointer,
                Some(serde_json::Value::String(id)),
            );
        }
    }
}
