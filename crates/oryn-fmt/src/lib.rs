use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use glob::glob;
use oryn::{
    BinOp, Expression, ObjMethod, OrynError, Spanned, Statement, StringPart, Token, TypeAnnotation,
    UnaryOp,
};

pub fn format_source(source: &str) -> Result<String, Vec<OrynError>> {
    // Lex with comments to extract them
    let (all_tokens, _) = oryn::lex_all(source);
    let line_starts = compute_line_starts(source);
    let comments = extract_comments(source, &all_tokens, &line_starts);

    // Lex without comments for the parser
    let (tokens, lex_errors) = oryn::lex(source);
    let (stmts, parse_errors) = oryn::parse(tokens);

    let errors: Vec<_> = lex_errors.into_iter().chain(parse_errors).collect();
    if !errors.is_empty() {
        return Err(errors);
    }

    let mut formatter = Formatter {
        out: String::new(),
        indent: 0,
        comments,
        comment_cursor: 0,
        line_starts,
        source: source.to_string(),
        last_source_end: 0,
    };
    formatter.write_program(&stmts);

    Ok(formatter.finish())
}

pub fn format_target(target: &str) -> Result<Vec<PathBuf>, FormatPathError> {
    let paths = resolve_targets(target)?;
    if paths.is_empty() {
        return Err(FormatPathError::NoMatches {
            target: target.to_string(),
        });
    }

    let mut changed = Vec::new();
    for path in paths {
        if format_file(&path)? {
            changed.push(path);
        }
    }

    Ok(changed)
}

fn resolve_targets(target: &str) -> Result<Vec<PathBuf>, FormatPathError> {
    let target_path = Path::new(target);
    if target_path.is_dir() {
        return collect_glob_paths(&format!("{}/**/*.on", target_path.display()));
    }

    if target_path.is_file() {
        return Ok(vec![target_path.to_path_buf()]);
    }

    collect_glob_paths(target)
}

fn collect_glob_paths(pattern: &str) -> Result<Vec<PathBuf>, FormatPathError> {
    let mut paths = Vec::new();
    let entries = glob(pattern).map_err(|source| FormatPathError::GlobPattern {
        pattern: pattern.to_string(),
        source,
    })?;

    for entry in entries {
        match entry {
            Ok(path) if path.is_file() && is_oryn_file(&path) => paths.push(path),
            Ok(path) if path.is_dir() => {
                paths.extend(collect_glob_paths(&format!("{}/**/*.on", path.display()))?);
            }
            Ok(_) => {}
            Err(source) => {
                return Err(FormatPathError::Glob {
                    pattern: pattern.to_string(),
                    source,
                });
            }
        }
    }

    Ok(paths)
}

/// Formats a single file in place. Returns `true` if the file was
/// modified, `false` if the contents were already correctly formatted.
fn format_file(path: &Path) -> Result<bool, FormatPathError> {
    let source = fs::read_to_string(path).map_err(|source| FormatPathError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    let formatted = match format_source(&source) {
        Ok(formatted) => formatted,
        Err(errors) => {
            return Err(FormatPathError::Format {
                path: path.to_path_buf(),
                source,
                errors,
            });
        }
    };

    if formatted == source {
        return Ok(false);
    }

    fs::write(path, &formatted).map_err(|source| FormatPathError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    Ok(true)
}

fn is_oryn_file(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "on")
}

#[derive(Debug)]
pub enum FormatPathError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    GlobPattern {
        pattern: String,
        source: glob::PatternError,
    },
    Glob {
        pattern: String,
        source: glob::GlobError,
    },
    NoMatches {
        target: String,
    },
    Format {
        path: PathBuf,
        source: String,
        errors: Vec<OrynError>,
    },
}

struct CommentInfo {
    text: String,
    offset: usize,
    end: usize,
    standalone: bool,
}

fn compute_line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (i, ch) in source.char_indices() {
        if ch == '\n' {
            starts.push(i + 1);
        }
    }
    starts
}

fn line_of(offset: usize, line_starts: &[usize]) -> usize {
    match line_starts.binary_search(&offset) {
        Ok(line) => line,
        Err(line) => line - 1,
    }
}

