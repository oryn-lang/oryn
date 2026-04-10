//! Oryn language library. Compile source code to bytecode and run it.
//!
//! # Pipeline
//!
//! ```text
//!   source.on
//!       |
//!       v
//!   +--------+     Vec<(Token, Span)>
//!   | Lexer  | ----------------------.
//!   | logos  |                        |
//!   +--------+                        v
//!       |  errors              +-----------+     Vec<Statement>
//!       |  (bad tokens)        |  Parser   | -----------------.
//!       v                      | chumsky   |                  |
//!   OrynError::Lexer           +-----------+                  v
//!                                  |  errors          +-----------+
//!                                  |  (syntax)        | Compiler  |
//!                                  v                  +-----------+
//!                              OrynError::Parser          |
//!                                                         | instructions
//!                                                         | obj_defs
//!                                                         | functions
//!                                         errors          | errors
//!                                   (undefined vars,      v
//!                                    val reassign,   +---------+
//!                                    bad fields)     |  Chunk  |
//!                                                    +---------+
//!                              OrynError::Compiler        |
//!                                                         v
//!                                                    +----------+     output
//!                                                    |   VM     | ---------->
//!                                                    | gc-arena |
//!                                                    +----------+
//!                                                         |
//!                                                         v
//!                                                  RuntimeError
//!                                               (type mismatch,
//!                                                div by zero,
//!                                                stack underflow)
//! ```
//!
//! # Usage
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
mod modules;
mod parser;
mod visitor;
mod vm;

pub use errors::{OrynError, RuntimeError};
pub use lexer::{Token, lex};
pub use parser::{
    BinOp, Expression, ObjField, ObjMethod, Spanned, Statement, StringPart, TypeAnnotation,
    UnaryOp, parse,
};
pub use visitor::{AstVisitor, walk_expr, walk_stmt, walk_stmts};
pub use vm::{Chunk, VM};
