use std::io;

use ariadne::{Color, Label, Report, ReportKind, Source};
use oryn::{OrynError, RuntimeError};

// Renders errors using ariadne. All error variants: Lexer, Parser, and
// Runtime. Get source-highlighted diagnostics when a span is available.
pub fn report_errors(filename: &str, source: &str, errors: &[OrynError]) -> io::Result<()> {
    let src = Source::from(source);

    for error in errors {
        match error {
            OrynError::Lexer { span } => {
                Report::build(ReportKind::Error, (filename, span.clone()))
                    .with_message("unexpected character")
                    .with_label(
                        Label::new((filename, span.clone()))
                            .with_message("not recognized")
                            .with_color(Color::Red),
                    )
                    .finish()
                    .eprint((filename, src.clone()))?;
            }
            OrynError::Parser { span, message } => {
                Report::build(ReportKind::Error, (filename, span.clone()))
                    .with_message(message)
                    .with_label(
                        Label::new((filename, span.clone()))
                            .with_message(message)
                            .with_color(Color::Red),
                    )
                    .finish()
                    .eprint((filename, src.clone()))?;
            }
            OrynError::Compiler { span, message } => {
                Report::build(ReportKind::Error, (filename, span.clone()))
                    .with_message(message)
                    .with_label(
                        Label::new((filename, span.clone()))
                            .with_message(message)
                            .with_color(Color::Red),
                    )
                    .finish()
                    .eprint((filename, src.clone()))?;
            }
            OrynError::Runtime(e) => {
                // Assertion failures get a richer message: the
                // condition's source text is sliced from the span so
                // the report reads "assertion failed: result == 5"
                // instead of just "assertion failed". All other
                // runtime errors use their Display impl verbatim.
                let message = match e {
                    RuntimeError::AssertionFailed { span: Some(span) } => {
                        let snippet = source
                            .get(span.start..span.end)
                            .map(str::trim)
                            .unwrap_or("");
                        if snippet.is_empty() {
                            "assertion failed".to_string()
                        } else {
                            format!("assertion failed: {snippet}")
                        }
                    }
                    _ => e.to_string(),
                };

                if let Some(span) = e.span() {
                    Report::build(ReportKind::Error, (filename, span.clone()))
                        .with_message(&message)
                        .with_label(
                            Label::new((filename, span.clone()))
                                .with_message(&message)
                                .with_color(Color::Red),
                        )
                        .finish()
                        .eprint((filename, src.clone()))?;
                } else {
                    // No span available — fall back to plain output.
                    eprintln!("runtime error: {message}");
                }
            }
            OrynError::Module { path, message } => {
                eprintln!("module error: {}: {}", path, message);
            }
        }
    }

    Ok(())
}