fn extract_comments(
    source: &str,
    all_tokens: &[(Token, std::ops::Range<usize>)],
    line_starts: &[usize],
) -> Vec<CommentInfo> {
    all_tokens
        .iter()
        .filter_map(|(tok, span)| {
            if let Token::Comment(text) = tok {
                let offset = span.start;
                let line = line_of(offset, line_starts);
                let line_start = line_starts[line];
                let before = &source[line_start..offset];
                let standalone = before.chars().all(|c| c.is_ascii_whitespace());
                Some(CommentInfo {
                    text: text.clone(),
                    offset,
                    end: span.end,
                    standalone,
                })
            } else {
                None
            }
        })
        .collect()
}

struct Formatter {
    out: String,
    indent: usize,
    comments: Vec<CommentInfo>,
    comment_cursor: usize,
    line_starts: Vec<usize>,
    source: String,
    /// Byte offset in the original source just past the last item we
    /// wrote (statement end or comment end). Used so
    /// `emit_leading_comments` can detect a blank line between the
    /// preceding code/comment and the first new comment it emits.
    last_source_end: usize,
}

impl Formatter {
    fn finish(self) -> String {
        self.out
    }

    /// Push a blank line, but only if the output doesn't already end
    /// with one. Prevents double blank lines from multiple detection
    /// sites firing on the same gap.
    fn ensure_blank_line(&mut self) {
        if !self.out.ends_with("\n\n") {
            if !self.out.ends_with('\n') {
                self.out.push('\n');
            }
            self.out.push('\n');
        }
    }

    /// Emit all standalone comments whose offset < `before_offset`.
    /// Each gets indented at the current level and ends with a newline.
    /// Preserves blank lines that existed between consecutive comments
    /// (or between the preceding code and the first comment) in the
    /// original source.
    fn emit_leading_comments(&mut self, before_offset: usize) {
        // `prev_end` tracks the source-end of the last thing we wrote
        // so we can detect blank lines. Start from the last statement
        // or comment that was already emitted.
        let mut prev_end = self.last_source_end;
        let mut emitted_any = false;

        while self.comment_cursor < self.comments.len() {
            let offset = self.comments[self.comment_cursor].offset;
            if offset >= before_offset {
                break;
            }
            let standalone = self.comments[self.comment_cursor].standalone;
            if standalone {
                let text = self.comments[self.comment_cursor].text.clone();
                let end = self.comments[self.comment_cursor].end;
                // Ensure we're at the start of a new line.
                if !self.out.is_empty() && !self.out.ends_with('\n') {
                    self.out.push('\n');
                }
                // Preserve blank lines from the original source.
                if prev_end > 0 && has_blank_line_between(&self.source, prev_end, offset) {
                    self.ensure_blank_line();
                }
                self.write_indent();
                self.out.push_str(&text);
                self.out.push('\n');
                prev_end = end;
                emitted_any = true;
            }
            self.comment_cursor += 1;
        }

        if emitted_any {
            // Preserve a blank line between the last leading comment
            // and the code that follows it.
            if has_blank_line_between(&self.source, prev_end, before_offset) {
                self.ensure_blank_line();
            }
            self.last_source_end = prev_end;
        }
    }

    /// If the next unconsumed comment is a trailing (non-standalone) comment
    /// on the same source line as `code_end_offset`, emit it after two spaces.
    fn emit_trailing_comment(&mut self, code_end_offset: usize) {
        if self.comment_cursor >= self.comments.len() {
            return;
        }
        let c = &self.comments[self.comment_cursor];
        if c.standalone {
            return;
        }
        let code_line = line_of(code_end_offset.saturating_sub(1), &self.line_starts);
        let comment_line = line_of(c.offset, &self.line_starts);
        if code_line == comment_line {
            self.out.push_str("  ");
            self.out.push_str(&c.text);
            self.comment_cursor += 1;
        }
    }

    /// Emit all remaining comments whose offset < `before_offset`.
    /// Used at the end of blocks and at the end of the file.
    fn emit_remaining_comments(&mut self, before_offset: usize) {
        self.emit_leading_comments(before_offset);
    }

