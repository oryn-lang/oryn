//! Module file resolution and loading.
//!
//! Oryn uses a Zig-style module system: every project has a `package.on`
//! file at the root, and `import foo.bar.baz` resolves to
//! `<project root>/foo/bar/baz.on`. This module provides the small set
//! of helpers used by [`crate::Chunk::compile_file`] to walk the file
//! system, locate modules, and read their source.

use std::path::{Path, PathBuf};

use crate::OrynError;

/// Walk up from `start` looking for a directory containing `package.on`.
/// Returns the project root when found, or `None` if no marker exists in
/// any ancestor. Used to anchor every `import` to a single project root
/// regardless of where the entry file lives within the tree.
pub(crate) fn find_project_root(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();

    loop {
        if current.join("package.on").exists() {
            return Some(current);
        }

        if let Some(parent) = current.parent() {
            current = parent.to_path_buf();
        } else {
            return None;
        }
    }
}

/// Convert a dotted import path into a file path relative to `root`.
/// `["math", "nested", "lib"]` becomes `<root>/math/nested/lib.on`.
/// Uses `PathBuf::push` so it works across platforms with native
/// directory separators.
pub(crate) fn resolve_import(root: &Path, path: &[String]) -> PathBuf {
    let mut result = root.to_path_buf();

    for segment in path {
        result.push(segment);
    }

    result.set_extension("on");

    result
}

/// Resolve a module path and read its source from disk. Returns an
/// [`OrynError::Module`] when the file does not exist or cannot be read.
pub(crate) fn load_module(root: &Path, path: &[String]) -> Result<String, OrynError> {
    let path = resolve_import(root, path);

    let content = std::fs::read_to_string(&path).map_err(|e| OrynError::Module {
        path: path.to_string_lossy().into_owned(),
        message: e.to_string(),
    })?;

    Ok(content)
}
