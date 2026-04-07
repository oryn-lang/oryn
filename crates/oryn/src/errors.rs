use std::fmt;
use std::ops::Range;

use crate::vm::value::Value;

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
    Lexer { span: Range<usize> },
    Parser { span: Range<usize>, message: String },
    Runtime(RuntimeError),
}

/// A runtime error from the VM.
///
/// Each variant carries an optional byte-offset span pointing back to
/// the source instruction that caused the error. When present, the CLI
/// can render full ariadne diagnostics with source highlighting.
#[derive(Debug)]
pub enum RuntimeError {
    UndefinedVariable {
        name: String,
        span: Option<Range<usize>>,
    },
    UndefinedFunction {
        name: String,
        span: Option<Range<usize>>,
    },
    StackUnderflow,
    IoError(std::io::Error),
    TypeError {
        expected: ValueType,
        actual: ValueType,
        span: Option<Range<usize>>,
    },
    ArityMismatch {
        name: String,
        expected: usize,
        actual: usize,
        span: Option<Range<usize>>,
    },
}

/// The type of a value.
#[derive(Debug)]
pub enum ValueType {
    Bool,
    Int,
    String,
}

impl From<&Value<'_>> for ValueType {
    fn from(value: &Value<'_>) -> Self {
        match value {
            Value::Bool(_) => ValueType::Bool,
            Value::Int(_) => ValueType::Int,
            Value::String(_) => ValueType::String,
        }
    }
}

impl fmt::Display for ValueType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValueType::Bool => write!(f, "bool"),
            ValueType::Int => write!(f, "int"),
            ValueType::String => write!(f, "string"),
        }
    }
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
            RuntimeError::UndefinedVariable { name, .. } => {
                write!(f, "undefined variable: {name}")
            }
            RuntimeError::UndefinedFunction { name, .. } => {
                write!(f, "undefined function: {name}")
            }
            RuntimeError::StackUnderflow => write!(f, "stack underflow"),
            RuntimeError::IoError(e) => write!(f, "{e}"),
            RuntimeError::TypeError {
                expected, actual, ..
            } => {
                write!(f, "type error: expected {expected}, got {actual}")
            }
            RuntimeError::ArityMismatch {
                name,
                expected,
                actual,
                ..
            } => {
                write!(f, "{name} expects {expected} argument(s), got {actual}")
            }
        }
    }
}

impl RuntimeError {
    /// Returns the source span associated with this error, if available.
    pub fn span(&self) -> Option<&Range<usize>> {
        match self {
            RuntimeError::UndefinedVariable { span, .. } => span.as_ref(),
            RuntimeError::UndefinedFunction { span, .. } => span.as_ref(),
            RuntimeError::StackUnderflow => None,
            RuntimeError::IoError(_) => None,
            RuntimeError::TypeError { span, .. } => span.as_ref(),
            RuntimeError::ArityMismatch { span, .. } => span.as_ref(),
        }
    }
}

impl From<RuntimeError> for OrynError {
    fn from(e: RuntimeError) -> Self {
        OrynError::Runtime(e)
    }
}
