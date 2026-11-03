//! Content sniffing by magic bytes. A client-declared `Content-Type` is never trusted on its own:
//! the bytes must actually look like the declared type.

/// Detects a small set of common formats; `None` if unrecognised.
pub fn detect(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some("image/png")
    } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        Some("image/webp")
    } else if data.starts_with(b"%PDF-") {
        Some("application/pdf")
    } else if is_plain_text(data) {
        Some("text/plain")
    } else {
        None
    }
}

/// UTF-8 without NUL bytes. Only the first 8 KiB is inspected for speed; a truncated multi-byte
/// sequence at the cut is tolerated.
fn is_plain_text(data: &[u8]) -> bool {
    let sample = &data[..data.len().min(8192)];
    if sample.contains(&0) {
        return false;
    }
    match std::str::from_utf8(sample) {
        Ok(_) => true,
        Err(err) => err.error_len().is_none() && sample.len() == 8192,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_known_formats() {
        assert_eq!(
            detect(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0]),
            Some("image/png")
        );
        assert_eq!(detect(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(detect(b"RIFF\x00\x00\x00\x00WEBPVP8 "), Some("image/webp"));
        assert_eq!(detect(b"%PDF-1.7 ..."), Some("application/pdf"));
        assert_eq!(detect(b"just some text\n"), Some("text/plain"));
    }

    #[test]
    fn rejects_binary_masquerading_as_text() {
        assert_eq!(detect(&[0x4D, 0x5A, 0x90, 0x00, 0x03]), None); // PE executable header
        assert_eq!(detect(&[0xFF, 0xFE, 0x00]), None);
    }
}
