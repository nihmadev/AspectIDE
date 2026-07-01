#![deny(clippy::pedantic)]
#![deny(clippy::nursery)]
#![allow(clippy::missing_errors_doc)]

use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use chrono::Utc;
use lux_core::{
    file_view_descriptor_for_path, monaco_language_id_for_path, AppError, AppResult, BufferId,
    DocumentSnapshot, TextEdit,
};

#[derive(Default)]
pub struct DocumentStore {
    documents: HashMap<BufferId, DocumentSnapshot>,
    by_path: HashMap<PathBuf, BufferId>,
    untitled_counter: u64,
}

#[derive(Debug, Clone)]
pub struct DocumentSavePayload {
    pub path: Option<PathBuf>,
    pub suggested_name: String,
    pub text: String,
    pub version: u64,
    pub is_untitled: bool,
}

#[derive(Debug, Clone)]
pub struct DocumentPathAttachment {
    pub document: DocumentSnapshot,
    pub previous_path: Option<PathBuf>,
}

impl DocumentStore {
    pub fn open_file(&mut self, path: &Path) -> AppResult<DocumentSnapshot> {
        let canonical = dunce::canonicalize(path)?;
        if let Some(document) = self.snapshot_for_path(&canonical)? {
            return Ok(document);
        }

        let text = std::fs::read_to_string(&canonical)?;
        self.open_loaded_file(&canonical, text)
    }

    pub fn snapshot_for_path(&self, path: &Path) -> AppResult<Option<DocumentSnapshot>> {
        let normalized_path = normalize_path_for_index(path);
        let Some(id) = self.by_path.get(&normalized_path) else {
            return Ok(None);
        };

        self.documents
            .get(id)
            .cloned()
            .map(Some)
            .ok_or_else(|| AppError::Service("document index is inconsistent".into()))
    }

