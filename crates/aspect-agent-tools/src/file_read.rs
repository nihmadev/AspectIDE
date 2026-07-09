use std::path::Path;

use crate::types::read_result::AiReadFileResult;

pub async fn ai_read_file(
    path: &Path,
    max_bytes: Option<u64>,
    start_line: Option<u32>,
    max_lines: Option<u32>,
) -> Result<AiReadFileResult, String> {
    const MAX_READ_BYTES: u64 = 10 * 1024 * 1024;
    let windowed = start_line.is_some() || max_lines.is_some();
    let default_bytes: u64 = if windowed { 2 * 1024 * 1024 } else { 120_000 };
    let max_bytes = max_bytes.unwrap_or(default_bytes).clamp(1, MAX_READ_BYTES);
    let path = path.to_path_buf();

    tokio::task::spawn_blocking(move || -> Result<AiReadFileResult, String> {
        use std::io::{BufRead, BufReader, Read};
        let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
        if !metadata.is_file() {
            return Err("path is not a file".to_string());
        }
        let size = metadata.len();

        if windowed {
            let start = start_line.map_or(1usize, |s| usize::try_from(s.max(1)).unwrap_or(1));
            let take = max_lines.map_or(usize::MAX, |c| usize::try_from(c).unwrap_or(usize::MAX));
            let out_cap = usize::try_from(max_bytes).unwrap_or(usize::MAX);
            let mut reader = BufReader::new(std::fs::File::open(&path).map_err(|e| e.to_string())?);
            let mut line_buf: Vec<u8> = Vec::new();
            let mut total_lines = 0usize;
            let mut text = String::new();
            let mut truncated = false;
            loop {
                line_buf.clear();
                let read = reader
                    .read_until(b'\n', &mut line_buf)
                    .map_err(|e| e.to_string())?;
                if read == 0 {
                    break;
                }
                total_lines += 1;
                if total_lines >= start && total_lines - start < take {
                    let mut end = line_buf.len();
                    if end > 0 && line_buf[end - 1] == b'\n' {
                        end -= 1;
                    }
                    if end > 0 && line_buf[end - 1] == b'\r' {
                        end -= 1;
                    }
                    let line = String::from_utf8_lossy(&line_buf[..end]);
                    if text.len() + line.len() + 1 > out_cap {
                        truncated = true;
                    } else {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(&line);
                    }
                }
            }
            return Ok(AiReadFileResult {
                path,
                text,
                truncated,
                size,
                total_lines,
                start_line: Some(start),
            });
        }

        let limit = max_bytes.min(size);
        let mut buffer = Vec::new();
        std::fs::File::open(&path)
            .map_err(|e| e.to_string())?
            .take(limit)
            .read_to_end(&mut buffer)
            .map_err(|e| e.to_string())?;
        if limit < size {
            if let Err(error) = std::str::from_utf8(&buffer) {
                if error.error_len().is_none() {
                    let valid = error.valid_up_to();
                    buffer.truncate(valid);
                }
            }
        }
        let text = String::from_utf8_lossy(&buffer).into_owned();
        let total_lines = if limit < size {
            let mut reader = BufReader::new(std::fs::File::open(&path).map_err(|e| e.to_string())?);
            let mut scratch: Vec<u8> = Vec::new();
            let mut count = 0usize;
            loop {
                scratch.clear();
                let read = reader
                    .read_until(b'\n', &mut scratch)
                    .map_err(|e| e.to_string())?;
                if read == 0 {
                    break;
                }
                count += 1;
            }
            count
        } else {
            text.lines().count()
        };
        Ok(AiReadFileResult {
            path,
            text,
            truncated: size > max_bytes,
            size,
            total_lines,
            start_line: None,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}
