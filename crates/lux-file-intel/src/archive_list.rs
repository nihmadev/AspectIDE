use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};

use bzip2::read::BzDecoder;
use flate2::read::MultiGzDecoder;
use lux_core::{AppError, AppResult, ArchiveEntryPreview};
use tar::Archive as TarArchive;
use xz2::read::XzDecoder;
use zip::ZipArchive;

pub struct ArchiveListing {
    pub entries: Vec<ArchiveEntryPreview>,
    pub total_entries: usize,
    pub truncated: bool,
}

pub fn list_archive_entries(path: &Path, max_entries: usize) -> AppResult<ArchiveListing> {
    let extension = lux_core::file_extension_for_path(path);
    match extension.as_str() {
        "zip" | "jar" | "war" | "ear" | "vsix" | "nupkg" | "whl" | "crate" | "apk" | "aab" => {
            list_zip_entries(path, max_entries)
        }
        "tar" => list_tar_entries(path, max_entries),
        "tar.gz" | "tgz" | "gz" => {
            list_compressed_tar_entries(path, max_entries, Compression::Gzip)
        }
        "tar.bz2" | "tbz2" | "bz2" => {
            list_compressed_tar_entries(path, max_entries, Compression::Bzip2)
        }
        "tar.xz" | "txz" | "xz" => list_compressed_tar_entries(path, max_entries, Compression::Xz),
        "rar" | "7z" | "zst" | "br" => Ok(ArchiveListing {
            entries: Vec::new(),
            total_entries: 0,
            truncated: false,
        }),
        _ => Err(AppError::Service(format!(
            "unsupported archive extension: {extension}"
        ))),
    }
}

#[derive(Clone, Copy)]
enum Compression {
    Gzip,
    Bzip2,
    Xz,
}

/// Hard ceiling on the number of *decompressed* bytes we are willing to inflate
/// while listing a compressed tar. On a non-seekable stream tar must inflate
/// each entry's payload to advance to the next header, so the entry-count cap
/// alone cannot bound work: a single entry declaring a huge size would inflate
/// gigabytes before the count check is ever reached. Capping the decoder makes
/// the iterator hit EOF once this budget is spent.
const MAX_DECOMPRESSED_BYTES: u64 = 256 * 1024 * 1024;

fn list_zip_entries(path: &Path, max_entries: usize) -> AppResult<ArchiveListing> {
    let file = File::open(path)?;
    let mut archive =
        ZipArchive::new(file).map_err(|error| AppError::Service(error.to_string()))?;
    let total_entries = archive.len();
    let mut entries = Vec::new();
    for index in 0..total_entries.min(max_entries) {
        let entry = archive
            .by_index(index)
            .map_err(|error| AppError::Service(error.to_string()))?;
        entries.push(ArchiveEntryPreview {
            path: entry.name().to_string(),
            compressed_size: entry.compressed_size(),
            uncompressed_size: entry.size(),
            is_dir: entry.is_dir(),
        });
    }
    Ok(ArchiveListing {
        entries,
        total_entries,
        truncated: total_entries > max_entries,
    })
}

fn list_tar_entries(path: &Path, max_entries: usize) -> AppResult<ArchiveListing> {
    let file = File::open(path)?;
    collect_tar_entries(BufReader::new(file), max_entries)
}

fn list_compressed_tar_entries(
    path: &Path,
    max_entries: usize,
    compression: Compression,
) -> AppResult<ArchiveListing> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let extension = lux_core::file_extension_for_path(path);
    let is_tar_inside = matches!(
        extension.as_str(),
        "tar.gz" | "tgz" | "tar.bz2" | "tbz2" | "tar.xz" | "txz"
    );

    if is_tar_inside {
        // Bound total decompressed bytes, not just entry count: tar must
        // inflate each entry's payload to reach the next header, so without
        // this cap a crafted archive with one oversized entry would inflate
        // gigabytes before the entry-count limit in collect_tar_entries runs.
        let decoder: Box<dyn Read> = match compression {
            Compression::Gzip => {
                Box::new(MultiGzDecoder::new(reader).take(MAX_DECOMPRESSED_BYTES))
            }
            Compression::Bzip2 => Box::new(BzDecoder::new(reader).take(MAX_DECOMPRESSED_BYTES)),
            Compression::Xz => Box::new(XzDecoder::new(reader).take(MAX_DECOMPRESSED_BYTES)),
        };
        return collect_tar_entries(decoder, max_entries);
    }

    // Standalone .gz/.bz2/.xz — expose as a single logical entry.
    let metadata = std::fs::metadata(path)?;
    let inner_name = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("payload")
        .to_string();
    Ok(ArchiveListing {
        entries: vec![ArchiveEntryPreview {
            path: inner_name,
            compressed_size: metadata.len(),
            // Decompressed payload size is unknown without inflating the
            // whole stream; report 0 rather than the compressed on-disk size.
            uncompressed_size: 0,
            is_dir: false,
        }],
        total_entries: 1,
        truncated: false,
    })
}

fn collect_tar_entries<R: Read>(reader: R, max_entries: usize) -> AppResult<ArchiveListing> {
    let mut archive = TarArchive::new(reader);
    let mut entries = Vec::new();
    let mut total_entries = 0usize;
    let mut truncated = false;
    for entry in archive
        .entries()
        .map_err(|error| AppError::Service(error.to_string()))?
    {
        // Stop before decoding further: for compressed tars the iterator must
        // inflate each entry's data to reach the next header, so draining to
        // EOF on a malicious archive is a decompression bomb. Bounding to
        // max_entries headers keeps a preview cheap. total_entries is then a
        // lower bound (== entries collected) rather than an exact count.
        if entries.len() >= max_entries {
            truncated = true;
            break;
        }
        let entry = entry.map_err(|error| AppError::Service(error.to_string()))?;
        total_entries = total_entries.saturating_add(1);
        let header = entry.header();
        let path = entry
            .path()
            .map_err(|error| AppError::Service(error.to_string()))?
            .display()
            .to_string();
        entries.push(ArchiveEntryPreview {
            path,
            compressed_size: header.size().unwrap_or(0),
            uncompressed_size: header.size().unwrap_or(0),
            is_dir: header.entry_type().is_dir(),
        });
    }
    Ok(ArchiveListing {
        entries,
        total_entries,
        truncated,
    })
}