    fn write_program(&mut self, stmts: &[Spanned<Statement>]) {
        for (i, stmt) in stmts.iter().enumerate() {
            if i > 0 {
                self.out.push('\n');
                // Preserve blank lines from the original source, or
                // insert them between declaration-level statements.
                let prev_end = stmts[i - 1].span.end;
                let next_start = self.next_content_offset(stmt.span.start);
                if needs_blank_line_between(&stmts[i - 1].node, &stmt.node)
                    || has_blank_line_between(&self.source, prev_end, next_start)
                {
                    self.ensure_blank_line();
                }
            }

            self.emit_leading_comments(stmt.span.start);
            self.write_statement(stmt);
            self.emit_trailing_comment(stmt.span.end);
            self.last_source_end = stmt.span.end;
        }

        // Trailing comments at end of file
        self.emit_remaining_comments(usize::MAX);

        // Ensure file ends with exactly one newline.
        if !self.out.is_empty() && !self.out.ends_with('\n') {
            self.out.push('\n');
        }
    }

    /// Returns the byte offset of the next piece of content (comment or
    /// statement) at `stmt_start`. If there's a leading comment before
    /// the statement, returns its offset instead so blank-line detection
    /// looks at the gap between the previous statement and the first
    /// comment, not the gap between the previous statement and the code.
    fn next_content_offset(&self, stmt_start: usize) -> usize {
        if self.comment_cursor < self.comments.len() {
            let c = &self.comments[self.comment_cursor];
            if c.offset < stmt_start && c.standalone {
                return c.offset;
            }
        }
        stmt_start
    }

    fn write_statement(&mut self, stmt: &Spanned<Statement>) {
        self.write_indent();

        match &stmt.node {
            Statement::Let {
                name,
                value,
                type_ann,
                is_pub,
            } => {
                if *is_pub {
                    self.out.push_str("pub ");
                }
                self.out.push_str("let ");
                self.out.push_str(name);
                self.write_type_annotation(type_ann);
                self.out.push_str(" = ");
                self.write_expression(value, 0);
            }
            Statement::Val {
                name,
                value,
                type_ann,
                is_pub,
            } => {
                if *is_pub {
                    self.out.push_str("pub ");
                }
                self.out.push_str("val ");
                self.out.push_str(name);
                self.write_type_annotation(type_ann);
                self.out.push_str(" = ");
                self.write_expression(value, 0);
            }
            Statement::Function {
                name,
                params,
                body,
                return_type,
                is_pub,
            } => {
                if *is_pub {
                    self.out.push_str("pub ");
                }
                self.write_function_header(name, params, return_type);
                self.out.push(' ');
                self.write_block_expression(body);
            }
            Statement::Return(Some(expr)) => {
                self.out.push_str("rn ");
                self.write_expression(expr, 0);
            }
            Statement::Return(None) => {
                self.out.push_str("rn");
            }
            Statement::ObjDef {
                name,
                fields,
                methods,
                uses,
                is_pub,
            } => {
                if *is_pub {
                    self.out.push_str("pub ");
                }
                self.out.push_str("obj ");
                self.out.push_str(name);
                self.out.push_str(" {\n");
                self.indent += 1;

                let mut wrote_group = false;

                if !uses.is_empty() {
                    for (i, used) in uses.iter().enumerate() {
                        if i > 0 {
                            self.out.push('\n');
                        }
                        self.write_indent();
                        self.out.push_str("use ");
                        self.out.push_str(&used.join("."));
                    }
                    wrote_group = true;
                }

                if !fields.is_empty() {
                    if wrote_group {
                        self.out.push('\n');
                        self.out.push('\n');
                    }

                    for (i, field) in fields.iter().enumerate() {
                        if i > 0 {
                            self.out.push('\n');
                        }
                        self.emit_leading_comments(field.span.start);
                        self.write_indent();
                        if field.is_pub {
                            self.out.push_str("pub ");
                        }
                        self.out.push_str(&field.name);
                        self.out.push_str(": ");
                        self.write_type_name(&field.type_ann);
                    }
                    wrote_group = true;
                }

                if !methods.is_empty() {
                    if wrote_group {
                        self.out.push('\n');
                        self.out.push('\n');
                    }

                    for (i, method) in methods.iter().enumerate() {
                        if i > 0 {
                            self.out.push('\n');
                            self.out.push('\n');
                        }
                        self.emit_leading_comments(method.span.start);
                        self.write_object_method(method);
                    }
                }

                // Comments between last method/field and closing brace
                self.emit_remaining_comments(stmt.span.end);
                self.indent -= 1;
                self.out.push('\n');
                self.write_indent();
                self.out.push('}');
            }
            Statement::FieldAssignment {
                object,
                field,
                value,
            } => {
                self.write_expression(object, PREC_POSTFIX);
                self.out.push('.');
                self.out.push_str(field);
                self.out.push_str(" = ");
                self.write_expression(value, 0);
            }
            Statement::Assignment { name, value } => {
                self.out.push_str(name);
                self.out.push_str(" = ");
                self.write_expression(value, 0);
            }
            Statement::If {
                condition,
                body,
                else_body,
            } => self.write_if_chain(condition, body, else_body.as_ref(), false),
            Statement::Unless {
                condition,
                body,
                else_body,
            } => self.write_unless(condition, body, else_body.as_ref()),
            Statement::While { condition, body } => {
                self.out.push_str("while ");
                self.write_expression(condition, 0);
                self.out.push(' ');
                self.write_block_expression(body);
            }
            Statement::For {
                name,
                iterable,
                body,
            } => {
                self.out.push_str("for ");
                self.out.push_str(name);
                self.out.push_str(" in ");
                self.write_expression(iterable, 0);
                self.out.push(' ');
                self.write_block_expression(body);
            }
            Statement::Break => self.out.push_str("break"),
            Statement::Continue => self.out.push_str("continue"),
            Statement::Expression(expr) => self.write_expression(expr, 0),
            Statement::Import { path } => {
                self.out.push_str("import ");
                self.out.push_str(&path.join("."));
            }
            Statement::IfLet {
                name,
                value,
                body,
                else_body,
            } => {
                self.out.push_str("if let ");
                self.out.push_str(name);
                self.out.push_str(" = ");
                self.write_expression(value, 0);
                self.out.push(' ');
                self.write_block_expression(body);
                if let Some(else_body) = else_body {
                    self.out.push(' ');
                    self.out.push_str("else ");
                    self.write_block_expression(else_body);
                }
            }
        }
    }

