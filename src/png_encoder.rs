/// Minimal PNG encoder for RGB 8-bit images.
/// Replaces the `png` crate to reduce WASM binary size.

/// Precomputed CRC32 lookup table (polynomial 0xEDB88320).
const CRC32_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0u32;
    while i < 256 {
        let mut crc = i;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i as usize] = crc;
        i += 1;
    }
    table
};

/// CRC32 over chunk type concatenated with data.
fn crc32(chunk_type: &[u8], data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in chunk_type.iter().chain(data.iter()) {
        let idx = ((crc ^ byte as u32) & 0xFF) as usize;
        crc = CRC32_TABLE[idx] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

/// Write a PNG chunk: length (4 BE) + type (4) + data + CRC32 (4 BE).
fn write_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(chunk_type);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32(chunk_type, data).to_be_bytes());
}

/// Encode raw RGB 8-bit pixels as a minimal PNG.
///
/// `pixels` must be exactly `width * height * 3` bytes (RGB, row-major).
/// Uses zlib compression level 1 (fast).
pub fn encode_png_rgb8(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    let row_bytes = width as usize * 3;
    debug_assert_eq!(pixels.len(), row_bytes * height as usize);

    // Build filtered image data using Sub filter (type 1) for each scanline.
    // Sub filter: for each byte, store byte[i] - byte[i-3] (wrapping).
    // The first pixel (3 bytes) is stored unchanged since there is no left neighbor.
    let filtered_len = height as usize * (1 + row_bytes);
    let mut raw = Vec::with_capacity(filtered_len);
    for y in 0..height as usize {
        let row = &pixels[y * row_bytes..(y + 1) * row_bytes];
        raw.push(1); // filter byte = Sub
        // First pixel (bytes 0..2): no left neighbor, stored as-is
        for i in 0..row_bytes {
            if i < 3 {
                raw.push(row[i]);
            } else {
                raw.push(row[i].wrapping_sub(row[i - 3]));
            }
        }
    }

    // Compress with zlib wrapper (required by PNG spec).
    let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 1);

    // Allocate output: signature(8) + IHDR(25) + IDAT(12+data) + IEND(12)
    let mut out = Vec::with_capacity(8 + 25 + 12 + compressed.len() + 12);

    // PNG signature
    out.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);

    // IHDR: 13 bytes
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = 8; // bit depth
    ihdr[9] = 2; // color type: RGB
    // ihdr[10..13] = 0: compression, filter, interlace (all default)
    write_chunk(&mut out, b"IHDR", &ihdr);

    // IDAT: compressed pixel data
    write_chunk(&mut out, b"IDAT", &compressed);

    // IEND: empty
    write_chunk(&mut out, b"IEND", &[]);

    out
}
