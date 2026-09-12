//! The preview image a `.scmap` carries, for a map that is not in the vault yet.
//!
//! The upload dialog shows what is about to be published, and for a map that
//! nobody has uploaded the vault has no thumbnail to offer: the picture has to
//! come out of the file on disk.
//!
//! # The format, as measured
//!
//! The header is the same in every version (`20-Domains/FAF-Engine/formats/scmap`):
//!
//! ```text
//! 16 bytes  magic, beginning `Map\x1a`
//! f32       width, discarded
//! f32       height, discarded
//! 6 bytes   zero padding
//! u32       preview length
//! ..        preview, an embedded DDS
//! u32       version
//! ```
//!
//! The preview is **not** DXT-compressed, which is the thing worth writing
//! down: a survey of the 400 real `.scmap` files on a developer machine found
//! every one of them carrying the same 256 by 256 uncompressed image, 262272
//! bytes, `fourcc` zero, flags `0x41` (RGB with alpha), 32 bits per pixel, and
//! the masks `R=0x00ff0000 G=0x0000ff00 B=0x000000ff A=0xff000000`. On a
//! little-endian machine those masks are the byte order B, G, R, A.
//!
//! So there is no block decompression here and no need for a decoder crate.
//! The DDS header is skipped, the channels are swapped, and the result is
//! wrapped in a PNG so it reaches the webview as an ordinary image.

use std::io::Read;
use std::path::Path;

use base64::Engine as _;

/// `Map\x1a`. The rest of the 16 byte magic varies.
const SCMAP_MAGIC: &[u8; 4] = b"Map\x1a";
/// Magic, two floats, six zero bytes: the preview length sits here.
const PREVIEW_LENGTH_AT: u64 = 16 + 4 + 4 + 6;
/// Every DDS carries `DDS ` and then a 124 byte header.
const DDS_HEADER_BYTES: usize = 128;
/// Refuse a preview that claims to be bigger than any real one. The largest
/// measured is 262272; this leaves room for a 1024 by 1024 without letting a
/// corrupt length allocate freely.
const MAX_PREVIEW_BYTES: u32 = 8 * 1024 * 1024;

/// The preview of `path`, as a `data:image/png;base64,...` URL.
pub fn preview_data_url(path: &Path) -> Result<String, String> {
    let image = read_preview(path)?;
    let png = encode_png(image.width, image.height, &image.rgba);
    Ok(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    ))
}

/// `Debug` prints the dimensions and the byte count, never the pixels: a
/// failing assertion should not put a quarter of a megabyte on the terminal.
struct Preview {
    width: u32,
    height: u32,
    /// Four bytes per pixel, in that order.
    rgba: Vec<u8>,
}

impl std::fmt::Debug for Preview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Preview")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.rgba.len())
            .finish()
    }
}

fn read_preview(path: &Path) -> Result<Preview, String> {
    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("could not open the map file: {error}"))?;

    let mut magic = [0_u8; 4];
    read_exact(&mut file, &mut magic, "the map file's header")?;
    if &magic != SCMAP_MAGIC {
        return Err("that file does not look like a Supreme Commander map".to_string());
    }

    seek(&mut file, PREVIEW_LENGTH_AT)?;
    let length = read_u32(&mut file, "the preview's length")?;
    if length == 0 {
        return Err("the map carries no preview image".to_string());
    }
    if length > MAX_PREVIEW_BYTES {
        return Err("the map's preview image is implausibly large".to_string());
    }

    let mut dds = vec![0_u8; length as usize];
    read_exact(&mut file, &mut dds, "the preview image")?;
    decode_dds(&dds)
}

/// Uncompressed 32 bit DDS only, which is the only kind a `.scmap` was found to
/// carry. Anything else is refused by name rather than decoded badly.
fn decode_dds(dds: &[u8]) -> Result<Preview, String> {
    if dds.len() < DDS_HEADER_BYTES || &dds[0..4] != b"DDS " {
        return Err("the map's preview image is not a DDS".to_string());
    }
    let u32_at = |offset: usize| -> u32 {
        u32::from_le_bytes([
            dds[offset],
            dds[offset + 1],
            dds[offset + 2],
            dds[offset + 3],
        ])
    };

    // Offsets are from the start of the file: 4 for `DDS `, then the 124 byte
    // header whose fields are at their documented places.
    let height = u32_at(12);
    let width = u32_at(16);
    let pixel_flags = u32_at(80);
    let four_cc = &dds[84..88];
    let bit_count = u32_at(88);

    const DDPF_RGB: u32 = 0x40;
    if four_cc != [0, 0, 0, 0] || pixel_flags & DDPF_RGB == 0 || bit_count != 32 {
        return Err("the map's preview image is in a format this client cannot read".to_string());
    }

    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| "the map's preview image has an impossible size".to_string())?;
    let expected = pixels
        .checked_mul(4)
        .ok_or_else(|| "the map's preview image has an impossible size".to_string())?;
    let body = &dds[DDS_HEADER_BYTES..];
    if body.len() < expected {
        return Err("the map's preview image is shorter than it claims".to_string());
    }

    // B, G, R, A on disk to R, G, B, A in the PNG. The alpha byte is carried
    // through rather than forced opaque: a preview with transparent corners is
    // a preview the map actually has.
    let mut rgba = Vec::with_capacity(expected);
    let (chunks, _) = body[..expected].as_chunks::<4>();
    for [blue, green, red, alpha] in chunks {
        rgba.extend_from_slice(&[*red, *green, *blue, *alpha]);
    }
    Ok(Preview {
        width,
        height,
        rgba,
    })
}

