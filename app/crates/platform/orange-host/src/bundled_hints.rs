//! Control hints for the host's bundled definitions, which have no definition file to
//! carry them (ADR-33). They go through the same reader and checks as a package's.

use orange_model::catalog::controls;
use orange_model::catalog::{Ports, Presentation, RegistrySnapshot};
use orange_model::document::DefinitionRef;
use serde_json::{Value as Json, json};

/// Hints by bundled definition ID: `(controls, layout)`.
fn hints(id: &str) -> Option<(Json, Json)> {
    Some(match id {
        // Map Range reads as "from this range to that one": each pair side by side.
        "org.orange.math/map-range" => (
            json!({"settings": {"clamp": {"widget": "toggle"}}}),
            json!({"regions": [
                {"region": "header", "fields": ["value"]},
                {"region": "body", "fields": ["from_min", "from_max"], "arrange": "row"},
                {"region": "body", "fields": ["to_min", "to_max"], "arrange": "row"}
            ]}),
        ),
        _ => return None,
    })
}

/// The presentations the bundled definitions in `registry` get from their hints.
pub(crate) fn presentations(registry: &RegistrySnapshot) -> Vec<(DefinitionRef, Presentation)> {
    registry
        .definitions()
        .filter_map(|definition| {
            let (controls, layout) = hints(definition.definition.id.as_str())?;
            let ports = match &definition.ports {
                Ports::Declared(ports) => ports.as_slice(),
                _ => &[],
            };
            let (controls, hint_problems) = controls::read(
                Some(&controls),
                Some(&layout),
                definition.config_schema.as_ref(),
                ports,
            );
            Some((
                definition.definition.clone(),
                Presentation {
                    controls,
                    hint_problems,
                    ..Presentation::default()
                },
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::{Host, HostOptions};

    #[test]
    fn map_range_pairs_its_ranges_and_every_bundled_hint_fits() {
        let (host, _events) = Host::new(HostOptions::default()).unwrap();
        let map = host
            .nodes()
            .find(|info| info.definition.definition.id.as_str() == "org.orange.math/map-range")
            .and_then(|info| info.presentation)
            .unwrap();
        assert_eq!(map.hint_problems, []);
        let (region, arrange) = map.controls.region_of("to_max").unwrap();
        assert_eq!(region, orange_model::catalog::controls::Region::Body);
        assert_eq!(arrange, orange_model::catalog::controls::Arrange::Row);
        assert_eq!(
            map.controls.region_of("value").map(|(region, _)| region),
            Some(orange_model::catalog::controls::Region::Header)
        );
    }
}
