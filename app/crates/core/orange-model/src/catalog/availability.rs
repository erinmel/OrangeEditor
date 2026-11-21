//! Publication checks for execution-sourced data output availability (11 §4.1, §7.2).

use std::collections::BTreeSet;

use super::definition::{NodeDefinition, Ports};
use super::language::{implicit_ports, intrinsic};
use super::snapshot::RegistryError;
use crate::document::{OutputSource, PortDirection, PortKind};
use crate::id::PortId;

pub(super) fn check_availability(definition: &NodeDefinition) -> Result<(), RegistryError> {
    let Ports::Declared(ports) = &definition.ports else {
        // Contextual variable outputs have fixed host-defined availability contracts.
        return Ok(());
    };

    let id = &definition.definition.id;
    let has_declared_exec_output = ports
        .iter()
        .any(|port| port.kind == PortKind::Exec && port.direction == PortDirection::Output);
    let mut outcomes: BTreeSet<PortId> = ports
        .iter()
        .filter(|port| port.kind == PortKind::Exec && port.direction == PortDirection::Output)
        .map(|port| port.port_id.clone())
        .collect();
    for name in implicit_ports(definition.kind, has_declared_exec_output)
        .into_iter()
        .filter(|name| !matches!(*name, "in" | "error"))
    {
        if name == "then" && intrinsic(id.as_str()).is_some_and(|rule| rule.terminator) {
            continue;
        }
        outcomes.insert(crate::id::fixed_id(name));
    }

    for port in ports {
        let Some(available_on) = &port.available_on else {
            if port.kind == PortKind::Data
                && port.direction == PortDirection::Output
                && port.source == Some(OutputSource::Execution)
            {
                let default: PortId = crate::port_id!("then");
                if !outcomes.contains(&default) {
                    return Err(RegistryError::AvailableOnOutcome {
                        definition: id.clone(),
                        port: port.port_id.clone(),
                        outcome: default,
                    });
                }
            }
            continue;
        };
        if port.kind != PortKind::Data
            || port.direction != PortDirection::Output
            || port.source != Some(OutputSource::Execution)
        {
            return Err(RegistryError::AvailableOnPosition {
                definition: id.clone(),
                port: port.port_id.clone(),
            });
        }
        if available_on.is_empty() {
            return Err(RegistryError::AvailableOnEmpty {
                definition: id.clone(),
                port: port.port_id.clone(),
            });
        }
        let mut seen = BTreeSet::new();
        for outcome in available_on {
            if !seen.insert(outcome) {
                return Err(RegistryError::AvailableOnDuplicate {
                    definition: id.clone(),
                    port: port.port_id.clone(),
                    outcome: outcome.clone(),
                });
            }
            if !outcomes.contains(outcome) {
                return Err(RegistryError::AvailableOnOutcome {
                    definition: id.clone(),
                    port: port.port_id.clone(),
                    outcome: outcome.clone(),
                });
            }
        }
    }
    Ok(())
}