    pub fn snapshot(&self, id: BufferId) -> AppResult<DocumentSnapshot> {
        self.documents
            .get(&id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("buffer {id:?}")))
    }

    pub fn close_path(&mut self, path: &Path) -> AppResult<Option<DocumentSnapshot>> {
        let normalized_path = normalize_path_for_index(path);
        let Some(id) = self.by_path.remove(&normalized_path) else {
            return Ok(None);
        };
        Ok(self.documents.remove(&id))
    }

    pub fn open_loaded_file(&mut self, path: &Path, text: String) -> AppResult<DocumentSnapshot> {
        let path = path.to_path_buf();
        let indexed_path = normalize_path_for_index(&path);
        if let Some(document) = self.snapshot_for_path(&indexed_path)? {
            return Ok(document);
        }

        let document = DocumentSnapshot {
            id: BufferId::new(),
            language_id: monaco_language_id_for_path(&indexed_path),
            title: file_title(&indexed_path),
            path: Some(indexed_path.clone()),
            text,
            view: file_view_descriptor_for_path(&indexed_path),
            version: 1,
            is_dirty: false,
            is_untitled: false,
            opened_at: Utc::now(),
        };

        self.by_path.insert(indexed_path, document.id);
        self.documents.insert(document.id, document.clone());
        Ok(document)
    }

    pub fn new_untitled(&mut self) -> DocumentSnapshot {
        self.untitled_counter += 1;
        let title = format!("Untitled-{}", self.untitled_counter);
        let document = DocumentSnapshot {
            id: BufferId::new(),
            path: None,
            title,
            language_id: "plaintext".to_string(),
            text: String::new(),
            view: lux_core::FileViewDescriptor::default(),
            version: 1,
            is_dirty: true,
            is_untitled: true,
            opened_at: Utc::now(),
        };
        self.documents.insert(document.id, document.clone());
        document
    }

    pub fn update_text(&mut self, id: BufferId, text: String) -> AppResult<DocumentSnapshot> {
        let document = self
            .documents
            .get_mut(&id)
            .ok_or_else(|| AppError::NotFound(format!("buffer {id:?}")))?;
        document.text = text;
        document.version += 1;
        document.is_dirty = true;
        Ok(document.clone())
    }

    pub fn replace_text_for_path(
        &mut self,
        path: &Path,
        text: String,
        dirty: bool,
    ) -> AppResult<Option<DocumentSnapshot>> {
        let Some(document) = self.snapshot_for_path(path)? else {
            return Ok(None);
        };
        let document = self
            .documents
            .get_mut(&document.id)
            .ok_or_else(|| AppError::Service("document index is inconsistent".into()))?;
        document.text = text;
        document.version += 1;
        document.is_dirty = dirty;
        Ok(Some(document.clone()))
    }

    pub fn apply_edits(&mut self, id: BufferId, edits: &[TextEdit]) -> AppResult<DocumentSnapshot> {
        let document = self
            .documents
            .get_mut(&id)
            .ok_or_else(|| AppError::NotFound(format!("buffer {id:?}")))?;

        if edits.is_empty() {
            return Ok(document.clone());
        }

        let mut text = document.text.clone();
        apply_text_edits(&mut text, edits)?;

        document.text = text;
        document.version += 1;
        document.is_dirty = true;
        Ok(document.clone())
    }

    pub fn apply_edits_for_path(
        &mut self,
        path: &Path,
        edits: &[TextEdit],
    ) -> AppResult<Option<DocumentSnapshot>> {
        let Some(document) = self.snapshot_for_path(path)? else {
            return Ok(None);
        };
        self.apply_edits(document.id, edits).map(Some)
    }

    pub fn save_file(&mut self, id: BufferId) -> AppResult<DocumentSnapshot> {
        let payload = self.save_payload(id)?;
        let path = payload.path.ok_or_else(|| {
            AppError::InvalidPath("untitled document requires a save path".to_string())
        })?;
        // Atomic save: a direct `fs::write` truncates the target before the new
        // bytes are durably on disk, so a disk-full error, crash, or AV lock can
        // leave the user's file empty or half-written. Write a sibling temp file,
        // fsync it, then rename over the target so the on-disk file is always
        // either the old or the new content — never a corrupt intermediate.
        atomic_write(&path, payload.text.as_bytes())?;
        Ok(self.finish_save(id, payload.version)?.0)
    }

    pub fn save_payload(&self, id: BufferId) -> AppResult<DocumentSavePayload> {
        let document = self
            .documents
            .get(&id)
            .ok_or_else(|| AppError::NotFound(format!("buffer {id:?}")))?;
        Ok(DocumentSavePayload {
            path: document.path.clone(),
            suggested_name: document.title.clone(),
            text: document.text.clone(),
            version: document.version,
            is_untitled: document.is_untitled,
        })
    }

    pub fn attach_path(&mut self, id: BufferId, path: PathBuf) -> AppResult<DocumentSnapshot> {
        Ok(self.attach_path_with_previous(id, path)?.document)
    }

    pub fn attach_path_with_previous(
        &mut self,
        id: BufferId,
        path: PathBuf,
    ) -> AppResult<DocumentPathAttachment> {
        let normalized_path = normalize_save_path(path)?;
        self.ensure_attachable_path(id, &normalized_path)?;

        let document = self
            .documents
            .get_mut(&id)
            .ok_or_else(|| AppError::NotFound(format!("buffer {id:?}")))?;
        let previous_path = document.path.clone();
        if let Some(previous_path) = &previous_path {
            self.by_path.remove(previous_path);
        }
        document.path = Some(normalized_path.clone());
        document.title = file_title(&normalized_path);
        document.language_id = monaco_language_id_for_path(&normalized_path);
        document.view = file_view_descriptor_for_path(&normalized_path);
        document.is_untitled = false;
        self.by_path.insert(normalized_path, id);
        Ok(DocumentPathAttachment {
            document: document.clone(),
            previous_path,
        })
    }

    pub fn validate_attach_path(&self, id: BufferId, path: &Path) -> AppResult<PathBuf> {
        let normalized_path = normalize_save_path(path.to_path_buf())?;
        self.ensure_attachable_path(id, &normalized_path)?;
        Ok(normalized_path)
    }

    fn ensure_attachable_path(&self, id: BufferId, normalized_path: &Path) -> AppResult<()> {
        if let Some(existing_id) = self.by_path.get(normalized_path) {
            if *existing_id != id {
                return Err(AppError::Service(format!(
                    "file is already open in another editor: {}",
                    normalized_path.display()
                )));
            }
        }
        Ok(())
    }

    pub fn finish_save(
        &mut self,
        id: BufferId,
        saved_version: u64,
    ) -> AppResult<(DocumentSnapshot, bool)> {
        let document = self
            .documents
            .get_mut(&id)
            .ok_or_else(|| AppError::NotFound(format!("buffer {id:?}")))?;

        let saved_current_version = document.version == saved_version;
        if saved_current_version {
            document.version += 1;
            document.is_dirty = false;
        }

        Ok((document.clone(), saved_current_version))
    }

    #[must_use]
    pub fn snapshots(&self) -> Vec<DocumentSnapshot> {
        self.documents.values().cloned().collect()
    }
}

