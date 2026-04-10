use std::collections::HashMap;

use oryn::{Expression, ObjField, ObjMethod, Spanned, Statement, StringPart};

use crate::session::{Comment, ParsedSource, line_of};

#[derive(Default)]
pub(crate) struct CommentAttachments {
    leading: HashMap<usize, Vec<Comment>>,
    trailing: HashMap<usize, Comment>,
    dangling: HashMap<usize, Vec<Comment>>,
}

impl CommentAttachments {
    pub(crate) fn build(parsed: &ParsedSource) -> Self {
        let mut attachments = Self::default();
        attachments.attach_statement_sequence(&parsed.stmts, 0, parsed.source.len(), parsed);

        for stmt in &parsed.stmts {
            attachments.walk_statement(stmt, parsed);
        }

        attachments
    }

    pub(crate) fn leading(&self, anchor: usize) -> &[Comment] {
        self.leading.get(&anchor).map(Vec::as_slice).unwrap_or(&[])
    }

    pub(crate) fn trailing(&self, anchor: usize) -> Option<&Comment> {
        self.trailing.get(&anchor)
    }

    pub(crate) fn dangling(&self, anchor: usize) -> &[Comment] {
        self.dangling.get(&anchor).map(Vec::as_slice).unwrap_or(&[])
    }

    fn walk_statement(&mut self, stmt: &Spanned<Statement>, parsed: &ParsedSource) {
        match &stmt.node {
            Statement::Let { value, .. }
            | Statement::Val { value, .. }
            | Statement::Assignment { value, .. }
            | Statement::Expression(value)
            | Statement::Return(Some(value))
            | Statement::Assert { condition: value } => self.walk_expression(value, parsed),
            Statement::Return(None)
            | Statement::Break
            | Statement::Continue
            | Statement::Import { .. } => {}
            Statement::Function { body, .. }
            | Statement::Test { body, .. }
            | Statement::While { body, .. }
            | Statement::For { body, .. }
            | Statement::Unless { body, .. } => self.walk_expression(body, parsed),
            Statement::If {
                condition,
                body,
                else_body,
            } => {
                self.walk_expression(condition, parsed);
                self.walk_expression(body, parsed);
                if let Some(else_body) = else_body {
                    self.walk_expression(else_body, parsed);
                }
            }
            Statement::IfLet {
                value,
                body,
                else_body,
                ..
            } => {
                self.walk_expression(value, parsed);
                self.walk_expression(body, parsed);
                if let Some(else_body) = else_body {
                    self.walk_expression(else_body, parsed);
                }
            }
            Statement::FieldAssignment { object, value, .. } => {
                self.walk_expression(object, parsed);
                self.walk_expression(value, parsed);
            }
            Statement::IndexAssignment {
                object,
                index,
                value,
            } => {
                self.walk_expression(object, parsed);
                self.walk_expression(index, parsed);
                self.walk_expression(value, parsed);
            }
            Statement::ObjDef {
                fields, methods, ..
            } => {
                self.attach_object_members(stmt, fields, methods, parsed);

                for method in methods {
                    if let Some(body) = &method.body {
                        self.walk_expression(body, parsed);
                    }
                }
            }
        }
    }

    fn walk_expression(&mut self, expr: &Spanned<Expression>, parsed: &ParsedSource) {
        match &expr.node {
            Expression::ObjLiteral { fields, .. } => {
                for (_, value) in fields {
                    self.walk_expression(value, parsed);
                }
            }
            Expression::FieldAccess { object, .. }
            | Expression::UnaryOp { expr: object, .. }
            | Expression::Try(object)
            | Expression::UnwrapError(object) => self.walk_expression(object, parsed),
            Expression::MethodCall { object, args, .. } => {
                self.walk_expression(object, parsed);
                for arg in args {
                    self.walk_expression(arg, parsed);
                }
            }
            Expression::BinaryOp { left, right, .. }
            | Expression::Range {
                start: left,
                end: right,
                ..
            }
            | Expression::Coalesce { left, right } => {
                self.walk_expression(left, parsed);
                self.walk_expression(right, parsed);
            }
            Expression::Call { args, .. } => {
                for arg in args {
                    self.walk_expression(arg, parsed);
                }
            }
            Expression::Block(stmts) => {
                self.attach_statement_sequence(stmts, expr.span.start, expr.span.end, parsed);
                for stmt in stmts {
                    self.walk_statement(stmt, parsed);
                }
            }
            Expression::StringInterp(parts) => {
                for part in parts {
                    if let StringPart::Interp(expr) = part {
                        self.walk_expression(expr, parsed);
                    }
                }
            }
            Expression::ListLiteral(elements) => {
                for element in elements {
                    self.walk_expression(element, parsed);
                }
            }
            Expression::MapLiteral(entries) => {
                for (key, value) in entries {
                    self.walk_expression(key, parsed);
                    self.walk_expression(value, parsed);
                }
            }
            Expression::Index { object, index } => {
                self.walk_expression(object, parsed);
                self.walk_expression(index, parsed);
            }
            Expression::Nil
            | Expression::True
            | Expression::False
            | Expression::Float(_)
            | Expression::Int(_)
            | Expression::String(_)
            | Expression::Ident(_) => {}
        }
    }

    fn attach_object_members(
        &mut self,
        stmt: &Spanned<Statement>,
        fields: &[ObjField],
        methods: &[ObjMethod],
        parsed: &ParsedSource,
    ) {
        let mut anchors: Vec<(usize, usize)> = fields
            .iter()
            .map(|field| (field.span.start, field.span.end))
            .chain(
                methods
                    .iter()
                    .map(|method| (method.span.start, method.span.end)),
            )
            .collect();
        anchors.sort_by_key(|(start, _)| *start);

        let mut prev_end = stmt.span.start;
        for (start, end) in anchors {
            let leading = collect_standalone_comments(parsed, prev_end, start);
            if !leading.is_empty() {
                self.leading.insert(start, leading);
            }
            prev_end = end;
        }

        let dangling = collect_standalone_comments(parsed, prev_end, stmt.span.end);
        if !dangling.is_empty() {
            self.dangling.insert(stmt.span.end, dangling);
        }
    }

    fn attach_statement_sequence(
        &mut self,
        stmts: &[Spanned<Statement>],
        container_start: usize,
        container_end: usize,
        parsed: &ParsedSource,
    ) {
        let mut prev_end = container_start;

        for stmt in stmts {
            let leading = collect_standalone_comments(parsed, prev_end, stmt.span.start);
            if !leading.is_empty() {
                self.leading.insert(stmt.span.start, leading);
            }

            if let Some(comment) = find_trailing_comment(parsed, stmt.span.end) {
                self.trailing.insert(stmt.span.end, comment.clone());
                prev_end = comment.end;
            } else {
                prev_end = stmt.span.end;
            }
        }

        let dangling = collect_standalone_comments(parsed, prev_end, container_end);
        if !dangling.is_empty() {
            self.dangling.insert(container_end, dangling);
        }
    }
}

fn collect_standalone_comments(parsed: &ParsedSource, from: usize, to: usize) -> Vec<Comment> {
    parsed
        .comments
        .iter()
        .filter(|comment| comment.standalone && comment.offset >= from && comment.offset < to)
        .cloned()
        .collect()
}

fn find_trailing_comment(parsed: &ParsedSource, stmt_end: usize) -> Option<&Comment> {
    let code_line = line_of(stmt_end.saturating_sub(1), &parsed.line_starts);
    parsed.comments.iter().find(|comment| {
        !comment.standalone
            && comment.offset >= stmt_end
            && line_of(comment.offset, &parsed.line_starts) == code_line
    })
}
