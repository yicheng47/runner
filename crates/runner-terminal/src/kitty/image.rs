//! Kitty image chunk accumulation, bounded zlib inflation, PNG validation and
//! decoding, and stored image metadata. Contains no filesystem access.

use std::io::Cursor;
use std::sync::Arc;

use flate2::{Decompress, FlushDecompress, Status};

use super::command::{Command, Compression, Display, Format, Quiet, Response};

/// Maximum width or height of an image (Kitty and Ghostty).
pub const MAX_DIMENSION: u32 = 10_000;

/// Maximum transmitted payload size per image after base64 decoding and before
/// inflation (32 MiB), so one transmission stays well under `wire::MAX_FRAME`.
pub const MAX_TRANSMITTED_BYTES: usize = 32 * 1024 * 1024;

/// Default per-screen image storage limit (320 MB, matching Ghostty).
pub const PER_SCREEN_LIMIT: usize = 320_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageError {
    OutOfMemory,
    InsufficientData,
    InvalidData,
    DecompressionFailed,
    FilePathTooLong,
    TemporaryFileNotInTempDir,
    TemporaryFileNotNamedCorrectly,
    UnsupportedFormat,
    UnsupportedMedium,
    UnsupportedDepth,
    DimensionsRequired,
    DimensionsTooLarge,
}

