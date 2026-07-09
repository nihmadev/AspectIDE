//! Vision image preprocessing for the AI chat pipeline.
//!
//! Raster images attached to a chat (pinned tabs, clipboard pastes, drag-drop,
//! browser screenshots) are sent to the model as base64 data URLs. Two problems
//! make the naive "read bytes → base64" path wasteful:
//!
//! 1. **Payload size.** Large PNG/BMP screenshots bloat the request body and the
//!    persisted chat history (every data URL is stored inline for thumbnails).
//! 2. **Vision token cost.** Providers bill vision by pixel *dimensions*, not
//!    bytes, so an oversized image silently burns context tokens.
//!
//! This module addresses both by optionally downscaling to a sane cap and
//! re-encoding to **lossless WebP** (`image` crate's pure-Rust VP8L encoder — no
//! libwebp/C dependency). WebP lossless is byte-for-byte reversible, so no visual
//! fidelity is lost.
//!
//! ## Adequate fallback (the hard requirement)
//!
//! Not every model accepts WebP — many local/OpenAI-compatible servers only
//! decode PNG/JPEG. The frontend capability resolver decides the target format
//! per provider/model; this module simply honors the requested `format` and
//! degrades safely on its own:
//!
//! - **Undecodable source** (HEIC/AVIF/SVG, or a corrupt file): the original
//!   bytes are passed through untouched with their real MIME type. The model
//!   either handles the original or it doesn't — we never make it *worse*.
//! - **`format = "png"`** requested: encode PNG (universally supported).
//! - **`format = "webp"`** requested but encoding fails for any reason: fall
//!   back to PNG rather than erroring the whole turn.
//! - **Smallest-wins:** if the source is already a model-friendly format
//!   (PNG/JPEG) and re-encoding would *grow* the payload (typical for JPEG
//!   photos → WebP-lossless), the original is kept. We never bloat.

use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};

use base64::{engine::general_purpose, Engine as _};
use image::{
    codecs::{png::PngEncoder, webp::WebPEncoder},
    DynamicImage, GenericImageView, ImageDecoder, ImageEncoder, ImageFormat,
};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{resolve_workspace_path, SharedState};

/// Hard ceiling on source bytes we will attempt to decode. Mirrors the vision
/// limit enforced on the frontend so we never load a runaway file into memory.
const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
/// Default longest-edge cap. Images larger than this are downscaled before
/// encoding; the value comfortably exceeds what any current model resolves
/// internally for vision, so detail relevant to the model is preserved.
const DEFAULT_MAX_DIMENSION: u32 = 2000;
/// Lower bound for the caller-supplied dimension cap, to avoid degenerate sizes.
const MIN_MAX_DIMENSION: u32 = 256;
/// Hard upper bound for the caller-supplied dimension cap. Prevents a buggy
/// frontend or malicious tool call from bypassing the downscale safety net and
/// sending oversized images that explode model vision tokens and encoder memory.
/// 4096 covers all current provider limits; larger values must be allowlisted.
const MAX_MAX_DIMENSION: u32 = 4096;
/// HARD decode-bomb ceiling: reject decoded images whose total pixel count
/// exceeds this limit even if their compressed byte size passed
/// `MAX_SOURCE_BYTES`. A hostile compressed PNG (e.g. 16k×16k solid colour) can
/// decode to hundreds of MiB while easily fitting in a 16 MiB compressed file.
/// Set well above `MAX_FORWARDED_PIXELS` so legitimately large screenshots are
/// decoded once and downscaled rather than rejected — only true decode bombs
/// past this ceiling fail closed.
const MAX_DECODED_PIXELS: u64 = 4 * 4096 * 4096; // ≈ 67 MP HARD decode-bomb ceiling
/// Soft forwarding ceiling: an image at or below this pixel count whose longest
/// edge already fits `MAX_MAX_DIMENSION` could in principle be forwarded as-is.
/// Images between this and [`MAX_DECODED_PIXELS`] are decoded once and
/// downscaled (by [`downscale_if_needed`]), never passed through full-resolution.
// `u64::from` is not const-stable, so widen with `as` here; the u32→u64 cast is
// lossless (that is exactly what `cast_lossless` would otherwise suggest).
#[allow(clippy::cast_lossless)]
const MAX_FORWARDED_PIXELS: u64 = (MAX_MAX_DIMENSION as u64) * (MAX_MAX_DIMENSION as u64);
/// Compile-time invariant: the soft forwarding ceiling must stay strictly below
/// the hard decode-bomb ceiling, so the middle band (FORWARDED..=DECODED) is
/// always decoded-then-downscaled rather than rejected. Also anchors a reference
/// to `MAX_FORWARDED_PIXELS` so the documented bound is not dead code.
// Deliberate static assertion on two constants — the whole point is to fail the
// build if the ceilings are ever reordered; not a redundant runtime check.
#[allow(clippy::assertions_on_constants)]
const _: () = assert!(MAX_FORWARDED_PIXELS < MAX_DECODED_PIXELS);
/// Encoded base64 payload length limit applied *before* decoding a data URL
/// (base64 expands ~33 %, so this matches `MAX_SOURCE_BYTES` after decode).
/// Using ceiling division: `ceil(MAX_SOURCE_BYTES` * 4 / 3).
// ~22 MB literal — fits `usize` on every supported (32/64-bit) target.
#[allow(clippy::cast_possible_truncation)]
const MAX_DATA_URL_BASE64_LEN: usize = ((MAX_SOURCE_BYTES * 4).div_ceil(3)) as usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetFormat {
    Webp,
    Png,
}

