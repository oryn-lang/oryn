use std::io;

use ariadne::{Color, Label, Report, ReportKind, Source};
use oryn::OrynError;

// Renders errors using ariadne. All error variants — Lexer, Parser, and
// Runtime — get source-highlighted diagnostics when a span is available.
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
            OrynError::Runtime(e) => {
                let message = e.to_string();

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
        }
    }

    Ok(())
}
