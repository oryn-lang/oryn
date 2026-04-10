use oryn::{OrynError, Spanned, Statement, Token, parse};

use crate::comments::CommentAttachments;
use crate::printer::Formatter;

#[derive(Clone, Debug)]
pub(crate) struct Comment {
    pub(crate) text: String,
    pub(crate) offset: usize,
    pub(crate) end: usize,
    pub(crate) standalone: bool,
}

pub(crate) struct ParsedSource {
    pub(crate) source: String,
    pub(crate) stmts: Vec<Spanned<Statement>>,
    pub(crate) line_starts: Vec<usize>,
    pub(crate) comments: Vec<Comment>,
}

pub fn format_source(source: &str) -> Result<String, Vec<OrynError>> {
    let parsed = ParsedSource::parse(source)?;
    let attachments = CommentAttachments::build(&parsed);
    let mut formatter = Formatter::new(&parsed, attachments);
    formatter.write_program();
    Ok(formatter.finish())
}

impl ParsedSource {
    fn parse(source: &str) -> Result<Self, Vec<OrynError>> {
        let (all_tokens, _) = oryn::lex_all(source);
        let line_starts = compute_line_starts(source);
        let comments = extract_comments(source, &all_tokens, &line_starts);

        let (tokens, lex_errors) = oryn::lex(source);
        let (stmts, parse_errors) = parse(tokens);

        let errors: Vec<_> = lex_errors.into_iter().chain(parse_errors).collect();
        if !errors.is_empty() {
            return Err(errors);
        }

        Ok(Self {
            source: source.to_string(),
            stmts,
            line_starts,
            comments,
        })
    }
}

pub(crate) fn compute_line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (i, ch) in source.char_indices() {
        if ch == '\n' {
            starts.push(i + 1);
        }
    }
    starts
}

pub(crate) fn line_of(offset: usize, line_starts: &[usize]) -> usize {
    match line_starts.binary_search(&offset) {
        Ok(line) => line,
        Err(line) => line.saturating_sub(1),
    }
}

pub(crate) fn has_blank_line_between(source: &str, from: usize, to: usize) -> bool {
    if from >= to || from >= source.len() {
        return false;
    }

    let end = to.min(source.len());
    let slice = &source[from..end];
    let mut saw_newline = false;

    for ch in slice.chars() {
        if ch == '\n' {
            if saw_newline {
                return true;
            }
            saw_newline = true;
        } else if !ch.is_ascii_whitespace() {
            saw_newline = false;
        }
    }

    false
}

fn extract_comments(
    source: &str,
    all_tokens: &[(Token, std::ops::Range<usize>)],
    line_starts: &[usize],
) -> Vec<Comment> {
    all_tokens
        .iter()
        .filter_map(|(tok, span)| {
            if let Token::Comment(text) = tok {
                let offset = span.start;
                let line = line_of(offset, line_starts);
                let line_start = line_starts[line];
                let before = &source[line_start..offset];
                let standalone = before.chars().all(|c| c.is_ascii_whitespace());
                Some(Comment {
                    text: text.clone(),
                    offset,
                    end: span.end,
                    standalone,
                })
            } else {
                None
            }
        })
        .collect()
}
