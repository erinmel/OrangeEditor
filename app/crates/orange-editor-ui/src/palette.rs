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

/// How well a node matches a search, best first: its name exactly, its name's start,
/// a word of its name, one of its aliases, then anything else it is found by.
pub fn rank(label: &str, aliases: &[&str], search: &str) -> u8 {
    let (label, search) = (label.to_lowercase(), search.trim().to_lowercase());
    if label == search {
        0
    } else if label.starts_with(&search) {
        1
    } else if label
        .split_whitespace()
        .any(|word| word.starts_with(&search))
    {
        2
    } else if aliases.iter().any(|alias| alias.starts_with(&search)) {
        3
    } else {
        4
    }
}

/// The host's nodes grouped by category, keeping those whose name, aliases, definition
/// or category contain every word of `search` in any letter case. With a search, the
/// best match comes first in its category and the category of the best match first;
/// without one both are in alphabetical order.
pub fn palette(host: &Host, search: &str) -> Vec<(String, Vec<PaletteItem>)> {
    let lowered = search.to_lowercase();
    let words: Vec<&str> = lowered.split_whitespace().collect();
    let mut groups: std::collections::BTreeMap<String, Vec<(u8, PaletteItem)>> = Default::default();
    for info in host.nodes() {
        let (label, category) = (info.label().into_owned(), info.category().into_owned());
        let id = info.definition.definition.id.as_str().to_owned();
        let aliases = info.aliases();
        let text = format!("{label} {id} {category} {}", aliases.join(" ")).to_lowercase();
        if !words.iter().all(|word| text.contains(word)) {
            continue;
        }
        let score = if words.is_empty() {
            0
        } else {
            rank(&label, aliases, search)
        };
        groups.entry(category).or_default().push((
            score,
            PaletteItem {
                id,
                label,
                package: info
                    .provider
                    .map(|provider| provider.package.clone())
                    .unwrap_or_default(),
            },
        ));
    }
    let mut ranked: Vec<(u8, String, Vec<PaletteItem>)> = groups
        .into_iter()
        .map(|(category, mut items)| {
            items.sort_by(|(a_score, a), (b_score, b)| {
                a_score
                    .cmp(b_score)
                    .then_with(|| a.label.cmp(&b.label))
                    .then_with(|| a.id.cmp(&b.id))
            });
            let best = items.first().map_or(u8::MAX, |(score, _)| *score);
            (
                best,
                category,
                items.into_iter().map(|(_, item)| item).collect(),
            )
        })
        .collect();
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    ranked
        .into_iter()
        .map(|(_, category, items)| (category, items))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use orange_host::HostOptions;

    #[test]
    fn a_search_finds_aliases_and_puts_the_best_match_first() {
        let (host, _events) = Host::new(HostOptions::default()).unwrap();
        let first = |search: &str| palette(&host, search)[0].1[0].label.clone();
        assert_eq!(first("integer"), "Int");
        assert_eq!(first("boolean"), "Bool");
        assert_eq!(first("wait"), "Wait");
        assert!(palette(&host, "unmatched-xyz").is_empty());
    }

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