    fn write_object_method(&mut self, method: &ObjMethod) {
        self.write_indent();
        if method.is_pub {
            self.out.push_str("pub ");
        }
        self.write_function_header(&method.name, &method.params, &method.return_type);
        if let Some(body) = &method.body {
            self.out.push(' ');
            self.write_block_expression(body);
        }
    }

    fn write_function_header(
        &mut self,
        name: &str,
        params: &[(String, Option<TypeAnnotation>)],
        return_type: &Option<TypeAnnotation>,
    ) {
        self.out.push_str("fn ");
        self.out.push_str(name);
        self.out.push('(');
        for (i, (param_name, ann)) in params.iter().enumerate() {
            if i > 0 {
                self.out.push_str(", ");
            }
            self.out.push_str(param_name);
            self.write_type_annotation(ann);
        }
        self.out.push(')');
        if let Some(ann) = return_type {
            self.out.push_str(" -> ");
            self.write_type_name(ann);
        }
    }

    fn write_block_expression(&mut self, expr: &Spanned<Expression>) {
        match &expr.node {
            Expression::Block(stmts) => self.write_block_statements(stmts, expr.span.end),
            _ => {
                self.out.push_str("{\n");
                self.indent += 1;
                self.write_indent();
                self.write_expression(expr, 0);
                self.indent -= 1;
                self.out.push('\n');
                self.write_indent();
                self.out.push('}');
            }
        }
    }

    fn write_block_statements(&mut self, stmts: &[Spanned<Statement>], block_end: usize) {
        self.out.push_str("{\n");
        self.indent += 1;
        for (i, stmt) in stmts.iter().enumerate() {
            if i > 0 {
                self.out.push('\n');
                // Preserve blank lines from the original source.
                let prev_end = stmts[i - 1].span.end;
                let next_start = self.next_content_offset(stmt.span.start);
                if has_blank_line_between(&self.source, prev_end, next_start) {
                    self.ensure_blank_line();
                }
            }
            self.emit_leading_comments(stmt.span.start);
            self.write_statement(stmt);
            self.emit_trailing_comment(stmt.span.end);
            self.last_source_end = stmt.span.end;
        }
        // Comments between last statement and closing brace
        self.emit_remaining_comments(block_end);
        self.indent -= 1;
        self.out.push('\n');
        self.write_indent();
        self.out.push('}');
    }

