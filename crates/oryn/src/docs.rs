//! Doc-comment sidecar. [`DocTable`] collects every `//` comment in a
//! source file and answers "what comment, if any, is directly above
//! this declaration?" without touching the AST or compiler.
//!
//! The LSP uses this to enrich hover tooltips; a future doc-test or
//! doc-generation subcommand can walk the same structure.

use std::ops::Range;

use crate::lexer::{Token, lex_all};

/// A sorted list of line comments extracted from a source file, with
/// a lookup that matches the "contiguous `//` block directly above a
/// declaration" convention used by most modern languages.
#[derive(Debug, Clone)]
pub struct DocTable {
    /// `(span, raw text)` for every comment in the file, in source
    /// order. Text includes the leading `//` exactly as written.
    comments: Vec<(Range<usize>, String)>,
}

impl DocTable {
    /// Build a table by re-lexing `source` with trivia preserved.
    pub fn build(source: &str) -> Self {
        let (tokens, _errors) = lex_all(source);
        let comments = tokens
            .into_iter()
            .filter_map(|(tok, span)| match tok {
                Token::Comment(text) => Some((span, text)),
                _ => None,
            })
            .collect();
        Self { comments }
    }

    /// Return the contiguous block of `//` lines immediately above the
    /// declaration that starts at byte offset `decl_start`. Each line's
    /// `//` prefix and one optional following space is stripped; lines
    /// are joined with `\n`. Returns `None` if no comment sits directly
    /// above the declaration (any blank line or code between them
    /// breaks the block).
    pub fn lookup_above(&self, source: &str, decl_start: usize) -> Option<String> {
        if self.comments.is_empty() {
            return None;
        }

        let decl_line = line_of(source, decl_start);
        if decl_line == 0 {
            return None;
        }

        // Walk comments in reverse (bottom-up). Only accept ones whose
        // line number is exactly `expected`, decrementing `expected` by
        // one each time. The first mismatch ends the block.
        let mut expected = decl_line - 1;
        let mut collected: Vec<String> = Vec::new();

        for (span, text) in self.comments.iter().rev() {
            if span.end > decl_start {
                // Comment is after (or overlapping) the declaration.
                continue;
            }
            let comment_line = line_of(source, span.start);

            if comment_line == expected {
                // Also confirm this comment is the *entire* line — i.e.
                // nothing non-whitespace precedes it on its own line.
                // Trailing comments (`let x = 5 // note`) are out of
                // scope; they should not attach to the next decl.
                if !is_standalone_comment(source, span.start) {
                    break;
                }
                collected.push(strip_slashes(text));
                if expected == 0 {
                    break;
                }
                expected -= 1;
            } else {
                break;
            }
        }

        if collected.is_empty() {
            None
        } else {
            collected.reverse();
            Some(collected.join("\n"))
        }
    }
}

/// Zero-based line number of the byte offset in `source`.
fn line_of(source: &str, offset: usize) -> usize {
    let end = offset.min(source.len());
    source[..end].bytes().filter(|&b| b == b'\n').count()
}

/// True if the byte range leading up to `comment_start` on its own
/// line contains only whitespace — i.e. the comment is not trailing
/// after some code on the same line.
fn is_standalone_comment(source: &str, comment_start: usize) -> bool {
    let line_start = source[..comment_start]
        .rfind('\n')
        .map(|i| i + 1)
        .unwrap_or(0);
    source[line_start..comment_start]
        .bytes()
        .all(|b| b == b' ' || b == b'\t')
}

/// Strip the `//` prefix and one optional leading space.
fn strip_slashes(text: &str) -> String {
    let stripped = text.strip_prefix("//").unwrap_or(text);
    stripped.strip_prefix(' ').unwrap_or(stripped).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc_above(source: &str, needle: &str) -> Option<String> {
        let offset = source.find(needle).expect("needle not in source");
        DocTable::build(source).lookup_above(source, offset)
    }

    #[test]
    fn single_comment_above_function() {
        let source = "// adds two numbers\nfn add(a, b) { rn a + b }";
        assert_eq!(doc_above(source, "fn add"), Some("adds two numbers".into()));
    }

    #[test]
    fn multi_line_block_above_is_joined() {
        let source = "// first line\n// second line\n// third line\nlet x = 1";
        assert_eq!(
            doc_above(source, "let x"),
            Some("first line\nsecond line\nthird line".into())
        );
    }

    #[test]
    fn blank_line_breaks_the_block() {
        let source = "// old comment\n\nlet x = 1";
        assert_eq!(doc_above(source, "let x"), None);
    }

    #[test]
    fn only_the_adjacent_block_is_collected() {
        let source = "// unrelated\n\n// doc line 1\n// doc line 2\nfn foo() {}";
        assert_eq!(
            doc_above(source, "fn foo"),
            Some("doc line 1\ndoc line 2".into())
        );
    }

    #[test]
    fn declaration_at_top_of_file_has_no_comment() {
        let source = "let x = 1";
        assert_eq!(doc_above(source, "let x"), None);
    }

    #[test]
    fn code_between_comment_and_decl_breaks_the_block() {
        let source = "// doc\nlet y = 0\nlet x = 1";
        assert_eq!(doc_above(source, "let x"), None);
    }

    #[test]
    fn trailing_comment_does_not_attach_to_next_decl() {
        let source = "let y = 1 // trailing\nlet x = 2";
        assert_eq!(doc_above(source, "let x"), None);
    }

    #[test]
    fn comment_inside_string_literal_is_not_picked_up() {
        // logos handles strings, so "//" inside a string never becomes
        // a Comment token in the first place.
        let source = "let s = \"// not a comment\"\nfn foo() {}";
        assert_eq!(doc_above(source, "fn foo"), None);
    }

    #[test]
    fn strip_slashes_removes_prefix_and_optional_space() {
        assert_eq!(strip_slashes("// hello"), "hello");
        assert_eq!(strip_slashes("//no space"), "no space");
        assert_eq!(strip_slashes("//  double space"), " double space");
    }

    #[test]
    fn indented_comment_block_is_accepted() {
        let source = "obj Foo {\n    // field doc\n    x: int\n}";
        assert_eq!(doc_above(source, "x: int"), Some("field doc".into()));
    }
}
