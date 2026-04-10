mod block;
mod compile;
mod expr;
mod func;
mod obj;
mod stmt;
mod tables;
mod types;

#[cfg(test)]
mod tests;

pub(crate) use compile::compile;
pub(crate) use types::BuiltinFunction;
pub(crate) use types::CompiledFunction;
pub(crate) use types::CompilerOutput;
pub(crate) use types::Instruction;
pub(crate) use types::ModuleExports;
pub(crate) use types::ModuleTable;
pub(crate) use types::ObjDefInfo;
pub use types::{TestInfo, TypeMap};