    fn write_if_chain(
        &mut self,
        condition: &Spanned<Expression>,
        body: &Spanned<Expression>,
        else_body: Option<&Spanned<Expression>>,
        is_elif: bool,
    ) {
        self.out.push_str(if is_elif { "elif " } else { "if " });
        self.write_expression(condition, 0);
        self.out.push(' ');
        self.write_block_expression(body);

        if let Some(else_body) = else_body {
            if let Some(nested_if) = extract_elif_stmt(else_body) {
                match &nested_if.node {
                    Statement::If {
                        condition,
                        body,
                        else_body,
                    } => {
                        self.out.push(' ');
                        self.write_if_chain(condition, body, else_body.as_ref(), true);
                    }
                    _ => unreachable!("extract_elif_stmt only returns if statements"),
                }
            } else {
                self.out.push(' ');
                self.out.push_str("else ");
                self.write_block_expression(else_body);
            }
        }
    }

    fn write_unless(
        &mut self,
        condition: &Spanned<Expression>,
        body: &Spanned<Expression>,
        else_body: Option<&Spanned<Expression>>,
    ) {
        self.out.push_str("unless ");
        self.write_expression(condition, 0);
        self.out.push(' ');
        self.write_block_expression(body);

        if let Some(else_body) = else_body {
            self.out.push(' ');
            self.out.push_str("else ");
            self.write_block_expression(else_body);
        }
    }

    fn write_expression(&mut self, expr: &Spanned<Expression>, parent_prec: u8) {
        let prec = expression_precedence(&expr.node);
        let needs_parens = prec < parent_prec;
        if needs_parens {
            self.out.push('(');
        }

        match &expr.node {
            Expression::True => self.out.push_str("true"),
            Expression::False => self.out.push_str("false"),
            Expression::Float(n) => {
                let s = n.to_string();
                if s.contains('.') {
                    self.out.push_str(&s);
                } else {
                    let _ = write!(self.out, "{s}.0");
                }
            }
            Expression::Int(n) => {
                let _ = write!(self.out, "{n}");
            }
            Expression::String(s) => {
                self.out.push('"');
                self.out.push_str(s);
                self.out.push('"');
            }
            Expression::Ident(name) => self.out.push_str(name),
            Expression::ObjLiteral { type_name, fields } => {
                self.out.push_str(&type_name.join("."));
                self.out.push_str(" { ");
                for (i, (name, value)) in fields.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    self.out.push_str(name);
                    self.out.push_str(": ");
                    self.write_expression(value, 0);
                }
                self.out.push_str(" }");
            }
            Expression::FieldAccess { object, field } => {
                self.write_expression(object, PREC_POSTFIX);
                self.out.push('.');
                self.out.push_str(field);
            }
            Expression::MethodCall {
                object,
                method,
                args,
            } => {
                self.write_expression(object, PREC_POSTFIX);
                self.out.push('.');
                self.out.push_str(method);
                self.out.push('(');
                self.write_args(args);
                self.out.push(')');
            }
            Expression::BinaryOp { op, left, right } => {
                let prec = binary_precedence(op);
                self.write_expression(left, prec);
                self.out.push(' ');
                self.out.push_str(binary_op_text(op));
                self.out.push(' ');
                self.write_expression(right, prec + 1);
            }
            Expression::Range {
                start,
                end,
                inclusive,
            } => {
                self.write_expression(start, PREC_RANGE);
                self.out.push_str(if *inclusive { "..=" } else { ".." });
                self.write_expression(end, PREC_RANGE + 1);
            }
            Expression::UnaryOp { op, expr } => {
                self.out.push_str(match op {
                    UnaryOp::Not => "not ",
                    UnaryOp::Negate => "-",
                });
                self.write_expression(expr, PREC_UNARY);
            }
            Expression::Call { name, args } => {
                self.out.push_str(name);
                self.out.push('(');
                self.write_args(args);
                self.out.push(')');
            }
            Expression::Block(stmts) => self.write_block_statements(stmts, expr.span.end),
            Expression::Nil => self.out.push_str("nil"),
            Expression::Try(inner) => {
                self.out.push_str("try ");
                self.write_expression(inner, PREC_UNARY);
            }
            Expression::UnwrapError(inner) => {
                self.out.push('!');
                self.write_expression(inner, PREC_UNARY);
            }
            Expression::Coalesce { left, right } => {
                self.write_expression(left, 1);
                self.out.push_str(" orelse ");
                self.write_expression(right, 2);
            }
            Expression::StringInterp(parts) => {
                self.out.push('"');
                for part in parts {
                    match part {
                        StringPart::Literal(s) => self.out.push_str(s),
                        StringPart::Interp(expr) => {
                            self.out.push('{');
                            self.write_expression(expr, 0);
                            self.out.push('}');
                        }
                    }
                }
                self.out.push('"');
            }
        }

