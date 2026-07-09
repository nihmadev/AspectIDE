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
use aspect_core::{
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
            view: aspect_core::FileViewDescriptor::default(),
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
        // either the old or the new content вЂ” never a corrupt intermediate.
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
/// The target is never truncated in place: we create `.<name>.aspect-tmp-<pid>-<nanos>`
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
        ".{file_name}.aspect-tmp-{}-{nanos}",
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
    // mutate вЂ” turning silent corruption into a clear, fail-fast error.
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

