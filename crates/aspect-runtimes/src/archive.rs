use std::path::Path;

/// Extract a `.zip` or `.tar.gz` archive into `dest` (created fresh). Runs on a
/// blocking thread — the zip/tar crates are synchronous.
pub async fn extract_archive(archive: &Path, dest: &Path, ext: &str) -> Result<(), String> {
    tokio::fs::create_dir_all(dest)
        .await
        .map_err(|e| e.to_string())?;
    let archive = archive.to_path_buf();
    let dest = dest.to_path_buf();
    let ext = ext.to_string();
    tokio::task::spawn_blocking(move || {
        if ext == "zip" {
            extract_zip(&archive, &dest)
        } else {
            extract_tar_gz(&archive, &dest)
        }
    })
    .await
    .map_err(|e| format!("Extraction task failed: {e}"))?
}

fn extract_zip(archive: &Path, dest: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|e| e.to_string())?;
        let Some(rel) = entry.enclosed_name() else {
            continue;
        };
        let outpath = dest.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&outpath).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = outpath.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out = std::fs::File::create(&outpath).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = entry.unix_mode() {
                let _ = std::fs::set_permissions(&outpath, std::fs::Permissions::from_mode(mode));
            }
        }
    }
    Ok(())
}

fn extract_tar_gz(archive: &Path, dest: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(decoder);
    tar.unpack(dest).map_err(|e| e.to_string())
}

/// Derive the archive kind from a release asset's file name.
pub fn archive_ext(asset: &str) -> Result<&'static str, String> {
    let path = Path::new(asset);
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    if ext.eq_ignore_ascii_case("zip") || ext.eq_ignore_ascii_case("tgz") {
        return Ok(if ext.eq_ignore_ascii_case("zip") {
            "zip"
        } else {
            "tar.gz"
        });
    }
    if ext.eq_ignore_ascii_case("gz")
        && Path::new(path.file_stem().unwrap_or_default())
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("tar"))
    {
        return Ok("tar.gz");
    }
    Err(format!(
        "Unsupported archive format for release asset {asset}"
    ))
}

