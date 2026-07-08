//! Media-type sniffing for the `assets/` tree.
//!
//! Assets are sorted into human-browsable family folders
//! (`assets/images/…`, `assets/models/…`) with a real extension, purely from
//! their **magic bytes** — the project tree is the deliverable, so a game-team
//! recipient should be able to browse it without Fantaisa (spec 09 user
//! stories). Folder and extension are *projections*: on read the id encoded in
//! the filename is the only thing that matters, so a mis-sniffed (or manually
//! moved) asset still round-trips losslessly.

/// Family folders under `assets/`, in the order the scaffold creates them.
pub(crate) const MEDIA_DIRS: [&str; 7] = [
    "images", "video", "audio", "models", "svg", "fonts", "other",
];

/// Sniff `(family folder, extension)` from the leading bytes of an asset.
/// Unknown content falls back to `("other", "bin")`.
pub(crate) fn sniff_media(bytes: &[u8]) -> (&'static str, &'static str) {
    // Images.
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return ("images", "png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return ("images", "jpg");
    }
    if bytes.starts_with(b"GIF8") {
        return ("images", "gif");
    }
    // RIFF containers: WebP (image) and WAV (audio) share the outer magic.
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" {
        match &bytes[8..12] {
            b"WEBP" => return ("images", "webp"),
            b"WAVE" => return ("audio", "wav"),
            _ => {}
        }
    }
    // Video. ISO base-media files put `ftyp` at offset 4; the brand at offset
    // 8 distinguishes QuickTime from the MP4 family.
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        return if &bytes[8..10] == b"qt" {
            ("video", "mov")
        } else {
            ("video", "mp4")
        };
    }
    if bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        // EBML header — Matroska/WebM family.
        return ("video", "webm");
    }
    // Audio.
    if bytes.starts_with(b"ID3") {
        return ("audio", "mp3");
    }
    if bytes.starts_with(b"OggS") {
        return ("audio", "ogg");
    }
    if bytes.starts_with(b"fLaC") {
        return ("audio", "flac");
    }
    // 3D models.
    if bytes.starts_with(b"glTF") {
        return ("models", "glb");
    }
    // Fonts.
    if bytes.starts_with(&[0x00, 0x01, 0x00, 0x00]) {
        return ("fonts", "ttf");
    }
    if bytes.starts_with(b"OTTO") {
        return ("fonts", "otf");
    }
    if bytes.starts_with(b"wOFF") {
        return ("fonts", "woff");
    }
    if bytes.starts_with(b"wOF2") {
        return ("fonts", "woff2");
    }
    // Text-shaped formats: sniff a bounded head as (lossy) UTF-8.
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]);
    let trimmed = head.trim_start();
    if trimmed.starts_with("<svg") || (trimmed.starts_with("<?xml") && head.contains("<svg")) {
        return ("svg", "svg");
    }
    if trimmed.starts_with('{') && head.contains("\"asset\"") {
        // .gltf is a JSON document whose required top-level key is "asset".
        return ("models", "gltf");
    }
    // Raw MPEG audio without an ID3 tag: 11-bit frame sync. Checked last among
    // binary sniffs because two leading 0xFF bits are weak evidence.
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0 {
        return ("audio", "mp3");
    }
    ("other", "bin")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_every_family() {
        let cases: &[(&[u8], (&str, &str))] = &[
            (b"\x89PNG\r\n\x1a\n....", ("images", "png")),
            (&[0xFF, 0xD8, 0xFF, 0xE0, 0x00], ("images", "jpg")),
            (b"GIF89a..", ("images", "gif")),
            (b"RIFF\x10\x00\x00\x00WEBPVP8 ", ("images", "webp")),
            (b"RIFF\x10\x00\x00\x00WAVEfmt ", ("audio", "wav")),
            (b"\x00\x00\x00\x18ftypisom....", ("video", "mp4")),
            (b"\x00\x00\x00\x14ftypqt  ....", ("video", "mov")),
            (&[0x1A, 0x45, 0xDF, 0xA3, 0x01], ("video", "webm")),
            (b"ID3\x03\x00....", ("audio", "mp3")),
            (&[0xFF, 0xFB, 0x90, 0x00], ("audio", "mp3")),
            (b"OggS....", ("audio", "ogg")),
            (b"fLaC....", ("audio", "flac")),
            (b"glTF\x02\x00\x00\x00", ("models", "glb")),
            (br#"{"asset":{"version":"2.0"}}"#, ("models", "gltf")),
            (b"<svg xmlns='x'></svg>", ("svg", "svg")),
            (b"<?xml version=\"1.0\"?><svg/>", ("svg", "svg")),
            (&[0x00, 0x01, 0x00, 0x00, 0x00], ("fonts", "ttf")),
            (b"OTTO....", ("fonts", "otf")),
            (b"wOFF....", ("fonts", "woff")),
            (b"wOF2....", ("fonts", "woff2")),
            (b"plain unknown bytes", ("other", "bin")),
            (b"", ("other", "bin")),
        ];
        for (bytes, expected) in cases {
            assert_eq!(sniff_media(bytes), *expected, "bytes: {bytes:?}");
        }
    }
}
