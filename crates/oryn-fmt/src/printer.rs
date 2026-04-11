use std::fmt::Write as _;

use oryn::{
    BinOp, Expression, ObjMethod, Param, Spanned, Statement, StringPart, TypeAnnotation, UnaryOp,
};

use crate::comments::CommentAttachments;
use crate::session::{ParsedSource, has_blank_line_between};

pub(crate) struct Formatter<'a> {
    parsed: &'a ParsedSource,
    attachments: CommentAttachments,
    out: String,
    indent: usize,
    last_source_end: usize,
}

impl<'a> Formatter<'a> {
    pub(crate) fn new(parsed: &'a ParsedSource, attachments: CommentAttachments) -> Self {
        Self {
            parsed,
            attachments,
            out: String::new(),
            indent: 0,
            last_source_end: 0,
        }
    }

    pub(crate) fn finish(self) -> String {
        self.out
    }

    pub(crate) fn write_program(&mut self) {
        self.write_statement_list(&self.parsed.stmts, 0, true);
        self.emit_dangling_comments(self.parsed.source.len());

        if !self.out.is_empty() && !self.out.ends_with('\n') {
            self.out.push('\n');
        }
    }

    fn write_statement_list(
        &mut self,
        stmts: &[Spanned<Statement>],
        container_start: usize,
        insert_decl_spacing: bool,
    ) {
        self.last_source_end = container_start;

        for (i, stmt) in stmts.iter().enumerate() {
            if i > 0 {
                self.out.push('\n');
                let next_start = self.next_content_offset(stmt.span.start);
                if (insert_decl_spacing && needs_blank_line_between(&stmts[i - 1].node, &stmt.node))
                    || has_blank_line_between(&self.parsed.source, self.last_source_end, next_start)
                {
                    self.ensure_blank_line();
                }
            }

            self.emit_leading_comments(stmt.span.start);
            self.write_statement(stmt);
            self.emit_trailing_comment(stmt.span.end);
            self.last_source_end = self.current_item_end(stmt.span.end);
        }
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
            Statement::Return(None) => self.out.push_str("rn"),
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
                self.last_source_end = stmt.span.start;

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
                        self.ensure_blank_line();
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
                        self.last_source_end = field.span.end;
                    }
                    wrote_group = true;
                }

                if !methods.is_empty() {
                    if wrote_group {
                        self.ensure_blank_line();
                    }

                    for (i, method) in methods.iter().enumerate() {
                        if i > 0 {
                            self.ensure_blank_line();
                        }
                        self.emit_leading_comments(method.span.start);
                        self.write_object_method(method);
                        self.last_source_end = method.span.end;
                    }
                }

                self.emit_dangling_comments(stmt.span.end);
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
            Statement::IndexAssignment {
                object,
                index,
                value,
            } => {
                self.write_expression(object, PREC_POSTFIX);
                self.out.push('[');
                self.write_expression(index, 0);
                self.out.push(']');
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
            Statement::Test { name, body } => {
                self.out.push_str("test \"");
                self.out.push_str(name);
                self.out.push_str("\" ");
                self.write_block_expression(body);
            }
            Statement::Assert { condition } => {
                self.out.push_str("assert(");
                self.write_expression(condition, 0);
                self.out.push(')');
            }
        }
    }

    fn write_object_method(&mut self, method: &ObjMethod) {
        self.write_indent();
        if method.is_pub {
            self.out.push_str("pub ");
        }
        // `mut self` is printed by `write_function_header` via the
        // per-parameter `mut` prefix, alongside any `mut x: T`
        // non-self parameters. There's no method-level `mut`
        // keyword to emit.
        self.write_function_header(&method.name, &method.params, &method.return_type);
        if let Some(body) = &method.body {
            self.out.push(' ');
            self.write_block_expression(body);
        }
    }

    fn write_function_header(
        &mut self,
        name: &str,
        params: &[Param],
        return_type: &Option<TypeAnnotation>,
    ) {
        self.out.push_str("fn ");
        self.out.push_str(name);
        self.out.push('(');
        for (i, param) in params.iter().enumerate() {
            if i > 0 {
                self.out.push_str(", ");
            }
            if param.is_mut {
                self.out.push_str("mut ");
            }
            self.out.push_str(&param.name);
            self.write_type_annotation(&param.type_ann);
        }
        self.out.push(')');
        if let Some(ann) = return_type {
            self.out.push_str(" -> ");
            self.write_type_name(ann);
        }
    }

    fn write_block_expression(&mut self, expr: &Spanned<Expression>) {
        match &expr.node {
            Expression::Block(stmts) => {
                self.write_block_statements(stmts, expr.span.start, expr.span.end)
            }
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

    fn write_block_statements(
        &mut self,
        stmts: &[Spanned<Statement>],
        block_start: usize,
        block_end: usize,
    ) {
        self.out.push_str("{\n");
        self.indent += 1;
        self.write_statement_list(stmts, block_start, false);
        self.emit_dangling_comments(block_end);
        self.indent -= 1;
        if !stmts.is_empty() || !self.attachments.dangling(block_end).is_empty() {
            self.out.push('\n');
        }
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
                if let Statement::If {
                    condition,
                    body,
                    else_body,
                } = &nested_if.node
                {
                    self.out.push(' ');
                    self.write_if_chain(condition, body, else_body.as_ref(), true);
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
            Expression::Block(stmts) => {
                self.write_block_statements(stmts, expr.span.start, expr.span.end)
            }
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
            Expression::ListLiteral(elements) => {
                self.out.push('[');
                for (i, element) in elements.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    self.write_expression(element, 0);
                }
                self.out.push(']');
            }
            Expression::MapLiteral(entries) => {
                self.out.push('{');
                for (i, (key, value)) in entries.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    self.write_expression(key, 0);
                    self.out.push_str(": ");
                    self.write_expression(value, 0);
                }
                self.out.push('}');
            }
            Expression::Index { object, index } => {
                self.write_expression(object, PREC_POSTFIX);
                self.out.push('[');
                self.write_expression(index, 0);
                self.out.push(']');
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
            TypeAnnotation::List(inner) => {
                self.out.push('[');
                self.write_type_name(inner);
                self.out.push(']');
            }
            TypeAnnotation::Map(key, value) => {
                self.out.push('{');
                self.write_type_name(key);
                self.out.push_str(": ");
                self.write_type_name(value);
                self.out.push('}');
            }
        }
    }

    fn emit_leading_comments(&mut self, anchor: usize) {
        let comments = self.attachments.leading(anchor).to_vec();
        if comments.is_empty() {
            return;
        }

        let mut prev_end = self.last_source_end;
        for comment in comments {
            if !self.out.is_empty() && !self.out.ends_with('\n') {
                self.out.push('\n');
            }
            if prev_end > 0 && has_blank_line_between(&self.parsed.source, prev_end, comment.offset)
            {
                self.ensure_blank_line();
            }
            self.write_indent();
            self.out.push_str(&comment.text);
            self.out.push('\n');
            prev_end = comment.end;
        }

        if has_blank_line_between(&self.parsed.source, prev_end, anchor) {
            self.ensure_blank_line();
        }
        self.last_source_end = prev_end;
    }

    fn emit_trailing_comment(&mut self, anchor: usize) {
        if let Some(comment) = self.attachments.trailing(anchor) {
            self.out.push_str("  ");
            self.out.push_str(&comment.text);
        }
    }

    fn emit_dangling_comments(&mut self, anchor: usize) {
        let comments = self.attachments.dangling(anchor).to_vec();
        if comments.is_empty() {
            return;
        }

        let mut prev_end = self.last_source_end;
        for comment in comments {
            if !self.out.is_empty() && !self.out.ends_with('\n') {
                self.out.push('\n');
            }
            if prev_end > 0 && has_blank_line_between(&self.parsed.source, prev_end, comment.offset)
            {
                self.ensure_blank_line();
            }
            self.write_indent();
            self.out.push_str(&comment.text);
            self.out.push('\n');
            prev_end = comment.end;
        }

        self.last_source_end = prev_end;
    }

    fn current_item_end(&self, anchor: usize) -> usize {
        self.attachments
            .trailing(anchor)
            .map(|comment| comment.end)
            .unwrap_or(anchor)
    }

    fn next_content_offset(&self, stmt_start: usize) -> usize {
        self.attachments
            .leading(stmt_start)
            .first()
            .map(|comment| comment.offset)
            .unwrap_or(stmt_start)
    }

    fn ensure_blank_line(&mut self) {
        if !self.out.ends_with("\n\n") {
            if !self.out.ends_with('\n') {
                self.out.push('\n');
            }
            self.out.push('\n');
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