fn seek(file: &mut std::fs::File, to: u64) -> Result<(), String> {
    use std::io::Seek as _;
    file.seek(std::io::SeekFrom::Start(to))
        .map(|_| ())
        .map_err(|error| format!("could not read the map file: {error}"))
}

fn read_exact(file: &mut impl Read, into: &mut [u8], what: &str) -> Result<(), String> {
    file.read_exact(into)
        .map_err(|error| format!("could not read {what}: {error}"))
}

fn read_u32(file: &mut impl Read, what: &str) -> Result<u32, String> {
    let mut bytes = [0_u8; 4];
    read_exact(file, &mut bytes, what)?;
    Ok(u32::from_le_bytes(bytes))
}

/// A PNG carrying `rgba` verbatim.
///
/// Written here rather than taken from a crate. The pixels are already exactly
/// what a PNG stores, so the whole job is a header, a zlib stream and three
/// checksums; a dependency for that is a supplier to vet for no work saved.
///
/// The zlib stream uses stored blocks, so this compresses nothing. A 256 by 256
/// preview is 256 KiB either way, it is built once when a dialog opens, and it
/// never leaves the machine.
fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(rgba.len() + height as usize);
    for row in rgba.chunks_exact(width as usize * 4) {
        raw.push(0); // filter: none
        raw.extend_from_slice(row);
    }

    let mut png = Vec::new();
    png.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8 bits, RGBA, no interlace
    write_chunk(&mut png, b"IHDR", &ihdr);
    write_chunk(&mut png, b"IDAT", &zlib_stored(&raw));
    write_chunk(&mut png, b"IEND", &[]);
    png
}

fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    let mut crc_over = Vec::with_capacity(4 + body.len());
    crc_over.extend_from_slice(kind);
    crc_over.extend_from_slice(body);
    out.extend_from_slice(&crc32(&crc_over).to_be_bytes());
}

/// A zlib stream of stored (uncompressed) deflate blocks.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01]; // deflate, 32 KiB window, no preset dict
    let mut chunks = data.chunks(0xffff).peekable();
    if data.is_empty() {
        out.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    while let Some(chunk) = chunks.next() {
        let last = chunks.peek().is_none();
        out.push(u8::from(last));
        let len = chunk.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(chunk);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1_u32, 0_u32);
    for byte in bytes {
        a = (a + u32::from(*byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two checksums, against values published with their specifications.
    #[test]
    fn the_checksums_match_their_specifications() {
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
    }

    fn uncompressed_dds(width: u32, height: u32, body: &[u8]) -> Vec<u8> {
        let mut dds = vec![0_u8; DDS_HEADER_BYTES];
        dds[0..4].copy_from_slice(b"DDS ");
        dds[12..16].copy_from_slice(&height.to_le_bytes());
        dds[16..20].copy_from_slice(&width.to_le_bytes());
        dds[80..84].copy_from_slice(&0x41_u32.to_le_bytes());
        dds[88..92].copy_from_slice(&32_u32.to_le_bytes());
        dds.extend_from_slice(body);
        dds
    }

    #[test]
    fn the_channels_are_swapped_from_bgra_to_rgba() {
        // One pixel: blue 1, green 2, red 3, alpha 4 as stored.
        let preview = decode_dds(&uncompressed_dds(1, 1, &[1, 2, 3, 4])).expect("a valid dds");
        assert_eq!(preview.rgba, vec![3, 2, 1, 4]);
        assert_eq!((preview.width, preview.height), (1, 1));
    }

    #[test]
    fn a_compressed_preview_is_refused_rather_than_misread() {
        let mut dxt5 = uncompressed_dds(4, 4, &[0; 16]);
        dxt5[84..88].copy_from_slice(b"DXT5");
        let refused = decode_dds(&dxt5).expect_err("DXT must not be decoded as raw pixels");
        assert!(refused.contains("cannot read"));
    }

    #[test]
    fn a_preview_shorter_than_it_claims_is_refused() {
        let truncated = uncompressed_dds(16, 16, &[0; 8]);
        assert!(decode_dds(&truncated).is_err());
    }

    #[test]
    fn something_that_is_not_a_dds_is_refused() {
        assert!(decode_dds(b"not a dds at all, not even long enough").is_err());
    }

    /// The bytes a decoder would need to find: signature, the three chunk
    /// names, and the dimensions written big-endian in the header.
    #[test]
    fn the_png_is_shaped_like_a_png() {
        let png = encode_png(2, 1, &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(
            &png[0..8],
            &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]
        );
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(&png[16..20], &2_u32.to_be_bytes());
        assert_eq!(&png[20..24], &1_u32.to_be_bytes());
        assert!(png.windows(4).any(|w| w == b"IDAT"));
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
    }
}
