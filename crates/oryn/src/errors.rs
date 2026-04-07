use std::fmt;

/// An error from any phase of the pipeline: lexing, parsing, or runtime.
///
/// `Lexer` and `Parser` variants carry byte-offset spans for diagnostics.
/// `Runtime` wraps a [`RuntimeError`] from the VM.
///
/// ```
/// let errors = oryn::Chunk::check("let x = @");
/// assert!(!errors.is_empty());
///
/// for error in &errors {
///     match error {
///         oryn::OrynError::Lexer { span } => {
///             println!("bad token at {}..{}", span.start, span.end);
///         }
///         oryn::OrynError::Parser { span, message } => {
///             println!("{message} at {}..{}", span.start, span.end);
///         }
///         oryn::OrynError::Runtime(e) => {
///             println!("runtime: {e}");
///         }
///     }
/// }
/// ```
#[derive(Debug)]
pub enum OrynError {
    Lexer {
        span: std::ops::Range<usize>,
    },
    Parser {
        span: std::ops::Range<usize>,
        message: String,
    },
    Runtime(RuntimeError),
}

/// A runtime error from the VM.
#[derive(Debug)]
pub enum RuntimeError {
    UndefinedVariable(String),
    UndefinedFunction(String),
    StackUnderflow,
    IoError(std::io::Error),
}

impl fmt::Display for OrynError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OrynError::Lexer { span } => {
                write!(f, "unexpected character at {}..{}", span.start, span.end)
            }
            OrynError::Parser { message, .. } => write!(f, "{message}"),
            OrynError::Runtime(e) => write!(f, "{e}"),
        }
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuntimeError::UndefinedVariable(name) => write!(f, "undefined variable: {name}"),
            RuntimeError::UndefinedFunction(name) => write!(f, "undefined function: {name}"),
            RuntimeError::StackUnderflow => write!(f, "stack underflow"),
            RuntimeError::IoError(e) => write!(f, "{e}"),
        }
    }
}

impl From<RuntimeError> for OrynError {
    fn from(e: RuntimeError) -> Self {
        OrynError::Runtime(e)
    }
}
