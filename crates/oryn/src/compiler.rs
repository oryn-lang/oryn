mod compile;
mod tables;
mod types;

#[cfg(test)]
mod tests;

pub(crate) use compile::compile;
pub(crate) use types::CompiledFunction;
#[allow(unused_imports)]
pub(crate) use types::CompilerOutput;
pub(crate) use types::Instruction;
pub(crate) use types::ObjDefInfo;
