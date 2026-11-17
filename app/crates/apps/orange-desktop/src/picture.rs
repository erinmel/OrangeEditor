//! Saving what the window shows as a PNG file, for `--screenshot`.
//!
//! The file is written without compression: a picture of the window is made rarely,
//! and this keeps an image library out of the program.

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for byte in bytes {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    let length = u32::try_from(data.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&length.to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(out.get(start..).unwrap_or_default());
    out.extend_from_slice(&crc.to_be_bytes());
}

/// A PNG file of `width` by `height` pixels from rows of red, green, blue and alpha
/// bytes. `None` when the pixels are not that many or the picture is too large.
pub(crate) fn png(width: usize, height: usize, rgba: &[u8]) -> Option<Vec<u8>> {
    let row = width.checked_mul(4)?;
    if rgba.len() != row.checked_mul(height)? || width == 0 || height == 0 {
        return None;
    }
    // Each row starts with the filter it uses: none.
    let mut raw = Vec::with_capacity(rgba.len() + height);
    for line in rgba.chunks(row) {
        raw.push(0);
        raw.extend_from_slice(line);
    }
    // A zlib stream of stored blocks, each at most 65,535 bytes.
    let mut data = vec![0x78, 0x01];
    let blocks = raw.chunks(65_535).count();
    for (index, block) in raw.chunks(65_535).enumerate() {
        let length = u16::try_from(block.len()).ok()?;
        data.push(u8::from(index + 1 == blocks));
        data.extend_from_slice(&length.to_le_bytes());
        data.extend_from_slice(&(!length).to_le_bytes());
        data.extend_from_slice(block);
    }
    data.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&u32::try_from(width).ok()?.to_be_bytes());
    header.extend_from_slice(&u32::try_from(height).ok()?.to_be_bytes());
    // Eight bits per channel, color with alpha, no interlacing.
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &header);
    chunk(&mut out, b"IDAT", &data);
    chunk(&mut out, b"IEND", &[]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_a_png_file_with_the_checksums_the_format_defines() {
        // The check values of the two checksums, from their specifications.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
        let pixels = [255, 0, 0, 255, 0, 255, 0, 255];
        let file = png(2, 1, &pixels).unwrap();
        assert_eq!(&file[..8], [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        // IHDR: 13 bytes, width 2, height 1.
        assert_eq!(
            &file[8..24],
            [0, 0, 0, 13, b'I', b'H', b'D', b'R', 0, 0, 0, 2, 0, 0, 0, 1]
        );
        assert_eq!(&file[file.len() - 8..file.len() - 4], b"IEND");
        // One stored block holding the filter byte and the eight pixel bytes.
        let data = &file[41..];
        assert_eq!(&data[..7], [0x78, 0x01, 1, 9, 0, 0xF6, 0xFF]);
        assert_eq!(&data[7..16], [0, 255, 0, 0, 255, 0, 255, 0, 255]);
        assert_eq!(png(2, 2, &pixels), None);
        assert_eq!(png(0, 0, &[]), None);
    }
}
