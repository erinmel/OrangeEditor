//! Structural form of canonical type expressions (11 §5.1):
//!
//! ```text
//! type      = named [ "<" type { "," type } ">" ] | param
//! named     = namespace "/" name "@" major
//! param     = "$" ident
//! namespace = ident { "." ident }   name = ident { "-" ident }   major = 1*DIGIT
//! ```
//!
//! `ident` is taken as an ASCII letter followed by letters, digits or `_`. Only canonical
//! text parses: re-displaying a term reproduces its input exactly. 00c builds interning,
//! constraints and unification on these terms.

use std::fmt;

use crate::error::IdError;
use crate::id::TypeExpr;

/// Nesting budget for parsing type expressions and decoding literals (03 requires codec
/// depth budgets). Proposed value, recorded as an open budget in 10.
pub const MAX_NESTING: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypeTerm {
    Named(NamedTerm),
    /// A definition's type parameter, without the `$`.
    Param(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NamedTerm {
    pub namespace: String,
    pub name: String,
    pub major: u32,
    pub args: Vec<TypeTerm>,
}

impl TypeTerm {
    pub fn parse(expr: &TypeExpr) -> Result<Self, IdError> {
        let text = expr.as_str();
        let invalid = || IdError::InvalidTypeExpr {
            text: text.to_owned(),
        };
        let mut parser = Parser {
            text: text.as_bytes(),
            pos: 0,
            too_deep: false,
        };
        let term = parser.term(0);
        if parser.too_deep {
            return Err(IdError::TypeTooDeep {
                text: text.to_owned(),
            });
        }
        let term = term.ok_or_else(invalid)?;
        if parser.pos != text.len() || term.to_string() != text {
            return Err(invalid());
        }
        Ok(term)
    }

    /// The arguments of `core/<name>@1`, if this is that type.
    pub fn core(&self, name: &str) -> Option<&[TypeTerm]> {
        match self {
            TypeTerm::Named(named)
                if named.namespace == "core" && named.name == name && named.major == 1 =>
            {
                Some(&named.args)
            }
            _ => None,
        }
    }

    /// True if a type parameter occurs anywhere in the term.
    pub fn contains_param(&self) -> bool {
        match self {
            TypeTerm::Param(_) => true,
            TypeTerm::Named(named) => named.args.iter().any(TypeTerm::contains_param),
        }
    }

    /// The named type without arguments, as the canonical text used to look up schemas.
    pub fn named(&self) -> Option<&NamedTerm> {
        match self {
            TypeTerm::Named(named) => Some(named),
            TypeTerm::Param(_) => None,
        }
    }
}

impl fmt::Display for TypeTerm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypeTerm::Param(name) => write!(f, "${name}"),
            TypeTerm::Named(named) => {
                write!(f, "{}/{}@{}", named.namespace, named.name, named.major)?;
                if let Some((first, rest)) = named.args.split_first() {
                    write!(f, "<{first}")?;
                    for arg in rest {
                        write!(f, ",{arg}")?;
                    }
                    f.write_str(">")?;
                }
                Ok(())
            }
        }
    }
}

struct Parser<'a> {
    text: &'a [u8],
    pos: usize,
    /// Set when nesting exceeds [`MAX_NESTING`], so the caller reports a budget, not syntax.
    too_deep: bool,
}

impl Parser<'_> {
    fn term(&mut self, depth: usize) -> Option<TypeTerm> {
        if depth > MAX_NESTING {
            self.too_deep = true;
            return None;
        }
        if self.eat(b'$') {
            return self.ident().map(|name| TypeTerm::Param(name.to_owned()));
        }
        let namespace = self.joined(b'.')?;
        self.expect(b'/')?;
        let name = self.joined(b'-')?;
        self.expect(b'@')?;
        let major = self.digits()?;
        let mut args = Vec::new();
        if self.eat(b'<') {
            loop {
                args.push(self.term(depth + 1)?);
                if self.eat(b'>') {
                    break;
                }
                self.expect(b',')?;
            }
        }
        Some(TypeTerm::Named(NamedTerm {
            namespace,
            name,
            major,
            args,
        }))
    }

    /// `ident { sep ident }`.
    fn joined(&mut self, separator: u8) -> Option<String> {
        let start = self.pos;
        self.ident()?;
        while self.eat(separator) {
            self.ident()?;
        }
        Some(String::from_utf8_lossy(&self.text[start..self.pos]).into_owned())
    }

    fn ident(&mut self) -> Option<&str> {
        let start = self.pos;
        if !self.peek().is_some_and(|b| b.is_ascii_alphabetic()) {
            return None;
        }
        while self
            .peek()
            .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            self.pos += 1;
        }
        std::str::from_utf8(&self.text[start..self.pos]).ok()
    }

    fn digits(&mut self) -> Option<u32> {
        let start = self.pos;
        while self.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.pos += 1;
        }
        std::str::from_utf8(&self.text[start..self.pos])
            .ok()?
            .parse()
            .ok()
    }

    fn peek(&self) -> Option<u8> {
        self.text.get(self.pos).copied()
    }

    fn eat(&mut self, byte: u8) -> bool {
        let matched = self.peek() == Some(byte);
        self.pos += usize::from(matched);
        matched
    }

    fn expect(&mut self, byte: u8) -> Option<()> {
        self.eat(byte).then_some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<TypeTerm, IdError> {
        TypeTerm::parse(&text.parse().unwrap())
    }

    #[test]
    fn parses_canonical_forms() {
        for text in [
            "core/i64@1",
            "$T",
            "core/map@1<core/string@1,core/list@1<$T>>",
            "org.example.media/playback-state@2",
            "core/handle@1<org.orange.websocket/session@1>",
        ] {
            assert_eq!(parse(text).unwrap().to_string(), text);
        }
        let list = parse("core/list@1<core/u32@1>").unwrap();
        assert_eq!(list.core("list").unwrap(), [parse("core/u32@1").unwrap()]);
        assert!(list.core("map").is_none());
        assert!(
            parse("core/list@2<core/u32@1>")
                .unwrap()
                .core("list")
                .is_none()
        );
    }

    #[test]
    fn rejects_malformed_and_non_canonical_text() {
        for text in [
            "core/i64",
            "core/i64@",
            "core/i64@01",
            "core/list@1<>",
            "core/list@1<core/i64@1",
            "core/list@1<core/i64@1, core/i64@1>",
            "core/map@1<core/i64@1,>",
            "/i64@1",
            "core//i64@1",
            "$",
            "$1",
            "core/i64@1x",
            "core/-x@1",
        ] {
            assert!(parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn nesting_is_bounded() {
        let deep = format!(
            "{}core/i64@1{}",
            "core/list@1<".repeat(MAX_NESTING + 2),
            ">".repeat(MAX_NESTING + 2)
        );
        assert!(parse(&deep).is_err());
    }
}
