//! Higher-order list methods. These can't be implemented as native
//! Rust functions because the VM dispatch loop is not re-entrant —
//! a native body has no safe way to call back into the VM to invoke
//! a user closure on each list element. So instead of registering
//! them in [`super::NativeRegistry`], the compiler consults this
//! intrinsic table during method-call dispatch and emits bytecode
//! directly when there's a hit.
//!
//! Each entry pairs a method name with two pieces:
//! - a `signature` computer that, given the receiver type and the
//!   already-resolved arg types, produces the method's full signature
//!   (params + return type).
//! - an `emitter` callback that the compiler invokes at the call site
//!   to write the desugared bytecode (a `for` loop with `CallValue`
//!   per element, plus whatever bookkeeping the method needs).
//!
//! The actual emitter implementations live in
//! `crates/oryn/src/compiler/expr.rs` because they need access to
//! the compiler's private helpers (`emit`, `compile_expr`, etc.).
//! This file just lists the signatures and the metadata the compiler
//! consults to dispatch into them.

use crate::compiler::types::ResolvedType;

/// One higher-order method definition.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Intrinsic {
    pub(crate) name: &'static str,
    /// The kind tag — the compiler dispatches on this in `compile_expr`
    /// to pick the right emitter.
    pub(crate) kind: IntrinsicKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IntrinsicKind {
    Map,
    Filter,
    Fold,
    Each,
    Find,
    Any,
    All,
    SortBy,
}

/// Element type of a list receiver, or `Unknown` if the receiver
/// isn't a list.
fn elem_of(receiver: &ResolvedType) -> ResolvedType {
    match receiver {
        ResolvedType::List(inner) => (**inner).clone(),
        _ => ResolvedType::Unknown,
    }
}

/// Compute the signature of a higher-order list method given the
/// receiver type and the actual argument types. Returns the
/// `(param_types, return_type)` that the compiler should type-check
/// against. Errors out for shape mismatches (wrong arity, callback
/// not a function, etc.).
pub(crate) fn signature(
    kind: IntrinsicKind,
    receiver: &ResolvedType,
    args: &[ResolvedType],
) -> Result<(Vec<ResolvedType>, ResolvedType), String> {
    let elem = elem_of(receiver);
    match kind {
        IntrinsicKind::Map => {
            // map(fn(T) -> U) -> [U]
            if args.len() != 1 {
                return Err(format!("map takes 1 argument, got {}", args.len()));
            }
            let result_elem = match &args[0] {
                ResolvedType::Function { return_type, .. } => (**return_type).clone(),
                ResolvedType::Unknown => ResolvedType::Unknown,
                other => {
                    return Err(format!(
                        "map expects a function argument, got `{}`",
                        other.display_name()
                    ));
                }
            };
            Ok((
                vec![ResolvedType::Function {
                    params: vec![elem],
                    return_type: Box::new(result_elem.clone()),
                }],
                ResolvedType::List(Box::new(result_elem)),
            ))
        }
        IntrinsicKind::Filter => {
            // filter(fn(T) -> bool) -> [T]
            if args.len() != 1 {
                return Err(format!("filter takes 1 argument, got {}", args.len()));
            }
            Ok((
                vec![ResolvedType::Function {
                    params: vec![elem.clone()],
                    return_type: Box::new(ResolvedType::Bool),
                }],
                ResolvedType::List(Box::new(elem)),
            ))
        }
        IntrinsicKind::Fold => {
            // fold(U, fn(U, T) -> U) -> U
            if args.len() != 2 {
                return Err(format!("fold takes 2 arguments, got {}", args.len()));
            }
            let acc_ty = args[0].clone();
            Ok((
                vec![
                    acc_ty.clone(),
                    ResolvedType::Function {
                        params: vec![acc_ty.clone(), elem],
                        return_type: Box::new(acc_ty.clone()),
                    },
                ],
                acc_ty,
            ))
        }
        IntrinsicKind::Each => {
            // each(fn(T)) -> nil
            if args.len() != 1 {
                return Err(format!("each takes 1 argument, got {}", args.len()));
            }
            Ok((
                vec![ResolvedType::Function {
                    params: vec![elem],
                    return_type: Box::new(ResolvedType::Nil),
                }],
                ResolvedType::Nil,
            ))
        }
        IntrinsicKind::Find => {
            // find(fn(T) -> bool) -> maybe T
            if args.len() != 1 {
                return Err(format!("find takes 1 argument, got {}", args.len()));
            }
            Ok((
                vec![ResolvedType::Function {
                    params: vec![elem.clone()],
                    return_type: Box::new(ResolvedType::Bool),
                }],
                ResolvedType::Nillable(Box::new(elem)),
            ))
        }
        IntrinsicKind::Any => {
            // any(fn(T) -> bool) -> bool
            if args.len() != 1 {
                return Err(format!("any takes 1 argument, got {}", args.len()));
            }
            Ok((
                vec![ResolvedType::Function {
                    params: vec![elem],
                    return_type: Box::new(ResolvedType::Bool),
                }],
                ResolvedType::Bool,
            ))
        }
        IntrinsicKind::All => {
            // all(fn(T) -> bool) -> bool
            if args.len() != 1 {
                return Err(format!("all takes 1 argument, got {}", args.len()));
            }
            Ok((
                vec![ResolvedType::Function {
                    params: vec![elem],
                    return_type: Box::new(ResolvedType::Bool),
                }],
                ResolvedType::Bool,
            ))
        }
        IntrinsicKind::SortBy => {
            // sort_by(fn(T, T) -> int) -> nil  — mutating
            if args.len() != 1 {
                return Err(format!("sort_by takes 1 argument, got {}", args.len()));
            }
            Ok((
                vec![ResolvedType::Function {
                    params: vec![elem.clone(), elem],
                    return_type: Box::new(ResolvedType::Int),
                }],
                ResolvedType::Nil,
            ))
        }
    }
}

/// Look up an intrinsic by method name on a list receiver. Returns
/// `None` for non-list receivers or unknown method names.
pub(crate) fn lookup(receiver: &ResolvedType, name: &str) -> Option<Intrinsic> {
    if !matches!(receiver, ResolvedType::List(_)) {
        return None;
    }
    let (canonical, kind): (&'static str, IntrinsicKind) = match name {
        "map" => ("map", IntrinsicKind::Map),
        "filter" => ("filter", IntrinsicKind::Filter),
        "fold" => ("fold", IntrinsicKind::Fold),
        "each" => ("each", IntrinsicKind::Each),
        "find" => ("find", IntrinsicKind::Find),
        "any" => ("any", IntrinsicKind::Any),
        "all" => ("all", IntrinsicKind::All),
        "sort_by" => ("sort_by", IntrinsicKind::SortBy),
        _ => return None,
    };
    Some(Intrinsic {
        name: canonical,
        kind,
    })
}
