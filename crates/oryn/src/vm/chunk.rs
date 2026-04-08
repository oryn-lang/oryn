use std::ops::Range;

use crate::compiler;
use crate::compiler::{CompiledFunction, Instruction};
use crate::errors::OrynError;
use crate::lexer;
use crate::parser;

/// Compiled bytecode ready to be run by a [`super::VM`].
///
/// ```
/// let chunk = oryn::Chunk::compile("let x = 5\nprint(x)").unwrap();
/// let mut vm = oryn::VM::new();
///
/// vm.run(&chunk).unwrap();
/// ```
#[derive(Debug)]
pub struct Chunk {
    pub(crate) instructions: Vec<Instruction>,
    pub(crate) spans: Vec<Range<usize>>,
    pub(crate) functions: Vec<CompiledFunction>,
}

impl Chunk {
    /// Compiles source code into a [`Chunk`].
    ///
    /// ```
    /// let chunk = oryn::Chunk::compile("let x = 1 + 2").unwrap();
    /// ```
    ///
    /// Returns lex/parse errors if the source is invalid:
    ///
    /// ```
    /// let err = oryn::Chunk::compile("let = @").unwrap_err();
    ///
    /// assert!(!err.is_empty());
    /// ```
    pub fn compile(source: &str) -> Result<Self, Vec<OrynError>> {
        let (tokens, lex_errors) = lexer::lex(source);
        let (statements, parse_errors) = parser::parse(tokens);

        let errors: Vec<_> = lex_errors.into_iter().chain(parse_errors).collect();
        if !errors.is_empty() {
            return Err(errors);
        }

        let output = compiler::compile(statements);

        Ok(Self {
            instructions: output.instructions,
            spans: output.spans,
            functions: output.functions,
        })
    }

    /// Returns all lex and parse errors without compiling. An empty
    /// vec means the source is valid.
    ///
    /// ```
    /// assert!(oryn::Chunk::check("let x = 5").is_empty());
    /// assert!(!oryn::Chunk::check("let = @").is_empty());
    /// ```
    pub fn check(source: &str) -> Vec<OrynError> {
        let (tokens, lex_errors) = lexer::lex(source);
        let (_, parse_errors) = parser::parse(tokens);

        lex_errors.into_iter().chain(parse_errors).collect()
    }

    /// Returns a human-readable disassembly of the compiled bytecode.
    ///
    /// ```
    /// let chunk = oryn::Chunk::compile("let x = 5\nprint(x)").unwrap();
    /// let output = chunk.disassemble();
    ///
    /// assert!(output.contains("SetLocal"));
    /// assert!(output.contains("CallBuiltin"));
    /// ```
    pub fn disassemble(&self) -> String {
        use std::fmt::Write;

        let mut out = String::new();

        writeln!(out, "== <main> ==").unwrap();
        disassemble_instructions(&mut out, &self.instructions);

        for func in &self.functions {
            let params = func.params.join(", ");
            writeln!(out, "\n== {}({}) ==", func.name, params).unwrap();
            disassemble_instructions(&mut out, &func.instructions);
        }

        out
    }
}

fn disassemble_instructions(out: &mut String, instructions: &[Instruction]) {
    use std::fmt::Write;

    for (i, instr) in instructions.iter().enumerate() {
        let formatted = match instr {
            Instruction::PushBool(b) => format!("PushBool {b}"),
            Instruction::PushFloat(n) => format!("PushFloat {n}"),
            Instruction::PushInt(n) => format!("PushInt {n}"),
            Instruction::PushString(s) => format!("PushString {s}"),
            Instruction::GetLocal(slot) => format!("GetLocal {slot}"),
            Instruction::SetLocal(slot) => format!("SetLocal {slot}"),
            Instruction::Return => "Return".to_string(),
            Instruction::Equal => "Equal".to_string(),
            Instruction::NotEqual => "NotEqual".to_string(),
            Instruction::LessThan => "LessThan".to_string(),
            Instruction::GreaterThan => "GreaterThan".to_string(),
            Instruction::LessThanEquals => "LessThanEquals".to_string(),
            Instruction::GreaterThanEquals => "GreaterThanEquals".to_string(),
            Instruction::And => "And".to_string(),
            Instruction::Or => "Or".to_string(),
            Instruction::Not => "Not".to_string(),
            Instruction::Add => "Add".to_string(),
            Instruction::Sub => "Sub".to_string(),
            Instruction::Mul => "Mul".to_string(),
            Instruction::Div => "Div".to_string(),
            Instruction::Call(idx, arity) => {
                let s = if *arity == 1 { "arg" } else { "args" };
                format!("Call fn#{idx} ({arity} {s})")
            }
            Instruction::CallBuiltin(name, arity) => {
                let s = if *arity == 1 { "arg" } else { "args" };
                format!("CallBuiltin \"{name}\" ({arity} {s})")
            }
            Instruction::Pop => "Pop".to_string(),
            Instruction::JumpIfFalse(target) => format!("JumpIfFalse -> {target:04}"),
            Instruction::Jump(target) => format!("Jump -> {target:04}"),
        };

        writeln!(out, "{i:04}  {formatted}").unwrap();
    }
}
