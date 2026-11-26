use super::Match;

/// A search typed by a person. Prepare it once for a list of results.
#[derive(Debug)]
pub struct SearchQuery {
    pub(super) phrase: String,
    pub(super) terms: Vec<Term>,
    valid: bool,
}

impl SearchQuery {
    /// Bound work before normalizing pasted text.
    pub const MAX_BYTES: usize = 4096;
    const MAX_TERMS: usize = 64;

    /// Basic admission for literal filters that do not need token ranking.
    pub fn accepts(text: &str) -> bool {
        text.len() <= Self::MAX_BYTES && !text.chars().any(|c| c.is_control() && !c.is_whitespace())
    }

    pub fn new(text: &str) -> Self {
        let rejected = Self {
            phrase: String::new(),
            terms: Vec::new(),
            valid: false,
        };
        if !Self::accepts(text) {
            return rejected;
        }
        let phrase = fold_search(text);
        if phrase.split_whitespace().nth(Self::MAX_TERMS).is_some() {
            return rejected;
        }
        let terms = phrase.split_whitespace().map(Term::new).collect();
        Self {
            phrase,
            terms,
            valid: true,
        }
    }

    /// Whether this input can be matched as search text.
    pub fn is_valid(&self) -> bool {
        self.valid
    }

    pub fn is_empty(&self) -> bool {
        self.valid && self.terms.is_empty()
    }

    /// Whether every search term occurs in the supplied captions or matches a typo.
    pub fn matches<'a>(&self, fields: impl IntoIterator<Item = &'a str>) -> bool {
        self.relevance(fields).is_some()
    }

    /// Direct matches rank above typo matches. An empty search matches every row.
    pub fn relevance<'a>(&self, fields: impl IntoIterator<Item = &'a str>) -> Option<Match> {
        if !self.valid {
            return None;
        }
        if self.is_empty() {
            return Some(Match::Exact);
        }
        let fields: Vec<_> = fields.into_iter().map(fold_search).collect();
        if fields.iter().any(|text| text == &self.phrase) {
            return Some(Match::Exact);
        }
        if fields.iter().any(|text| text.starts_with(&self.phrase)) {
            return Some(Match::NameStart);
        }
        self.terms.iter().try_fold(Match::Exact, |worst, term| {
            let matched = if fields.iter().any(|text| text.contains(&term.text)) {
                Match::WordStart
            } else if fields.iter().any(|text| term.typo_in(text)) {
                Match::Typo
            } else {
                return None;
            };
            Some(worst.max(matched))
        })
    }
}

/// Fold case, common Latin accents and full-width ASCII, and normalize separators.
/// Trailing minus signs and signs in front of a number are retained.
pub fn fold_search(text: &str) -> String {
    let mut folded = String::with_capacity(text.len());
    let mut chars = text
        .chars()
        .map(|c| match c {
            '\u{ff01}'..='\u{ff5e}' => char::from_u32(u32::from(c) - 0xfee0).unwrap_or(c),
            '\u{ff61}'..='\u{ff9f}' => "。「」、・ヲァィゥェォャュョッーアイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワン\u{3099}\u{309a}"
                .chars()
                .nth((u32::from(c) - 0xff61) as usize)
                .unwrap_or(c),
            c => c,
        })
        .flat_map(char::to_lowercase)
        .peekable();
    let mut gap = false;
    while let Some(c) = chars.next() {
        let separator = c.is_whitespace()
            || matches!(c, '_' | '\u{2010}'..='\u{2015}')
            || c == '-'
                && chars.clone().any(|next| !next.is_whitespace())
                && !(chars.peek().is_some_and(char::is_ascii_digit) && (gap || folded.is_empty()));
        if separator {
            gap = !folded.is_empty();
            continue;
        }
        if gap {
            folded.push(' ');
            gap = false;
        }
        match c {
            '\u{3099}' | '\u{309a}' => {
                if let Some(voiced) = folded
                    .chars()
                    .next_back()
                    .and_then(|base| voiced_kana(base, c))
                {
                    folded.pop();
                    folded.push(voiced);
                } else {
                    folded.push(c);
                }
            }
            'á' | 'à' | 'â' | 'ä' | 'ã' => folded.push('a'),
            'é' | 'è' | 'ê' | 'ë' => folded.push('e'),
            'í' | 'ì' | 'î' | 'ï' => folded.push('i'),
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' => folded.push('o'),
            'ú' | 'ù' | 'û' | 'ü' => folded.push('u'),
            'ç' => folded.push('c'),
            '\u{303}' if folded.ends_with('n') => {
                folded.pop();
                folded.push('ñ');
            }
            '\u{300}' | '\u{301}' | '\u{302}' | '\u{303}' | '\u{308}' | '\u{327}'
                if folded.ends_with(['a', 'e', 'i', 'o', 'u', 'c']) => {}
            c => folded.push(c),
        }
    }
    folded
}

fn voiced_kana(base: char, mark: char) -> Option<char> {
    let offset = match mark {
        '\u{3099}' if "かきくけこさしすせそたちつてとはひふへほゝカキクケコサシスセソタチツテトハヒフヘホヽ".contains(base) => 1,
        '\u{309a}' if "はひふへほハヒフヘホ".contains(base) => 2,
        '\u{3099}' => return match base {
            'う' => Some('ゔ'),
            'ウ' => Some('ヴ'),
            'ワ' => Some('ヷ'),
            'ヰ' => Some('ヸ'),
            'ヱ' => Some('ヹ'),
            'ヲ' => Some('ヺ'),
            _ => None,
        },
        _ => return None,
    };
    char::from_u32(u32::from(base) + offset)
}

