//! The list filter understands `*` and `?` within a path component and `**` as
//! an entire component. A single-component filter applies to each entry's name.

pub(super) struct Glob<'a>(Vec<&'a str>);

impl<'a> Glob<'a> {
    pub(super) fn parse(pattern: &'a str) -> Option<Self> {
        if pattern.len() > 128 || pattern.contains(['\\', '\0']) {
            return None;
        }
        if pattern.is_empty() {
            return Some(Self(Vec::new()));
        }
        let segments: Vec<_> = pattern.split('/').collect();
        segments
            .iter()
            .all(|segment| !matches!(*segment, "" | "." | ".."))
            .then_some(Self(segments))
    }

    pub(super) fn matches(&self, path: &str) -> bool {
        match self.0.as_slice() {
            [] => true,
            [pattern] => segment_matches(pattern, path.rsplit('/').next().unwrap_or(path)),
            patterns => {
                let names: Vec<_> = path.split('/').collect();
                wildcard_match(
                    patterns,
                    &names,
                    |pattern| *pattern == "**",
                    |pattern, name| segment_matches(pattern, name),
                )
            }
        }
    }
}

fn segment_matches(pattern: &str, name: &str) -> bool {
    if !pattern.contains(['*', '?']) {
        return pattern == name;
    }
    let pattern: Vec<_> = pattern.chars().collect();
    let name: Vec<_> = name.chars().collect();
    wildcard_match(
        &pattern,
        &name,
        |c| *c == '*',
        |c, name| *c == '?' || c == name,
    )
}

fn wildcard_match<P, N>(
    pattern: &[P],
    name: &[N],
    any_count: impl Fn(&P) -> bool,
    matches_one: impl Fn(&P, &N) -> bool,
) -> bool {
    let (mut p, mut n) = (0, 0);
    let mut wildcard = None;
    while n < name.len() {
        if pattern.get(p).is_some_and(&any_count) {
            wildcard = Some((p, n));
            p += 1;
        } else if pattern
            .get(p)
            .is_some_and(|part| matches_one(part, &name[n]))
        {
            p += 1;
            n += 1;
        } else if let Some((previous, start)) = wildcard {
            p = previous + 1;
            n = start + 1;
            wildcard = Some((previous, n));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(any_count)
}

#[cfg(test)]
mod tests {
    use super::Glob;

    #[test]
    fn wildcard_matching_is_case_sensitive_and_path_aware() {
        let glob = Glob::parse("**/notes/t?st.*").unwrap();
        assert!(glob.matches("notes/test.txt"));
        assert!(glob.matches("x/y/notes/tést.md"));
        assert!(!glob.matches("x/y/Notes/test.txt"));
        assert!(!glob.matches("notes/teest.txt"));
        assert!(!Glob::parse("x/*.txt").unwrap().matches("x/y/z.txt"));
        assert!(Glob::parse("*.txt").unwrap().matches("x/y/z.txt"));
        assert!(Glob::parse("*").unwrap().matches("a"));
        assert!(Glob::parse("a/*").unwrap().matches("a/b"));
        assert!(!Glob::parse("a/*").unwrap().matches("b"));
        assert!(Glob::parse("../*").is_none());
    }
}