/// Durably write `bytes` to `path` via a sibling temp file + atomic rename.
///
/// The target is never truncated in place: we create `.<name>.lux-tmp-<pid>-<nanos>`
/// next to it, write + flush + `sync_all` (so the bytes hit the disk, not just the
/// page cache), then `rename` over the target. On any failure the temp file is
/// removed and the original is left untouched. The sibling lives in the same
/// directory so the rename stays on one filesystem (cross-device renames fail).
pub fn atomic_write(path: &Path, bytes: &[u8]) -> AppResult<()> {
    let temp_path = temp_sibling_path(path);

    let write_result = (|| -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)?;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        Ok(())
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp_path);
        return Err(error.into());
    }

    if let Err(error) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(error.into());
    }
    Ok(())
}

/// A hidden, per-process, per-call sibling path used as the atomic-write staging
/// file. The `.` prefix keeps it out of casual listings; pid + nanos avoid
/// collisions between concurrent saves of the same file.
fn temp_sibling_path(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("document");
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    parent.join(format!(
        ".{file_name}.lux-tmp-{}-{nanos}",
        std::process::id()
    ))
}

pub fn apply_text_edit(text: &mut String, edit: &TextEdit) -> AppResult<()> {
    let start = position_to_byte_offset(text, edit.start_line, edit.start_column)?;
    let end = position_to_byte_offset(text, edit.end_line, edit.end_column)?;
    if start > end {
        return Err(AppError::Service(format!(
            "invalid edit range: start {start} is after end {end}"
        )));
    }

    text.replace_range(start..end, &edit.text);
    Ok(())
}

pub fn apply_text_edits(text: &mut String, edits: &[TextEdit]) -> AppResult<()> {
    if edits.is_empty() {
        return Ok(());
    }

    // Resolve every edit against the *original* document once, up front. Applying
    // edits one-by-one in reverse only stays correct if the ranges do not overlap;
    // an AI tool or LSP adapter emitting overlapping ranges would otherwise have
    // its later edits interpreted against already-mutated text and silently corrupt
    // the file. We therefore precompute byte ranges, reject overlaps, and only then
    // mutate — turning silent corruption into a clear, fail-fast error.
    let mut ranges: Vec<ResolvedEdit<'_>> = edits
        .iter()
        .map(|edit| {
            let start = position_to_byte_offset(text, edit.start_line, edit.start_column)?;
            let end = position_to_byte_offset(text, edit.end_line, edit.end_column)?;
            if start > end {
                return Err(AppError::Service(format!(
                    "invalid edit range: start {start} is after end {end}"
                )));
            }
            Ok(ResolvedEdit {
                start,
                end,
                text: &edit.text,
            })
        })
        .collect::<AppResult<Vec<_>>>()?;

    // Sort by start ascending so an overlap is always a clash with the immediate
    // predecessor. Adjacent ranges (and multiple zero-width inserts at the same
    // offset) are permitted; a strict `start < previous_end` is the only overlap.
    ranges.sort_by_key(|edit| (edit.start, edit.end));
    for window in ranges.windows(2) {
        let (previous, current) = (&window[0], &window[1]);
        if current.start < previous.end {
            return Err(AppError::Service(format!(
                "overlapping edits: range {}..{} overlaps {}..{}",
                previous.start, previous.end, current.start, current.end
            )));
        }
    }

    // Apply from the end of the document backwards so each replacement leaves the
    // offsets of the not-yet-applied (earlier) edits unchanged.
    for edit in ranges.iter().rev() {
        text.replace_range(edit.start..edit.end, edit.text);
    }
    Ok(())
}

/// An edit resolved to absolute byte offsets in the original document, ready for
/// overlap validation and back-to-front application.
struct ResolvedEdit<'a> {
    start: usize,
    end: usize,
    text: &'a str,
}

pub fn position_to_byte_offset(text: &str, line: u32, column: u32) -> AppResult<usize> {
    if line == 0 || column == 0 {
        return Err(AppError::Service("text edit positions are 1-based".into()));
    }

    let target_line = line as usize;
    let target_column = column as usize;
    let mut current_line = 1_usize;
    let mut current_column = 1_usize;

    for (offset, character) in text.char_indices() {
        if current_line == target_line && current_column == target_column {
            return Ok(offset);
        }

        if character == '\n' {
            current_line += 1;
            current_column = 1;
        } else {
            current_column += character.len_utf16();
        }
    }

    if current_line == target_line && current_column == target_column {
        Ok(text.len())
    } else {
        Err(AppError::Service(format!(
            "text edit position {line}:{column} is outside the document"
        )))
    }
}

