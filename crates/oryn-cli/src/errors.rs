use std::io;

use ariadne::{Color, Label, Report, ReportKind, Source};
use oryn::OrynError;

// Renders errors using ariadne. Only `Lexer` and `Parser` variants have
// spans to point at — `Runtime` errors are printed as plain messages.
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
                eprintln!("runtime error: {e}");
            }
        }
    }

    Ok(())
}