impl ImageError {
    pub fn protocol_message(self) -> &'static str {
        match self {
            Self::OutOfMemory => "ENOMEM: out of memory",
            Self::InsufficientData => "ENODATA: insufficient data",
            Self::InvalidData => "EINVAL: invalid data",
            Self::DecompressionFailed => "EINVAL: decompression failed",
            Self::FilePathTooLong => "EINVAL: file path too long",
            Self::TemporaryFileNotInTempDir => "EINVAL: temporary file not in temp dir",
            Self::TemporaryFileNotNamedCorrectly => "EINVAL: temporary file not named correctly",
            Self::UnsupportedFormat => "EINVAL: unsupported format",
            Self::UnsupportedMedium => "EINVAL: unsupported medium",
            Self::UnsupportedDepth => "EINVAL: unsupported pixel depth",
            Self::DimensionsRequired => "EINVAL: dimensions required",
            Self::DimensionsTooLarge => "EINVAL: dimensions too large",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodedFormat {
    Rgb,
    Rgba,
}

impl DecodedFormat {
    pub fn bpp(self) -> usize {
        match self {
            Self::Rgb => 3,
            Self::Rgba => 4,
        }
    }
}

/// An image stored in screen memory in its transmitted encoding (PNG, zlib, or
/// raw RGB/RGBA) together with its validated dimensions and accounting size.
#[derive(Debug, Clone)]
pub struct StoredImage {
    pub id: u32,
    pub number: u32,
    pub width: u32,
    pub height: u32,
    pub wire_format: Format,
    pub compression: Compression,
    pub decoded_format: DecodedFormat,
    pub encoded_data: Arc<[u8]>,
    pub transient: bool,
    pub implicit_id: bool,
    pub placement_count: u32,
    pub generation: u64,
    pub transmission: u64,
}

impl StoredImage {
    /// Bytes counted against the per-screen limit: RGB at 3 bpp, RGBA
    /// (including decoded PNG) at 4 bpp, matching Ghostty.
    pub fn storage_bytes(&self) -> usize {
        (self.width as usize)
            .saturating_mul(self.height as usize)
            .saturating_mul(self.decoded_format.bpp())
    }

    /// Actual encoded payload bytes retained in memory, counted against the
    /// daemon-wide `GlobalImageBudget`.
    pub fn encoded_bytes(&self) -> usize {
        self.encoded_data.len()
    }

    /// Decode the stored image into tight RGBA8 pixels (`width * height * 4`).
    pub fn decode_rgba(&self, max_decoded_bytes: usize) -> Result<Vec<u8>, ImageError> {
        decode_to_rgba(
            self.wire_format,
            self.compression,
            self.width,
            self.height,
            &self.encoded_data,
            max_decoded_bytes,
        )
    }

    /// Decode the stored image directly into tight BGRA8 pixels
    /// (`width * height * 4`), ready for GPUI `RenderImage` without an extra
    /// full-frame channel-swap pass.
    pub fn decode_bgra(&self, max_decoded_bytes: usize) -> Result<Vec<u8>, ImageError> {
        decode_to_bgra(
            self.wire_format,
            self.compression,
            self.width,
            self.height,
            &self.encoded_data,
            max_decoded_bytes,
        )
    }
}

/// An in-progress chunked transmission on one screen.
#[derive(Debug, Clone)]
pub struct LoadingImage {
    pub id: u32,
    pub number: u32,
    pub width: u32,
    pub height: u32,
    pub format: Format,
    pub compression: Compression,
    pub transient: bool,
    pub implicit_id: bool,
    pub display: Option<Display>,
    pub quiet: Quiet,
    pub response: Response,
    pub data: Vec<u8>,
}

impl LoadingImage {
    pub fn new(cmd: &Command, initial_data: &[u8]) -> Result<Self, ImageError> {
        let t = cmd.control.transmission().ok_or(ImageError::InvalidData)?;
        if t.format_unknown {
            return Err(ImageError::UnsupportedFormat);
        }
        let mut loading = Self {
            id: t.image_id,
            number: t.image_number,
            width: t.width,
            height: t.height,
            format: t.format,
            compression: t.compression,
            transient: t.transient,
            implicit_id: false,
            display: cmd.control.display().copied(),
            quiet: cmd.quiet,
            response: Response {
                id: t.image_id,
                image_number: t.image_number,
                placement_id: t.placement_id,
                message: "OK",
            },
            data: Vec::new(),
        };
        loading.add_data(initial_data)?;
        Ok(loading)
    }

    pub fn add_data(&mut self, chunk: &[u8]) -> Result<(), ImageError> {
        if chunk.is_empty() {
            return Ok(());
        }
        if self.data.len().saturating_add(chunk.len()) > MAX_TRANSMITTED_BYTES {
            return Err(ImageError::InvalidData);
        }
        self.data.extend_from_slice(chunk);
        Ok(())
    }

    /// Validate the complete transmitted payload against `max_decoded_bytes`
    /// without retaining decoded pixels, returning the stored image in its
    /// transmitted encoding.
    pub fn complete(self, max_decoded_bytes: usize) -> Result<StoredImage, ImageError> {
        let (width, height, decoded_format) = validate_transmitted(
            self.format,
            self.compression,
            self.width,
            self.height,
            &self.data,
            max_decoded_bytes,
        )?;
        Ok(StoredImage {
            id: self.id,
            number: self.number,
            width,
            height,
            wire_format: self.format,
            compression: self.compression,
            decoded_format,
            encoded_data: Arc::from(self.data),
            transient: self.transient,
            implicit_id: self.implicit_id,
            placement_count: 0,
            generation: 0,
            transmission: 0,
        })
    }

    /// Complete a normalized transmission on a mirror without inflating zlib or
    /// decoding PNG under the terminal lock. Authoritative `runnerd` always
    /// validates payloads and emits explicit `s=` / `v=` dimensions (even for
    /// PNG), so missing dimensions are rejected without touching decoders.
    pub fn complete_normalized_mirror(
        self,
        max_decoded_bytes: usize,
    ) -> Result<StoredImage, ImageError> {
        if self.data.len() > MAX_TRANSMITTED_BYTES {
            return Err(ImageError::InvalidData);
        }
        if self.width == 0 || self.height == 0 {
            return Err(ImageError::DimensionsRequired);
        }
        if self.width > MAX_DIMENSION || self.height > MAX_DIMENSION {
            return Err(ImageError::DimensionsTooLarge);
        }
        let decoded_format = match self.format {
            Format::Rgb => DecodedFormat::Rgb,
            Format::Rgba | Format::Png => DecodedFormat::Rgba,
        };
        let expected = (self.width as usize)
            .checked_mul(self.height as usize)
            .and_then(|pixels| pixels.checked_mul(decoded_format.bpp()))
            .ok_or(ImageError::OutOfMemory)?;
        if expected > max_decoded_bytes {
            return Err(ImageError::OutOfMemory);
        }
        if self.format != Format::Png
            && self.compression == Compression::None
            && self.data.len() != expected
        {
            return Err(ImageError::InvalidData);
        }
        Ok(StoredImage {
            id: self.id,
            number: self.number,
            width: self.width,
            height: self.height,
            wire_format: self.format,
            compression: self.compression,
            decoded_format,
            encoded_data: Arc::from(self.data),
            transient: self.transient,
            implicit_id: self.implicit_id,
            placement_count: 0,
            generation: 0,
            transmission: 0,
        })
    }
}

pub fn validate_transmitted(
    format: Format,
    compression: Compression,
    width: u32,
    height: u32,
    data: &[u8],
    max_decoded_bytes: usize,
) -> Result<(u32, u32, DecodedFormat), ImageError> {
    if data.len() > MAX_TRANSMITTED_BYTES {
        return Err(ImageError::InvalidData);
    }
    match format {
        Format::Rgb | Format::Rgba => {
            if width == 0 || height == 0 {
                return Err(ImageError::DimensionsRequired);
            }
            if width > MAX_DIMENSION || height > MAX_DIMENSION {
                return Err(ImageError::DimensionsTooLarge);
            }
            let decoded_format = match format {
                Format::Rgb => DecodedFormat::Rgb,
                Format::Rgba => DecodedFormat::Rgba,
                Format::Png => unreachable!(),
            };
            let expected = (width as usize)
                .checked_mul(height as usize)
                .and_then(|pixels| pixels.checked_mul(decoded_format.bpp()))
                .ok_or(ImageError::OutOfMemory)?;
            if expected > max_decoded_bytes {
                return Err(ImageError::OutOfMemory);
            }
            match compression {
                Compression::None => {
                    if data.len() != expected {
                        return Err(ImageError::InvalidData);
                    }
                }
                Compression::Zlib => {
                    validate_zlib_exact_len(data, expected)?;
                }
            }
            Ok((width, height, decoded_format))
        }
        Format::Png => {
            let inflated;
            let png_bytes = match compression {
                Compression::None => data,
                Compression::Zlib => {
                    inflated = inflate_bounded(data, MAX_TRANSMITTED_BYTES)?;
                    &inflated
                }
            };
            let (png_w, png_h) = validate_png(png_bytes, max_decoded_bytes)?;
            Ok((png_w, png_h, DecodedFormat::Rgba))
        }
    }
}

fn validate_zlib_exact_len(compressed: &[u8], expected: usize) -> Result<(), ImageError> {
    let mut decomp = Decompress::new(true);
    let mut scratch = [0u8; 16 * 1024];
    let mut in_pos = 0usize;
    let mut total = 0usize;
    loop {
        let want = (expected + 1 - total).min(scratch.len());
        let before_in = decomp.total_in();
        let before_out = decomp.total_out();
        let status = decomp
            .decompress(
                &compressed[in_pos..],
                &mut scratch[..want],
                FlushDecompress::Sync,
            )
            .map_err(|_| ImageError::DecompressionFailed)?;
        let consumed = (decomp.total_in() - before_in) as usize;
        let produced = (decomp.total_out() - before_out) as usize;
        in_pos += consumed;
        total += produced;
        if total > expected {
            // Stop inflating past s * v * bpp without allocating the excess.
            return Err(ImageError::InvalidData);
        }
        match status {
            Status::StreamEnd => {
                return if total == expected {
                    Ok(())
                } else {
                    Err(ImageError::InvalidData)
                };
            }
            Status::Ok => {
                if consumed == 0 && produced == 0 {
                    return Err(ImageError::DecompressionFailed);
                }
            }
            Status::BufError => {
                return Err(ImageError::DecompressionFailed);
            }
        }
    }
}

fn inflate_bounded(compressed: &[u8], limit: usize) -> Result<Vec<u8>, ImageError> {
    let mut decomp = Decompress::new(true);
    let mut out = Vec::new();
    let mut scratch = [0u8; 16 * 1024];
    let mut in_pos = 0usize;
    loop {
        let before_in = decomp.total_in();
        let before_out = decomp.total_out();
        let status = decomp
            .decompress(&compressed[in_pos..], &mut scratch, FlushDecompress::Sync)
            .map_err(|_| ImageError::DecompressionFailed)?;
        let consumed = (decomp.total_in() - before_in) as usize;
        let produced = (decomp.total_out() - before_out) as usize;
        in_pos += consumed;
        if out.len().saturating_add(produced) > limit {
            return Err(ImageError::DecompressionFailed);
        }
        out.extend_from_slice(&scratch[..produced]);
        match status {
            Status::StreamEnd => return Ok(out),
            Status::Ok => {
                if consumed == 0 && produced == 0 {
                    return Err(ImageError::DecompressionFailed);
                }
            }
            Status::BufError => return Err(ImageError::DecompressionFailed),
        }
    }
}

fn png_decoder(bytes: &[u8]) -> png::Decoder<Cursor<&[u8]>> {
    // Cap internal decoder allocations (metadata/filter buffers); pixel-buffer
    // size is checked explicitly against `max_decoded_bytes` from IHDR first.
    let limits = png::Limits {
        bytes: 64 * 1024 * 1024,
    };
    let mut decoder = png::Decoder::new_with_limits(Cursor::new(bytes), limits);
    decoder.set_transformations(
        png::Transformations::EXPAND | png::Transformations::STRIP_16 | png::Transformations::ALPHA,
    );
    decoder
}

fn check_png_header(
    info: &png::Info<'_>,
    max_decoded_bytes: usize,
) -> Result<(u32, u32), ImageError> {
    let width = info.width;
    let height = info.height;
    if width == 0 || height == 0 {
        return Err(ImageError::DimensionsRequired);
    }
    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(ImageError::DimensionsTooLarge);
    }
    let rgba_bytes = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(ImageError::OutOfMemory)?;
    if rgba_bytes > max_decoded_bytes {
        return Err(ImageError::OutOfMemory);
    }
    Ok((width, height))
}

fn validate_png(png_bytes: &[u8], max_decoded_bytes: usize) -> Result<(u32, u32), ImageError> {
    let decoder = png_decoder(png_bytes);
    let mut reader = decoder.read_info().map_err(|_| ImageError::InvalidData)?;
    let (width, height) = check_png_header(reader.info(), max_decoded_bytes)?;
    // Drain rows one scanline at a time so validation never allocates the full
    // decoded image buffer in runnerd.
    while reader
        .next_interlaced_row()
        .map_err(|_| ImageError::InvalidData)?
        .is_some()
    {}
    Ok((width, height))
}

fn inflate_exact(compressed: &[u8], expected: usize) -> Result<Vec<u8>, ImageError> {
    let mut decomp = Decompress::new(true);
    let mut out = vec![0u8; expected];
    let mut in_pos = 0usize;
    let mut out_pos = 0usize;
    let mut overflow = [0u8; 1];
    loop {
        let before_in = decomp.total_in();
        let before_out = decomp.total_out();
        let status = if out_pos < expected {
            decomp.decompress(
                &compressed[in_pos..],
                &mut out[out_pos..],
                FlushDecompress::Sync,
            )
        } else {
            decomp.decompress(&compressed[in_pos..], &mut overflow, FlushDecompress::Sync)
        }
        .map_err(|_| ImageError::DecompressionFailed)?;
        let consumed = (decomp.total_in() - before_in) as usize;
        let produced = (decomp.total_out() - before_out) as usize;
        in_pos += consumed;
        if out_pos == expected && produced > 0 {
            return Err(ImageError::InvalidData);
        }
        out_pos += produced;
        match status {
            Status::StreamEnd => {
                return if out_pos == expected {
                    Ok(out)
                } else {
                    Err(ImageError::InvalidData)
                };
            }
            Status::Ok => {
                if consumed == 0 && produced == 0 {
                    return Err(ImageError::DecompressionFailed);
                }
            }
            Status::BufError => return Err(ImageError::DecompressionFailed),
        }
    }
}

fn expand_rgb_to_4bpp(src: &[u8], pixels: usize, bgra: bool) -> Vec<u8> {
    let mut out = vec![0u8; pixels * 4];
    let (src_chunks, src_rem) = src.as_chunks::<24>();
    let (dst_chunks, dst_rem) = out.as_chunks_mut::<32>();
    if bgra {
        for (s, d) in src_chunks.iter().zip(dst_chunks.iter_mut()) {
            *d = [
                s[2], s[1], s[0], 255, s[5], s[4], s[3], 255, s[8], s[7], s[6], 255, s[11], s[10],
                s[9], 255, s[14], s[13], s[12], 255, s[17], s[16], s[15], 255, s[20], s[19], s[18],
                255, s[23], s[22], s[21], 255,
            ];
        }
        for (s, d) in src_rem
            .as_chunks::<3>()
            .0
            .iter()
            .zip(dst_rem.as_chunks_mut::<4>().0.iter_mut())
        {
            *d = [s[2], s[1], s[0], 255];
        }
    } else {
        for (s, d) in src_chunks.iter().zip(dst_chunks.iter_mut()) {
            *d = [
                s[0], s[1], s[2], 255, s[3], s[4], s[5], 255, s[6], s[7], s[8], 255, s[9], s[10],
                s[11], 255, s[12], s[13], s[14], 255, s[15], s[16], s[17], 255, s[18], s[19],
                s[20], 255, s[21], s[22], s[23], 255,
            ];
        }
        for (s, d) in src_rem
            .as_chunks::<3>()
            .0
            .iter()
            .zip(dst_rem.as_chunks_mut::<4>().0.iter_mut())
        {
            *d = [s[0], s[1], s[2], 255];
        }
    }
    out
}

fn swap_rb_in_place(pixels: &mut [u8]) {
    let (chunks, rem) = pixels.as_chunks_mut::<32>();
    for c in chunks {
        *c = [
            c[2], c[1], c[0], c[3], c[6], c[5], c[4], c[7], c[10], c[9], c[8], c[11], c[14], c[13],
            c[12], c[15], c[18], c[17], c[16], c[19], c[22], c[21], c[20], c[23], c[26], c[25],
            c[24], c[27], c[30], c[29], c[28], c[31],
        ];
    }
    for px in rem.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
}

pub fn decode_to_rgba(
    format: Format,
    compression: Compression,
    width: u32,
    height: u32,
    data: &[u8],
    max_decoded_bytes: usize,
) -> Result<Vec<u8>, ImageError> {
    decode_to_4bpp(
        format,
        compression,
        width,
        height,
        data,
        max_decoded_bytes,
        false,
    )
}

pub fn decode_to_bgra(
    format: Format,
    compression: Compression,
    width: u32,
    height: u32,
    data: &[u8],
    max_decoded_bytes: usize,
) -> Result<Vec<u8>, ImageError> {
    decode_to_4bpp(
        format,
        compression,
        width,
        height,
        data,
        max_decoded_bytes,
        true,
    )
}

fn decode_to_4bpp(
    format: Format,
    compression: Compression,
    width: u32,
    height: u32,
    data: &[u8],
    max_decoded_bytes: usize,
    bgra: bool,
) -> Result<Vec<u8>, ImageError> {
    match format {
        Format::Rgb | Format::Rgba => {
            if width == 0 || height == 0 {
                return Err(ImageError::DimensionsRequired);
            }
            if width > MAX_DIMENSION || height > MAX_DIMENSION {
                return Err(ImageError::DimensionsTooLarge);
            }
            let bpp = format.raw_bpp().unwrap();
            let pixels = (width as usize)
                .checked_mul(height as usize)
                .ok_or(ImageError::OutOfMemory)?;
            let expected = pixels.checked_mul(bpp).ok_or(ImageError::OutOfMemory)?;
            if expected > max_decoded_bytes {
                return Err(ImageError::OutOfMemory);
            }
            match (format, compression) {
                (Format::Rgba, Compression::None) => {
                    if data.len() != expected {
                        return Err(ImageError::InvalidData);
                    }
                    let mut out = data.to_vec();
                    if bgra {
                        swap_rb_in_place(&mut out);
                    }
                    Ok(out)
                }
                (Format::Rgba, Compression::Zlib) => {
                    let mut out = inflate_exact(data, expected)?;
                    if bgra {
                        swap_rb_in_place(&mut out);
                    }
                    Ok(out)
                }
                (Format::Rgb, Compression::None) => {
                    if data.len() != expected {
                        return Err(ImageError::InvalidData);
                    }
                    Ok(expand_rgb_to_4bpp(data, pixels, bgra))
                }
                (Format::Rgb, Compression::Zlib) => {
                    let inflated = inflate_exact(data, expected)?;
                    Ok(expand_rgb_to_4bpp(&inflated, pixels, bgra))
                }
                (Format::Png, _) => unreachable!(),
            }
        }
        Format::Png => {
            let inflated;
            let png_bytes = match compression {
                Compression::None => data,
                Compression::Zlib => {
                    inflated = inflate_bounded(data, MAX_TRANSMITTED_BYTES)?;
                    &inflated
                }
            };
            let decoder = png_decoder(png_bytes);
            let mut reader = decoder.read_info().map_err(|_| ImageError::InvalidData)?;
            let (w, h) = check_png_header(reader.info(), max_decoded_bytes)?;
            if (width > 0 && w != width) || (height > 0 && h != height) {
                return Err(ImageError::InvalidData);
            }
            let buf_size = reader.output_buffer_size().ok_or(ImageError::OutOfMemory)?;
            if buf_size > max_decoded_bytes {
                return Err(ImageError::OutOfMemory);
            }
            let mut buf = vec![0u8; buf_size];
            let output = reader
                .next_frame(&mut buf)
                .map_err(|_| ImageError::InvalidData)?;
            let src = &buf[..output.buffer_size()];
            let pixels = (w as usize) * (h as usize);
            match output.color_type {
                png::ColorType::Rgba => {
                    buf.truncate(pixels * 4);
                    if bgra {
                        swap_rb_in_place(&mut buf);
                    }
                    Ok(buf)
                }
                png::ColorType::Rgb => Ok(expand_rgb_to_4bpp(src, pixels, bgra)),
                png::ColorType::GrayscaleAlpha => {
                    let mut out = vec![0u8; pixels * 4];
                    for (s, d) in src
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .zip(out.as_chunks_mut::<4>().0.iter_mut())
                    {
                        *d = [s[0], s[0], s[0], s[1]];
                    }
                    Ok(out)
                }
                png::ColorType::Grayscale => {
                    let mut out = vec![0u8; pixels * 4];
                    for (&g, d) in src.iter().zip(out.as_chunks_mut::<4>().0.iter_mut()) {
                        *d = [g, g, g, 255];
                    }
                    Ok(out)
                }
                png::ColorType::Indexed => Err(ImageError::InvalidData),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use flate2::write::ZlibEncoder;
    use flate2::Compression as FlateLevel;

    use super::*;

    fn zlib_compress(data: &[u8]) -> Vec<u8> {
        let mut enc = ZlibEncoder::new(Vec::new(), FlateLevel::fast());
        enc.write_all(data).unwrap();
        enc.finish().unwrap()
    }

    fn make_png(width: u32, height: u32, color: png::ColorType, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, width, height);
            enc.set_color(color);
            enc.set_depth(png::BitDepth::Eight);
            let mut writer = enc.write_header().unwrap();
            writer.write_image_data(data).unwrap();
        }
        out
    }

    #[test]
    fn validates_rgb_rgba_png_and_zlib() {
        let rgb = vec![255, 0, 0, 0, 255, 0];
        let (w, h, fmt) =
            validate_transmitted(Format::Rgb, Compression::None, 2, 1, &rgb, PER_SCREEN_LIMIT)
                .unwrap();
        assert_eq!((w, h, fmt), (2, 1, DecodedFormat::Rgb));

        let z_rgb = zlib_compress(&rgb);
        let (w, h, fmt) = validate_transmitted(
            Format::Rgb,
            Compression::Zlib,
            2,
            1,
            &z_rgb,
            PER_SCREEN_LIMIT,
        )
        .unwrap();
        assert_eq!((w, h, fmt), (2, 1, DecodedFormat::Rgb));
        assert_eq!(
            decode_to_rgba(
                Format::Rgb,
                Compression::Zlib,
                2,
                1,
                &z_rgb,
                PER_SCREEN_LIMIT
            )
            .unwrap(),
            vec![255, 0, 0, 255, 0, 255, 0, 255]
        );
        assert_eq!(
            decode_to_bgra(
                Format::Rgb,
                Compression::Zlib,
                2,
                1,
                &z_rgb,
                PER_SCREEN_LIMIT
            )
            .unwrap(),
            vec![0, 0, 255, 255, 0, 255, 0, 255]
        );

        // Exercise both the 8-pixel unrolled loop and the tail remainder for
        // RGB, RGBA and PNG direct-to-BGRA decoding.
        let (w_odd, h_odd) = (11u32, 3u32);
        let rgb_odd: Vec<u8> = (0..(w_odd * h_odd))
            .flat_map(|i| [(i * 7) as u8, (i * 13) as u8, (i * 29) as u8])
            .collect();
        let z_rgb_odd = zlib_compress(&rgb_odd);
        let rgba_from_rgb = decode_to_rgba(
            Format::Rgb,
            Compression::Zlib,
            w_odd,
            h_odd,
            &z_rgb_odd,
            PER_SCREEN_LIMIT,
        )
        .unwrap();
        let mut expected_bgra = rgba_from_rgb.clone();
        for px in expected_bgra.as_chunks_mut::<4>().0 {
            px.swap(0, 2);
        }
        assert_eq!(
            decode_to_bgra(
                Format::Rgb,
                Compression::Zlib,
                w_odd,
                h_odd,
                &z_rgb_odd,
                PER_SCREEN_LIMIT
            )
            .unwrap(),
            expected_bgra
        );
        assert_eq!(
            decode_to_bgra(
                Format::Rgba,
                Compression::Zlib,
                w_odd,
                h_odd,
                &zlib_compress(&rgba_from_rgb),
                PER_SCREEN_LIMIT
            )
            .unwrap(),
            expected_bgra
        );
        let png_rgb_odd = make_png(w_odd, h_odd, png::ColorType::Rgb, &rgb_odd);
        assert_eq!(
            decode_to_bgra(
                Format::Png,
                Compression::None,
                0,
                0,
                &png_rgb_odd,
                PER_SCREEN_LIMIT
            )
            .unwrap(),
            expected_bgra
        );
        let png_rgba_odd = make_png(w_odd, h_odd, png::ColorType::Rgba, &rgba_from_rgb);
        assert_eq!(
            decode_to_bgra(
                Format::Png,
                Compression::None,
                0,
                0,
                &png_rgba_odd,
                PER_SCREEN_LIMIT
            )
            .unwrap(),
            expected_bgra
        );

        let png_bytes = make_png(2, 1, png::ColorType::Rgba, &[1, 2, 3, 4, 5, 6, 7, 8]);
        let (w, h, fmt) = validate_transmitted(
            Format::Png,
            Compression::None,
            0,
            0,
            &png_bytes,
            PER_SCREEN_LIMIT,
        )
        .unwrap();
        assert_eq!((w, h, fmt), (2, 1, DecodedFormat::Rgba));
        assert_eq!(
            decode_to_rgba(
                Format::Png,
                Compression::None,
                0,
                0,
                &png_bytes,
                PER_SCREEN_LIMIT
            )
            .unwrap(),
            vec![1, 2, 3, 4, 5, 6, 7, 8]
        );

        // Grayscale PNG expands cleanly to RGBA.
        let gray_png = make_png(2, 1, png::ColorType::Grayscale, &[10, 200]);
        assert_eq!(
            decode_to_rgba(
                Format::Png,
                Compression::None,
                0,
                0,
                &gray_png,
                PER_SCREEN_LIMIT
            )
            .unwrap(),
            vec![10, 10, 10, 255, 200, 200, 200, 255]
        );

        // Zlib-wrapped PNG.
        let z_png = zlib_compress(&png_bytes);
        assert_eq!(
            validate_transmitted(
                Format::Png,
                Compression::Zlib,
                0,
                0,
                &z_png,
                PER_SCREEN_LIMIT
            )
            .unwrap(),
            (2, 1, DecodedFormat::Rgba)
        );
    }

    #[test]
    fn refuses_zlib_and_png_bombs_without_allocating_past_limit() {
        // 1x1 RGB expects 3 bytes; a 1 MiB zero-filled zlib payload stops immediately.
        let bomb = zlib_compress(&vec![0u8; 1024 * 1024]);
        assert_eq!(
            validate_transmitted(
                Format::Rgb,
                Compression::Zlib,
                1,
                1,
                &bomb,
                PER_SCREEN_LIMIT
            ),
            Err(ImageError::InvalidData)
        );

        // PNG claiming 9000x9000 (324 MB > 320 MB per-screen limit) is refused from
        // its header before pixels are allocated.
        let mut fake_png = make_png(1, 1, png::ColorType::Rgba, &[0, 0, 0, 255]);
        // Overwrite IHDR width (bytes 16..20) and height (20..24), and recompute IHDR CRC (29..33).
        fake_png[16..20].copy_from_slice(&9000u32.to_be_bytes());
        fake_png[20..24].copy_from_slice(&9000u32.to_be_bytes());
        let crc = crc32(&fake_png[12..29]);
        fake_png[29..33].copy_from_slice(&crc.to_be_bytes());
        assert_eq!(
            validate_transmitted(
                Format::Png,
                Compression::None,
                0,
                0,
                &fake_png,
                PER_SCREEN_LIMIT
            ),
            Err(ImageError::OutOfMemory)
        );

        // PNG over MAX_DIMENSION (10001x1) is DimensionsTooLarge.
        let mut wide_png = make_png(1, 1, png::ColorType::Rgba, &[0, 0, 0, 255]);
        wide_png[16..20].copy_from_slice(&10_001u32.to_be_bytes());
        let crc = crc32(&wide_png[12..29]);
        wide_png[29..33].copy_from_slice(&crc.to_be_bytes());
        assert_eq!(
            validate_transmitted(
                Format::Png,
                Compression::None,
                0,
                0,
                &wide_png,
                PER_SCREEN_LIMIT
            ),
            Err(ImageError::DimensionsTooLarge)
        );
    }

    #[test]
    fn rejects_truncated_and_corrupted_zlib_streams_for_raw_and_png() {
        let rgb = vec![10u8, 20, 30, 40, 50, 60];
        let z_rgb = zlib_compress(&rgb);
        // Dropping any of the trailing 1..=4 Adler-32 checksum bytes must fail.
        for drop_bytes in 1..=4 {
            let truncated = &z_rgb[..z_rgb.len() - drop_bytes];
            assert_eq!(
                validate_transmitted(
                    Format::Rgb,
                    Compression::Zlib,
                    2,
                    1,
                    truncated,
                    PER_SCREEN_LIMIT
                ),
                Err(ImageError::DecompressionFailed),
                "raw rgb truncated by {drop_bytes}"
            );
            assert_eq!(
                decode_to_rgba(
                    Format::Rgb,
                    Compression::Zlib,
                    2,
                    1,
                    truncated,
                    PER_SCREEN_LIMIT
                ),
                Err(ImageError::DecompressionFailed),
                "decode raw rgb truncated by {drop_bytes}"
            );
        }
        // Flipping a checksum bit must also fail.
        let mut bad_crc = z_rgb.clone();
        *bad_crc.last_mut().unwrap() ^= 0x01;
        assert_eq!(
            validate_transmitted(
                Format::Rgb,
                Compression::Zlib,
                2,
                1,
                &bad_crc,
                PER_SCREEN_LIMIT
            ),
            Err(ImageError::DecompressionFailed)
        );

        let png_bytes = make_png(2, 1, png::ColorType::Rgba, &[1, 2, 3, 4, 5, 6, 7, 8]);
        let z_png = zlib_compress(&png_bytes);
        for drop_bytes in 1..=4 {
            let truncated = &z_png[..z_png.len() - drop_bytes];
            assert_eq!(
                validate_transmitted(
                    Format::Png,
                    Compression::Zlib,
                    0,
                    0,
                    truncated,
                    PER_SCREEN_LIMIT
                ),
                Err(ImageError::DecompressionFailed),
                "zlib png truncated by {drop_bytes}"
            );
            assert_eq!(
                decode_to_rgba(
                    Format::Png,
                    Compression::Zlib,
                    0,
                    0,
                    truncated,
                    PER_SCREEN_LIMIT
                ),
                Err(ImageError::DecompressionFailed),
                "decode zlib png truncated by {drop_bytes}"
            );
        }
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for &b in bytes {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
}