/// Whether a non-empty query matches a label or user-supplied name.
pub fn word_finds(word: &str, text: &str) -> bool {
    let query = SearchQuery::new(word);
    !query.is_empty() && query.matches([text])
}

pub(super) fn starts_a_word(text: &str, search: &str) -> bool {
    text.starts_with(search)
        || text.split_whitespace().any(|word| word.starts_with(search))
        || (search.contains(cjk) && text.contains(search))
}

pub(super) fn cjk(c: char) -> bool {
    matches!(c, '\u{3040}'..='\u{30ff}' | '\u{3400}'..='\u{9fff}' | '\u{ff66}'..='\u{ff9f}')
}

const MAX_TYPO_LENGTH: usize = 48;

#[derive(Debug)]
pub(super) struct Term {
    pub(super) text: String,
    spelling: Vec<char>,
}

impl Term {
    fn new(text: &str) -> Self {
        let spelling = if text.len() >= 4 && text.chars().all(char::is_alphabetic) {
            let letters: Vec<_> = text.chars().take(MAX_TYPO_LENGTH + 1).collect();
            if (4..=MAX_TYPO_LENGTH).contains(&letters.len()) {
                letters
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        Self {
            text: text.to_owned(),
            spelling,
        }
    }

    pub(super) fn typo_in(&self, text: &str) -> bool {
        if self.spelling.is_empty() {
            return false;
        }
        let interior = self.spelling.iter().copied().any(cjk);
        text.split(|c: char| !c.is_alphabetic()).any(|word| {
            self.typo_in_word(word)
                || interior
                    && word
                        .char_indices()
                        .skip(1)
                        .any(|(at, c)| cjk(c) && self.typo_in_word(&word[at..]))
        })
    }

    fn typo_in_word(&self, word: &str) -> bool {
        let len = self.spelling.len();
        if word.len() < len - 1 {
            return false;
        }
        let mut chars = ['\0'; MAX_TYPO_LENGTH + 1];
        let mut count = 0;
        for (slot, c) in chars[..=len].iter_mut().zip(word.chars()) {
            *slot = c;
            count += 1;
        }
        within_one(&self.spelling, &chars[..count])
            || count > len && within_one(&self.spelling, &chars[..len])
    }
}

/// Edit distance at most one, counting a swap of two adjacent letters as one.
fn within_one(a: &[char], b: &[char]) -> bool {
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let first = short.iter().zip(long).position(|(a, b)| a != b);
    match (long.len() - short.len(), first) {
        (0 | 1, None) => true,
        (1, Some(at)) => short[at..] == long[at + 1..],
        (0, Some(at)) => {
            a[at + 1..] == b[at + 1..]
                || at + 1 < a.len()
                    && a[at] == b[at + 1]
                    && a[at + 1] == b[at]
                    && a[at + 2..] == b[at + 2..]
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn japanese_width_and_voicing_variants_match_the_same_caption() {
        for (typed, caption) in [
            ("ｶﾗｰ", "カラー"),
            ("ｶﾞｯﾂﾎﾟｰｽﾞ", "ガッツポーズ"),
            ("か\u{3099}", "が"),
            ("は\u{309a}", "ぱ"),
        ] {
            assert_eq!(
                SearchQuery::new(typed).relevance([caption]),
                Some(Match::Exact)
            );
        }
        assert!(!SearchQuery::new("か").matches(["が"]));
    }

    #[test]
    fn separators_and_accents_are_folded_without_losing_negative_values() {
        assert_eq!(
            fold_search("  NúMEro_Entero – JSON　"),
            "numero entero json"
        );
        assert_eq!(fold_search("audio - level"), "audio level");
        assert_eq!(fold_search("UTF-8"), "utf 8");
        assert_eq!(fold_search("bump-"), "bump-");
        assert_eq!(fold_search("range -12"), "range -12");
    }

    #[test]
    fn typos_are_one_edit() {
        let chars = |text: &str| text.chars().collect::<Vec<_>>();
        for typo in ["tiemr", "timr", "timers", "tamer"] {
            assert!(within_one(&chars("timer"), &chars(typo)));
        }
        assert!(!within_one(&chars("timer"), &chars("tmire")));
        assert!(!within_one(&chars("timer"), &chars("time it")));
    }

    #[test]
    fn mixed_language_terms_can_match_different_fields() {
        let query = SearchQuery::new("cuadricual-spacing");
        assert_eq!(
            query.relevance(["Cuadrícula", "Grid spacing"]),
            Some(Match::Typo)
        );
        assert!(SearchQuery::new("ＪＳＯＮ-パス").matches(["JSON のパス"]));
        assert_eq!(
            SearchQuery::new("フィルタア").relevance(["オーディオフィルター"]),
            Some(Match::Typo)
        );
        assert!(!SearchQuery::new("grd").matches(["grid"]));
        assert!(!SearchQuery::new("1081").matches(["1080"]));
        assert!(!SearchQuery::new("bump-").matches(["bump+", "count up"]));
    }
}
