//! Oryn language library. Compile source code to bytecode and run it.
//!
//! ```
//! let chunk = oryn::Chunk::compile("let x = 5\nprint(x)").unwrap();
//! let mut vm = oryn::VM::new();
//!
//! vm.run(&chunk).unwrap();
//! ```

mod compiler;
mod errors;
mod lexer;
mod parser;
mod vm;

pub use errors::{OrynError, RuntimeError};
pub use lexer::{Token, lex};
pub use parser::{BinOp, Expression, Statement, parse};
pub use vm::{Chunk, VM};
