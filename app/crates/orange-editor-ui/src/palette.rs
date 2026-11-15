//! Node catalogue projection and its presentation cache.
use orange_host::Host;

/// Own only the currently displayed result, not old searches or provider snapshots.
#[derive(Debug, Default)]
pub(crate) struct PaletteCache {
    generation: Option<u64>,
    search: String,
    groups: Vec<(String, Vec<PaletteItem>)>,
}

impl PaletteCache {
    pub fn get(&mut self, host: &Host, search: &str) -> &[(String, Vec<PaletteItem>)] {
        let generation = host.registry().generation();
        if self.generation != Some(generation) || self.search != search {
            self.groups = palette(host, search);
            self.search.clear();
            self.search.push_str(search);
            self.generation = Some(generation);
        }
        &self.groups
    }
}

/// One node kind a document can use, as the palette lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteItem {
    /// The definition, such as `org.orange.files/write-text`.
    pub id: String,
    pub label: String,
    /// The package that provides it; empty for the language's own nodes.
    pub package: String,
}

/// The host's nodes grouped by category, both in alphabetical order, keeping those
/// whose label, definition or category contains every word of `search` in any letter
/// case. An empty search keeps all.
pub fn palette(host: &Host, search: &str) -> Vec<(String, Vec<PaletteItem>)> {
    let search = search.to_lowercase();
    let words: Vec<&str> = search.split_whitespace().collect();
    let mut groups: std::collections::BTreeMap<String, Vec<PaletteItem>> = Default::default();
    for info in host.nodes() {
        let (label, category) = (info.label().into_owned(), info.category().into_owned());
        let id = info.definition.definition.id.as_str().to_owned();
        let text = format!("{label} {id} {category}").to_lowercase();
        if !words.iter().all(|word| text.contains(word)) {
            continue;
        }
        groups.entry(category).or_default().push(PaletteItem {
            id,
            label,
            package: info
                .provider
                .map(|provider| provider.package.clone())
                .unwrap_or_default(),
        });
    }
    groups
        .into_iter()
        .map(|(category, mut items)| {
            items.sort_by(|a, b| a.label.cmp(&b.label).then_with(|| a.id.cmp(&b.id)));
            (category, items)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use orange_host::HostOptions;

    #[test]
    fn cached_palette_tracks_search_and_registry_and_keeps_the_current_allocation() {
        let (host, _events) = Host::new(HostOptions::default()).unwrap();
        let mut cache = PaletteCache::default();
        assert_eq!(cache.get(&host, ""), palette(&host, ""));
        let allocation = cache.get(&host, "").as_ptr();
        assert_eq!(cache.get(&host, "").as_ptr(), allocation);
        for search in ["MATH add", "unmatched-xyz", ""] {
            assert_eq!(cache.get(&host, search), palette(&host, search));
        }
        // Registry generations are process-unique, including when one editor is
        // reused with another host. Old provider labels must not survive that switch.
        let (core, _events) = Host::new(HostOptions {
            base_nodes: false,
            ..Default::default()
        })
        .unwrap();
        assert_ne!(core.registry().generation(), host.registry().generation());
        assert_ne!(palette(&core, ""), palette(&host, ""));
        assert_eq!(cache.get(&core, ""), palette(&core, ""));
        assert_eq!(cache.get(&host, ""), palette(&host, ""));
    }
}
