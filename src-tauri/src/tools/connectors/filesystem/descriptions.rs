//! What the model is told about the filesystem tools, naming the folders the user allowed.

use super::*;

pub(super) fn display_root_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

pub(super) fn format_allowed_roots_block(roots: &[String]) -> String {
    if roots.is_empty() {
        return "No allowed directories are configured yet.".into();
    }
    let listed = roots
        .iter()
        .map(|root| format!("- {root}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!("Allowed directories:\n{listed}")
}

pub(super) fn fs_search_description(roots: &[String]) -> String {
    format!(
        "Search for files by name pattern or content. {}\n\
         Use `root` to limit the search to a subdirectory of one allowed root. \
         Omit `root` to search all allowed directories. \
         Paths may be absolute (under an allowed root) or relative to the first allowed root.",
        format_allowed_roots_block(roots)
    )
}

pub(super) fn fs_read_description(roots: &[String]) -> String {
    format!(
        "Open and read a local file on the user's computer, including files found by fs_search. \
         Use this tool for local files, rather than knowledge-base lookup or analysis tools, which cannot open local filesystem paths. \
         Supports text, code, PDF, documents, CSV, spreadsheets and images, up to 5 MiB per file. \
         Set `format` to `file` and omit `start` and `end` for whole-file processing. \
         Returns readable text or image content with an image_id, depending on the file type. \
         Local files do not need to be uploaded to the chat first. \
         Byte ranges and `format: raw` return raw content and should be used only for text files. {}\n\
         The `path` must be absolute under one of these roots, or relative to the first allowed root.",
        format_allowed_roots_block(roots)
    )
}

pub(super) fn fs_search_root_property_description(roots: &[String]) -> String {
    if roots.is_empty() {
        return "Sub-path within an allowed root. Omit to search all roots.".into();
    }
    format!(
        "Optional subdirectory to search within. Must be under one of: {}. Omit to search all allowed directories.",
        roots.join(", ")
    )
}

pub(super) fn fs_read_path_property_description(roots: &[String]) -> String {
    if roots.is_empty() {
        return "File path under an allowed root.".into();
    }
    format!(
        "File path under one of the allowed directories: {}. \
         Absolute paths must stay within an allowed root; otherwise they are resolved relative to {}.",
        roots.join(", "),
        roots[0]
    )
}

pub(super) fn fs_write_description(roots: &[String]) -> String {
    format!(
        "Create or overwrite a text file under an allowed directory. {}\n\
         Existing files are replaced entirely. Parent directories must already exist.",
        format_allowed_roots_block(roots)
    )
}

pub(super) fn fs_edit_description(roots: &[String]) -> String {
    format!(
        "Edit a text file by replacing strings. {}\n\
         Fails if `old_string` is not found unless `replace_all` is true and every occurrence is replaced.",
        format_allowed_roots_block(roots)
    )
}