fn normalize_save_path(path: PathBuf) -> AppResult<PathBuf> {
    let Some(file_name) = path.file_name() else {
        return Err(AppError::InvalidPath(path.display().to_string()));
    };
    let Some(parent) = path.parent() else {
        return Ok(path);
    };
    Ok(dunce::canonicalize(parent)?.join(file_name))
}

fn normalize_path_for_index(path: &Path) -> PathBuf {
    dunce::canonicalize(path)
        .or_else(|_| normalize_save_path(path.to_path_buf()))
        .unwrap_or_else(|_| path.to_path_buf())
}

fn file_title(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .map_or_else(|| path.to_string_lossy().into_owned(), ToOwned::to_owned)
}

#[must_use]
pub fn language_id_for_path(path: &Path) -> String {
    monaco_language_id_for_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_text_edit_replaces_single_line_range() {
        let mut text = "hello world".to_string();
        let edit = TextEdit {
            start_line: 1,
            start_column: 7,
            end_line: 1,
            end_column: 12,
            text: "Lux".to_string(),
        };

        apply_text_edit(&mut text, &edit).expect("edit should apply");

        assert_eq!(text, "hello Lux");
    }

    #[test]
    fn apply_text_edit_handles_multiline_insert() {
        let mut text = "fn main() {\n}\n".to_string();
        let edit = TextEdit {
            start_line: 2,
            start_column: 1,
            end_line: 2,
            end_column: 1,
            text: "    println!(\"lux\");\n".to_string(),
        };

        apply_text_edit(&mut text, &edit).expect("edit should apply");

        assert_eq!(text, "fn main() {\n    println!(\"lux\");\n}\n");
    }

    #[test]
    fn apply_edits_for_path_updates_open_document() {
        let mut store = DocumentStore::default();
        let path = PathBuf::from("/tmp/lux-editor-rename.rs");
        let document = store
            .open_loaded_file(&path, "let before = 1;\n".to_string())
            .expect("open should succeed");
        let edits = vec![TextEdit {
            start_line: 1,
            start_column: 5,
            end_line: 1,
            end_column: 11,
            text: "after".to_string(),
        }];

        let updated = store
            .apply_edits_for_path(&path, &edits)
            .expect("path edits should apply")
            .expect("open document should be returned");

        assert_eq!(updated.id, document.id);
        assert_eq!(updated.text, "let after = 1;\n");
        assert!(updated.is_dirty);
    }

    #[test]
    fn replace_text_for_path_can_keep_saved_document_clean() {
        let mut store = DocumentStore::default();
        let path = PathBuf::from("/tmp/lux-editor-ai-write.rs");
        let document = store
            .open_loaded_file(&path, "old".to_string())
            .expect("open should succeed");

        let updated = store
            .replace_text_for_path(&path, "new".to_string(), false)
            .expect("replace should succeed")
            .expect("open document should be returned");

        assert_eq!(updated.id, document.id);
        assert_eq!(updated.text, "new");
        assert!(!updated.is_dirty);
        assert_eq!(updated.version, document.version + 1);
    }

    #[test]
    fn close_path_removes_open_document_and_path_index() {
        let mut store = DocumentStore::default();
        let path = PathBuf::from("/tmp/lux-editor-close-path.rs");
        let document = store
            .open_loaded_file(&path, "text".to_string())
            .expect("open should succeed");

        let closed = store
            .close_path(&path)
            .expect("close should succeed")
            .expect("document should be closed");

        assert_eq!(closed.id, document.id);
        assert!(store.snapshot_for_path(&path).unwrap().is_none());
        assert!(store.snapshot(document.id).is_err());
    }

    #[test]
    fn apply_edits_uses_original_positions_for_multiple_edits() {
        let mut store = DocumentStore::default();
        let document = store.new_untitled();
        store
            .update_text(document.id, "alpha beta gamma".to_string())
            .unwrap();
        let edits = vec![
            TextEdit {
                start_line: 1,
                start_column: 1,
                end_line: 1,
                end_column: 6,
                text: "a".to_string(),
            },
            TextEdit {
                start_line: 1,
                start_column: 12,
                end_line: 1,
                end_column: 17,
                text: "g".to_string(),
            },
        ];

        let updated = store
            .apply_edits(document.id, &edits)
            .expect("edits should apply");

        assert_eq!(updated.text, "a beta g");
    }

    #[test]
    fn apply_edits_rejects_overlapping_ranges() {
        let mut text = "alpha beta gamma".to_string();
        // Two edits whose original ranges overlap (1..6 and 3..10) must fail fast
        // instead of silently corrupting the buffer.
        let edits = vec![
            TextEdit {
                start_line: 1,
                start_column: 1,
                end_line: 1,
                end_column: 6,
                text: "x".to_string(),
            },
            TextEdit {
                start_line: 1,
                start_column: 4,
                end_line: 1,
                end_column: 11,
                text: "y".to_string(),
            },
        ];

        let error = apply_text_edits(&mut text, &edits).expect_err("overlap must be rejected");
        assert!(error.to_string().contains("overlapping"));
        // The buffer is left untouched when validation fails.
        assert_eq!(text, "alpha beta gamma");
    }

    #[test]
    fn apply_edits_allows_adjacent_and_same_point_inserts() {
        let mut text = "ab".to_string();
        // Adjacent ranges (1..1 insert + 1..2 replace) and zero-width inserts at the
        // same offset are not overlaps and must apply cleanly.
        let edits = vec![
            TextEdit {
                start_line: 1,
                start_column: 1,
                end_line: 1,
                end_column: 1,
                text: "[".to_string(),
            },
            TextEdit {
                start_line: 1,
                start_column: 1,
                end_line: 1,
                end_column: 2,
                text: "A".to_string(),
            },
        ];

        apply_text_edits(&mut text, &edits).expect("adjacent edits should apply");
        assert_eq!(text, "[Ab");
    }

    #[test]
    fn save_file_writes_atomically_without_leaving_temp_files() {
        let mut store = DocumentStore::default();
        let path = std::env::temp_dir().join(format!(
            "lux-editor-atomic-{}-{}.txt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos())
        ));
        std::fs::write(&path, "original").expect("seed file");
        let document = store.open_file(&path).expect("open should succeed");
        store
            .update_text(document.id, "updated contents".to_string())
            .expect("update should succeed");

        let saved = store.save_file(document.id).expect("save should succeed");
        assert!(!saved.is_dirty);
        assert_eq!(
            std::fs::read_to_string(&path).expect("read back"),
            "updated contents"
        );
        // The staging sibling for *this* file must be gone after a successful
        // rename (scope the check to our own file name to stay robust under the
        // parallel test runner).
        let our_temp_marker = format!(".{}.lux-tmp-", path.file_name().unwrap().to_string_lossy());
        let leftover = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&our_temp_marker)
            });
        assert!(!leftover, "atomic save must not leave temp files behind");

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn position_to_byte_offset_uses_utf16_columns() {
        let text = "a😀b";

        assert_eq!(position_to_byte_offset(text, 1, 1).unwrap(), 0);
        assert_eq!(position_to_byte_offset(text, 1, 2).unwrap(), 1);
        assert_eq!(position_to_byte_offset(text, 1, 4).unwrap(), 5);
        assert_eq!(position_to_byte_offset(text, 1, 5).unwrap(), 6);
    }

    #[test]
    fn open_loaded_file_reuses_existing_snapshot_for_path() {
        let mut store = DocumentStore::default();
        let path = PathBuf::from("/tmp/lux-editor-reuse.rs");

        let first = store
            .open_loaded_file(&path, "fn main() {}".to_string())
            .expect("first open should succeed");
        let second = store
            .open_loaded_file(&path, "changed on disk".to_string())
            .expect("second open should reuse existing document");

        assert_eq!(first.id, second.id);
        assert_eq!(second.text, "fn main() {}");
        assert!(second
            .path
            .as_ref()
            .is_some_and(|candidate| same_path_suffix(candidate, &path)));
        assert_eq!(second.title, "lux-editor-reuse.rs");
        assert!(!second.is_untitled);
        assert_eq!(
            store.snapshot_for_path(&path).unwrap().unwrap().id,
            first.id
        );
    }

    #[test]
    fn new_untitled_creates_dirty_unbacked_plaintext_document() {
        let mut store = DocumentStore::default();

        let first = store.new_untitled();
        let second = store.new_untitled();

        assert_ne!(first.id, second.id);
        assert_eq!(first.path, None);
        assert_eq!(first.title, "Untitled-1");
        assert_eq!(second.title, "Untitled-2");
        assert_eq!(first.language_id, "plaintext");
        assert!(first.is_dirty);
        assert!(first.is_untitled);
    }

    #[test]
    fn attach_path_converts_untitled_document_to_file_backed_snapshot() {
        let mut store = DocumentStore::default();
        let document = store.new_untitled();
        let path = std::env::temp_dir().join("lux-editor-attach.md");

        let attached = store
            .attach_path(document.id, path.clone())
            .expect("attach should succeed");

        let attached_path = attached
            .path
            .as_ref()
            .expect("attached document should have a path");
        assert!(same_path_suffix(attached_path, &path));
        assert_eq!(attached.title, "lux-editor-attach.md");
        assert_eq!(attached.language_id, "markdown");
        assert!(!attached.is_untitled);
        assert_eq!(
            store.snapshot_for_path(attached_path).unwrap().unwrap().id,
            document.id
        );
    }

    #[test]
    fn attach_path_reindexes_file_backed_document_for_save_as() {
        let mut store = DocumentStore::default();
        let old_path = std::env::temp_dir().join("lux-editor-save-as-old.rs");
        let new_path = std::env::temp_dir().join("lux-editor-save-as-new.ts");
        let document = store
            .open_loaded_file(&old_path, "fn main() {}".to_string())
            .expect("open should succeed");

        let attachment = store
            .attach_path_with_previous(document.id, new_path.clone())
            .expect("save as attach should succeed");

        assert!(attachment
            .previous_path
            .as_ref()
            .is_some_and(|path| same_path_suffix(path, &old_path)));
        let attached_path = attachment
            .document
            .path
            .as_ref()
            .expect("attached document should have a path");
        assert!(same_path_suffix(attached_path, &new_path));
        assert_eq!(attachment.document.title, "lux-editor-save-as-new.ts");
        assert_eq!(attachment.document.language_id, "typescript");
        assert!(store.snapshot_for_path(&old_path).unwrap().is_none());
        assert_eq!(
            store.snapshot_for_path(attached_path).unwrap().unwrap().id,
            document.id
        );
    }

    #[test]
    fn validate_attach_path_rejects_path_open_in_another_document() {
        let mut store = DocumentStore::default();
        let first_path = std::env::temp_dir().join("lux-editor-open-first.rs");
        let second_path = std::env::temp_dir().join("lux-editor-open-second.rs");
        let first = store
            .open_loaded_file(&first_path, "first".to_string())
            .expect("first open should succeed");
        store
            .open_loaded_file(&second_path, "second".to_string())
            .expect("second open should succeed");

        let error = store
            .validate_attach_path(first.id, &second_path)
            .expect_err("attaching to another open document path should fail");

        assert!(error.to_string().contains("already open"));
        assert_eq!(
            store.snapshot_for_path(&first_path).unwrap().unwrap().id,
            first.id
        );
    }

    #[test]
    fn finish_save_clears_dirty_only_when_saved_version_is_current() {
        let mut store = DocumentStore::default();
        let document = store
            .open_loaded_file(&PathBuf::from("/tmp/lux-editor-save.rs"), "one".to_string())
            .expect("open should succeed");
        let dirty = store
            .update_text(document.id, "two".to_string())
            .expect("update should succeed");
        let payload = store
            .save_payload(document.id)
            .expect("payload should exist");
        assert!(payload
            .path
            .as_ref()
            .is_some_and(|path| same_path_suffix(path, &PathBuf::from("/tmp/lux-editor-save.rs"))));
        assert_eq!(payload.version, dirty.version);

        store
            .update_text(document.id, "three".to_string())
            .expect("concurrent edit should succeed");
        let (after_stale_save, saved_current_version) = store
            .finish_save(document.id, payload.version)
            .expect("stale save should finish without clearing dirty state");

        assert!(!saved_current_version);
        assert!(after_stale_save.is_dirty);
        assert_eq!(after_stale_save.text, "three");

        let payload = store
            .save_payload(document.id)
            .expect("payload should exist");
        let (after_current_save, saved_current_version) = store
            .finish_save(document.id, payload.version)
            .expect("current save should clear dirty state");

        assert!(saved_current_version);
        assert!(!after_current_save.is_dirty);
        assert_eq!(after_current_save.version, payload.version + 1);
    }

    fn same_path_suffix(left: &Path, right: &Path) -> bool {
        let left = left
            .to_string_lossy()
            .replace("\\\\?\\", "")
            .replace('\\', "/");
        let right = right
            .to_string_lossy()
            .replace("\\\\?\\", "")
            .replace('\\', "/");
        left.ends_with(&right)
    }
}