        if needs_parens {
            self.out.push(')');
        }
    }

    fn write_args(&mut self, args: &[Spanned<Expression>]) {
        for (i, arg) in args.iter().enumerate() {
            if i > 0 {
                self.out.push_str(", ");
            }
            self.write_expression(arg, 0);
        }
    }

    fn write_type_annotation(&mut self, ann: &Option<TypeAnnotation>) {
        if let Some(ann) = ann {
            self.out.push_str(": ");
            self.write_type_name(ann);
        }
    }

    fn write_type_name(&mut self, ann: &TypeAnnotation) {
        match ann {
            TypeAnnotation::Named(path) => self.out.push_str(&path.join(".")),
            TypeAnnotation::Nillable(inner) => {
                self.write_type_name(inner);
                self.out.push('?');
            }
            TypeAnnotation::ErrorUnion(inner) => {
                self.out.push('!');
                self.write_type_name(inner);
            }
        }
    }

    fn write_indent(&mut self) {
        for _ in 0..self.indent {
            self.out.push_str("    ");
        }
    }
}

const PREC_RANGE: u8 = 3;
const PREC_UNARY: u8 = 6;
const PREC_POSTFIX: u8 = 7;

fn expression_precedence(expr: &Expression) -> u8 {
    match expr {
        Expression::Coalesce { .. } => 1,
        Expression::BinaryOp { op, .. } => binary_precedence(op),
        Expression::Range { .. } => PREC_RANGE,
        Expression::UnaryOp { .. } => PREC_UNARY,
        Expression::FieldAccess { .. }
        | Expression::MethodCall { .. }
        | Expression::Call { .. } => PREC_POSTFIX,
        _ => 8,
    }
}

fn binary_precedence(op: &BinOp) -> u8 {
    match op {
        BinOp::Or => 1,
        BinOp::And => 2,
        BinOp::Equals
        | BinOp::NotEquals
        | BinOp::LessThan
        | BinOp::GreaterThan
        | BinOp::LessThanEquals
        | BinOp::GreaterThanEquals => 4,
        BinOp::Add | BinOp::Sub => 5,
        BinOp::Mul | BinOp::Div => 6,
    }
}

fn binary_op_text(op: &BinOp) -> &'static str {
    match op {
        BinOp::Equals => "==",
        BinOp::NotEquals => "!=",
        BinOp::LessThan => "<",
        BinOp::GreaterThan => ">",
        BinOp::LessThanEquals => "<=",
        BinOp::GreaterThanEquals => ">=",
        BinOp::And => "and",
        BinOp::Or => "or",
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
    }
}

fn needs_blank_line_between(prev: &Statement, next: &Statement) -> bool {
    statement_is_declaration(prev) || statement_is_declaration(next)
}

/// Returns `true` if the source text between byte offsets `from` and
/// `to` contains at least one blank line (two newlines with only
/// whitespace between them).
fn has_blank_line_between(source: &str, from: usize, to: usize) -> bool {
    if from >= to || from >= source.len() {
        return false;
    }
    let end = to.min(source.len());
    let slice = &source[from..end];
    // A blank line means two \n with only spaces/tabs between them.
    let mut saw_newline = false;
    for ch in slice.chars() {
        if ch == '\n' {
            if saw_newline {
                return true;
            }
            saw_newline = true;
        } else if !ch.is_ascii_whitespace() {
            saw_newline = false;
        }
    }
    false
}