impl TargetFormat {
    fn parse(value: &str) -> Self {
        // `auto` is resolved on the frontend (it knows the provider/model). Any
        // unrecognized value degrades to the universally-supported PNG.
        match value.trim().to_ascii_lowercase().as_str() {
            "webp" => Self::Webp,
            _ => Self::Png,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisionEncodeRequest {
    /// Workspace-relative or absolute path to an image on disk. Mutually
    /// exclusive with `data_url`; `path` takes precedence when both are set.
    pub path: Option<PathBuf>,
    /// Inline `data:image/...;base64,...` source (clipboard paste, screenshot).
    pub data_url: Option<String>,
    /// Requested target encoding: `"webp"`, `"png"`, or `"auto"`.
    pub format: Option<String>,
    /// Longest-edge cap in pixels. Defaults to [`DEFAULT_MAX_DIMENSION`].
    pub max_dimension: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisionEncodeResponse {
    /// `data:<mime>;base64,<...>` ready to drop into an `image_url` content part.
    pub data_url: String,
    /// MIME type of the produced payload (`image/webp`, `image/png`, or the
    /// original type when passed through).
    pub mime_type: String,
    /// Encoded byte length of the payload (pre-base64).
    pub size: u64,
    /// Width of the produced image in pixels, when known.
    pub width: Option<u32>,
    /// Height of the produced image in pixels, when known.
    pub height: Option<u32>,
    /// True when the original bytes were forwarded unchanged (undecodable source
    /// or smallest-wins). Lets the caller surface "sent as-is" if useful.
    pub passthrough: bool,
}

/// Decoded source plus the bytes/MIME we would emit if we passed it through.
struct DecodedSource {
    image: DynamicImage,
    original_bytes: Vec<u8>,
    original_mime: String,
    /// Whether the original format is one every vision model accepts.
    original_model_friendly: bool,
}

pub fn encode_vision_image(
    request: VisionEncodeRequest,
    resolved_path: Option<PathBuf>,
) -> Result<VisionEncodeResponse, String> {
    let target = TargetFormat::parse(request.format.as_deref().unwrap_or("png"));
    // Clamp within [MIN_MAX_DIMENSION, MAX_MAX_DIMENSION]: a lower bound prevents
    // degenerate sizes; an upper bound stops a caller from disabling downscale
    // by passing an arbitrarily large value (finding: caller-controlled max bypass).
    let max_dimension = request
        .max_dimension
        .unwrap_or(DEFAULT_MAX_DIMENSION)
        .clamp(MIN_MAX_DIMENSION, MAX_MAX_DIMENSION);

    let (bytes, hint_mime) = load_source(&request, resolved_path)?;

    // Decode failure (HEIC/AVIF/SVG/corrupt) → forward the original untouched so
    // a model that *can* read the source format still receives it.
    let Some(decoded) = decode_source(&bytes, hint_mime.clone()) else {
        let mime = hint_mime.unwrap_or_else(|| "application/octet-stream".to_string());
        return Ok(passthrough_bytes(&mime, &bytes, None));
    };

    let resized = downscale_if_needed(&decoded.image, max_dimension);
    let effective = resized.as_ref().unwrap_or(&decoded.image);
    let was_resized = resized.is_some();

    // High-bit-depth / HDR sources (16-bit PNG, float) would be truncated to
    // 8-bit by `to_rgb8()`/`to_rgba8()` below, and since the 8-bit re-encode is
    // typically smaller than the original the smallest-wins guard would then
    // adopt it — silently dropping precision and contradicting the module's
    // "no fidelity lost" guarantee. When the original is already a model-friendly
    // format and we did not have to resize, forward it untouched instead. (A
    // high-bit-depth source in a format the model can't read, or one that needs
    // resizing, still falls through to the 8-bit encode — unavoidable there.)
    if !was_resized
        && decoded.original_model_friendly
        && matches!(
            decoded.image.color(),
            image::ColorType::Rgb16
                | image::ColorType::Rgba16
                | image::ColorType::L16
                | image::ColorType::La16
                | image::ColorType::Rgb32F
                | image::ColorType::Rgba32F
        )
    {
        return Ok(passthrough_bytes(
            &decoded.original_mime,
            &decoded.original_bytes,
            Some(decoded.image.dimensions()),
        ));
    }

    let encoded = match target {
        TargetFormat::Webp => {
            encode_webp(effective).or_else(|_| encode_png(effective).map(|b| (b, png_mime())))
        }
        TargetFormat::Png => encode_png(effective).map(|b| (b, png_mime())),
    };

    // Last-resort: even PNG failed (extremely unlikely). Forward original.
    let Ok((encoded_bytes, encoded_mime)) = encoded else {
        return Ok(passthrough_bytes(
            &decoded.original_mime,
            &decoded.original_bytes,
            Some(decoded.image.dimensions()),
        ));
    };

    // Smallest-wins: when we did not have to resize and the source was already a
    // model-friendly format, keep whichever payload is smaller. Re-encoding a
    // JPEG photo to lossless WebP/PNG routinely grows it 3–10×; bloating the
    // request would defeat the entire purpose.
    if !was_resized
        && decoded.original_model_friendly
        && decoded.original_bytes.len() <= encoded_bytes.len()
    {
        return Ok(passthrough_bytes(
            &decoded.original_mime,
            &decoded.original_bytes,
            Some(decoded.image.dimensions()),
        ));
    }

    let (out_w, out_h) = effective.dimensions();
    Ok(VisionEncodeResponse {
        data_url: to_data_url(&encoded_mime, &encoded_bytes),
        size: encoded_bytes.len() as u64,
        mime_type: encoded_mime,
        width: Some(out_w),
        height: Some(out_h),
        passthrough: false,
    })
}

fn load_source(
    request: &VisionEncodeRequest,
    resolved_path: Option<PathBuf>,
) -> Result<(Vec<u8>, Option<String>), String> {
    if let Some(path) = resolved_path {
        let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
        if !metadata.is_file() {
            return Err("path is not a file".to_string());
        }
        if metadata.len() > MAX_SOURCE_BYTES {
            return Err(format!(
                "image is too large for vision preprocessing: {} bytes",
                metadata.len()
            ));
        }
        let bytes = fs::read(&path).map_err(|error| error.to_string())?;
        let mime = mime_from_extension(&path);
        return Ok((bytes, mime));
    }

    if let Some(data_url) = request.data_url.as_deref() {
        return decode_data_url(data_url);
    }

    Err("vision encode request has neither path nor dataUrl".to_string())
}

/// Splits a `data:<mime>;base64,<payload>` URL into raw bytes + declared MIME.
///
/// Size guard is applied to the *encoded* payload length before any allocation
/// to prevent a large base64 string from causing a multi-hundred-MiB decode
/// before the check fires (finding: data URL size checked only after allocation).
fn decode_data_url(data_url: &str) -> Result<(Vec<u8>, Option<String>), String> {
    let rest = data_url
        .strip_prefix("data:")
        .ok_or_else(|| "dataUrl is not a data: URL".to_string())?;
    let (meta, payload) = rest
        .split_once(',')
        .ok_or_else(|| "dataUrl is missing a comma separator".to_string())?;

    // Require "base64" token and reject any non-standard metadata variant
    // (e.g. ";base64;extra") that might smuggle unsanitised MIME values.
    if !meta.contains(";base64") {
        return Err("dataUrl is not base64-encoded".to_string());
    }

    let payload = payload.trim();

    // Reject before allocation: base64 encodes 3 bytes as 4 chars, so a
    // payload length above MAX_DATA_URL_BASE64_LEN guarantees the decoded
    // result would exceed MAX_SOURCE_BYTES.
    if payload.len() > MAX_DATA_URL_BASE64_LEN {
        return Err(format!(
            "image data URL is too large for vision preprocessing: {} encoded chars",
            payload.len()
        ));
    }

    let mime = meta.split(';').next().filter(|value| !value.is_empty());
    let bytes = general_purpose::STANDARD
        .decode(payload)
        .map_err(|error| format!("dataUrl base64 decode failed: {error}"))?;
    // Post-decode check as a belt-and-suspenders safety net (padding variations
    // mean the pre-check is an approximation for non-canonical base64).
    if bytes.len() as u64 > MAX_SOURCE_BYTES {
        return Err(format!(
            "image is too large for vision preprocessing: {} bytes",
            bytes.len()
        ));
    }
    Ok((bytes, mime.map(str::to_string)))
}

/// Attempts to decode the source into a [`DynamicImage`]. Returns `None` for
/// formats `image` cannot read (HEIC/AVIF/SVG, …) so the caller can passthrough.
/// Borrows `bytes` so the caller retains ownership for the passthrough path.
///
/// Single-decoder path: the source is read exactly once through one
/// [`image::ImageReader`]/[`ImageDecoder`]. Dimensions are taken from the decoder
/// header before the full pixel buffer is allocated, so the pixel-bomb guard
/// rejects hostile compressed inputs (e.g. 16 k×16 k solid-colour PNG) that slip
/// under `MAX_SOURCE_BYTES` without ever expanding them.
///
/// Fail-closed: every fallible step (`with_guessed_format`, `into_decoder`,
/// `from_decoder`) early-returns `None` on error, so a probe that cannot be fully
/// established forwards the original bytes untouched rather than guessing.
///
/// EXIF orientation is read from the decoder and applied to the decoded image
/// *before* any downscale/re-encode downstream, so re-encoded payloads are
/// upright. `w * h` is orientation-invariant, so the bomb guard is unaffected by
/// a 90°/270° rotation tag.
fn decode_source(bytes: &[u8], hint_mime: Option<String>) -> Option<DecodedSource> {
    let format = image::guess_format(bytes).ok();

    // One reader, one decoder. `with_guessed_format`/`into_decoder` failing means
    // we cannot reliably decode this source — fail closed to passthrough.
    let mut decoder = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_decoder()
        .ok()?;

    // Reject decode bombs using the decoder's header dimensions, before the full
    // pixel buffer is allocated by `from_decoder`. Mirrors the HEIC/AVIF
    // passthrough so the model still receives the original bytes.
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > MAX_DECODED_PIXELS {
        return None;
    }

    // Read orientation off the decoder before it is consumed; tolerate failure
    // (no EXIF / unsupported) by treating it as "no transform".
    let orientation = decoder.orientation().ok();
    let mut image = DynamicImage::from_decoder(decoder).ok()?;
    if let Some(orientation) = orientation {
        image.apply_orientation(orientation);
    }

    // The source decoded successfully, so passthrough forwards the REAL bytes;
    // the content-sniffed `format` is therefore authoritative for the emitted
    // MIME. A mislabeled file (e.g. JPEG bytes saved as `.png`) must not be
    // announced under its extension/data-url hint, which strict media_type
    // validators (Anthropic) would reject. `hint_mime` only acts as a fallback
    // when `guess_format` could not name the format.
    let original_mime = format
        .map(mime_for_format)
        .or(hint_mime)
        .unwrap_or_else(|| "application/octet-stream".to_string());
    let original_model_friendly = matches!(format, Some(ImageFormat::Png | ImageFormat::Jpeg));
    Some(DecodedSource {
        image,
        original_bytes: bytes.to_vec(),
        original_mime,
        original_model_friendly,
    })
}

/// Downscales so the longest edge is at most `max_dimension`. Returns `None`
/// when the image already fits (no allocation, no quality change).
fn downscale_if_needed(image: &DynamicImage, max_dimension: u32) -> Option<DynamicImage> {
    let (width, height) = image.dimensions();
    if width.max(height) <= max_dimension {
        return None;
    }
    // `resize` preserves aspect ratio and fits within the bounding box.
    Some(image.resize(
        max_dimension,
        max_dimension,
        image::imageops::FilterType::Lanczos3,
    ))
}

/// Encodes to lossless WebP (VP8L). The encoder requires Rgb8/Rgba8 input.
fn encode_webp(image: &DynamicImage) -> Result<(Vec<u8>, String), String> {
    let mut buffer = Vec::new();
    let has_alpha = image.color().has_alpha();
    if has_alpha {
        let rgba = image.to_rgba8();
        WebPEncoder::new_lossless(Cursor::new(&mut buffer))
            .encode(
                rgba.as_raw(),
                rgba.width(),
                rgba.height(),
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|error| error.to_string())?;
    } else {
        let rgb = image.to_rgb8();
        WebPEncoder::new_lossless(Cursor::new(&mut buffer))
            .encode(
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                image::ExtendedColorType::Rgb8,
            )
            .map_err(|error| error.to_string())?;
    }
    Ok((buffer, webp_mime()))
}

/// Encodes to PNG — the universal fallback every vision model accepts.
fn encode_png(image: &DynamicImage) -> Result<Vec<u8>, String> {
    let mut buffer = Vec::new();
    let has_alpha = image.color().has_alpha();
    if has_alpha {
        let rgba = image.to_rgba8();
        PngEncoder::new(Cursor::new(&mut buffer))
            .write_image(
                rgba.as_raw(),
                rgba.width(),
                rgba.height(),
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|error| error.to_string())?;
    } else {
        let rgb = image.to_rgb8();
        PngEncoder::new(Cursor::new(&mut buffer))
            .write_image(
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                image::ExtendedColorType::Rgb8,
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(buffer)
}

/// Forwards already-owned source bytes verbatim under their declared MIME.
/// Used both for the decoded smallest-wins / encode-failure paths and for
/// sources `image` could not decode at all (HEIC/AVIF/SVG).
///
/// `dims` carries the already-known dimensions on the decoded paths so we avoid
/// a redundant second full-size decode (which would transiently double peak
/// memory for large images). The undecodable path passes `None` — its bytes
/// cannot be decoded for dimensions anyway, so the size stays unknown.
fn passthrough_bytes(mime: &str, bytes: &[u8], dims: Option<(u32, u32)>) -> VisionEncodeResponse {
    VisionEncodeResponse {
        data_url: to_data_url(mime, bytes),
        size: bytes.len() as u64,
        mime_type: mime.to_string(),
        width: dims.map(|(width, _)| width),
        height: dims.map(|(_, height)| height),
        passthrough: true,
    }
}

fn to_data_url(mime: &str, bytes: &[u8]) -> String {
    format!(
        "data:{mime};base64,{}",
        general_purpose::STANDARD.encode(bytes)
    )
}

/// Tauri command: preprocess a vision image (downscale + lossless WebP/PNG) for
/// the chat pipeline. A `path` is resolved against the active workspace exactly
/// like `file_asset_data`; a `dataUrl` source needs no path resolution. CPU-bound
/// decode/encode runs on a blocking thread to keep the async runtime responsive.
#[tauri::command]
pub async fn ai_vision_encode(
    state: State<'_, SharedState>,
    request: VisionEncodeRequest,
) -> Result<VisionEncodeResponse, String> {
    let resolved_path = match request.path.as_ref() {
        Some(path) => Some(resolve_workspace_path(&state, path)?),
        None => None,
    };
    tokio::task::spawn_blocking(move || encode_vision_image(request, resolved_path))
        .await
        .map_err(|error| error.to_string())?
}

fn webp_mime() -> String {
    "image/webp".to_string()
}

fn png_mime() -> String {
    "image/png".to_string()
}

fn mime_for_format(format: ImageFormat) -> String {
    match format {
        ImageFormat::Png => "image/png",
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::WebP => "image/webp",
        ImageFormat::Gif => "image/gif",
        ImageFormat::Bmp => "image/bmp",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn mime_from_extension(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" | "jpe" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "avif" => "image/avif",
        "heic" | "heif" => "image/heic",
        "svg" => "image/svg+xml",
        _ => return None,
    };
    Some(mime.to_string())
}