fn statement_is_declaration(stmt: &Statement) -> bool {
    matches!(stmt, Statement::Function { .. } | Statement::ObjDef { .. })
}

fn extract_elif_stmt(expr: &Spanned<Expression>) -> Option<&Spanned<Statement>> {
    match expr {
        Spanned {
            node: Expression::Block(stmts),
            ..
        } if stmts.len() == 1 && matches!(stmts[0].node, Statement::If { .. }) => Some(&stmts[0]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_leading_comments() {
        let source = "// This is a greeting\nlet x = 5";
        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, "// This is a greeting\nlet x = 5\n");
    }

    #[test]
    fn preserves_trailing_comments() {
        let source = "let x = 5 // important value";
        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, "let x = 5  // important value\n");
    }

    #[test]
    fn preserves_section_comments() {
        let source = "// --- section 1 ---\nlet x = 5\n\n// --- section 2 ---\nlet y = 10";
        let formatted = format_source(source).unwrap();
        assert_eq!(
            formatted,
            "// --- section 1 ---\nlet x = 5\n\n// --- section 2 ---\nlet y = 10\n"
        );
    }

    #[test]
    fn preserves_comments_in_blocks() {
        let source = "fn foo() {\n    // inside\n    let x = 5\n}";
        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, "fn foo() {\n    // inside\n    let x = 5\n}\n");
    }

    #[test]
    fn formats_function_and_if() {
        let source = "fn add(a:int,b:int)->int{if a>b {rn a}else{rn b}}";
        let formatted = format_source(source).unwrap();

        assert_eq!(
            formatted,
            "fn add(a: int, b: int) -> int {\n    if a > b {\n        rn a\n    } else {\n        rn b\n    }\n}\n"
        );
    }

    #[test]
    fn formats_objects_and_static_methods() {
        let source = "obj Vec2 {\nx:int\ny:int\nfn zero()->Vec2{rn Vec2{x:0,y:0}}\n}";
        let formatted = format_source(source).unwrap();

        assert_eq!(
            formatted,
            "obj Vec2 {\n    x: int\n    y: int\n\n    fn zero() -> Vec2 {\n        rn Vec2 { x: 0, y: 0 }\n    }\n}\n"
        );
    }

    #[test]
    fn formats_for_and_ranges() {
        let source = "for i in 0..=3{print(i)}";
        let formatted = format_source(source).unwrap();

        assert_eq!(formatted, "for i in 0..=3 {\n    print(i)\n}\n");
    }

    #[test]
    fn formats_if_let() {
        let source = "if let x = maybe_val() { print(x) }";
        let formatted = format_source(source).unwrap();

        assert_eq!(formatted, "if let x = maybe_val() {\n    print(x)\n}\n");
    }

    #[test]
    fn formats_if_let_with_else() {
        let source = "if let x=maybe_val(){print(x)}else{print(0)}";
        let formatted = format_source(source).unwrap();

        assert_eq!(
            formatted,
            "if let x = maybe_val() {\n    print(x)\n} else {\n    print(0)\n}\n"
        );
    }

    #[test]
    fn preserves_elif_syntax() {
        let source = "if a { print(1) } elif b { print(2) } else { print(3) }";
        let formatted = format_source(source).unwrap();

        assert_eq!(
            formatted,
            "if a {\n    print(1)\n} elif b {\n    print(2)\n} else {\n    print(3)\n}\n"
        );
    }

    #[test]
    fn formats_unless() {
        let source = "unless ready{print(0)}";
        let formatted = format_source(source).unwrap();

        assert_eq!(formatted, "unless ready {\n    print(0)\n}\n");
    }

    #[test]
    fn formats_unless_with_else() {
        let source = "unless ready{print(0)}else{print(1)}";
        let formatted = format_source(source).unwrap();

        assert_eq!(
            formatted,
            "unless ready {\n    print(0)\n} else {\n    print(1)\n}\n"
        );
    }
}
