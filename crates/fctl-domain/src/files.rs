//! Files people attach to messages: limits, names, media types, and which
//! images Claude can be shown in the message itself.

/// Largest file accepted (20 MB).
pub const MAX_FILE_BYTES: u64 = 20 * 1000 * 1000;
/// Most files in one message.
pub const MAX_FILES_PER_MESSAGE: usize = 10;
/// Uploads that no message used are deleted after this long.
pub const UNSENT_TTL_MS: i64 = 24 * 60 * 60 * 1000;
/// Bytes per stored chunk (Durable Object SQLite rows hold at most 2 MB).
pub const CHUNK_BYTES: usize = 1024 * 1024;

/// Claude's API refuses images whose base64 is longer than this. Larger
/// images stay on disk, where Claude reads them with a tool that resizes.
pub const INLINE_IMAGE_MAX_BASE64: usize = 5 * 1024 * 1024;
/// Requests with many images refuse any image with a longer side, and one
/// refused image would break every later turn, so inline images stay within it.
pub const INLINE_IMAGE_MAX_SIDE: u32 = 2000;

/// Inline images in one message together stay within this much base64:
/// every later turn sends them again, and requests are capped at 32 MB.
pub const INLINE_MESSAGE_MAX_BASE64: usize = 5 * 1024 * 1024;

/// Longest file name kept, in bytes.
const MAX_NAME_BYTES: usize = 200;

/// A name that is safe as one path component: no directories, no control
/// characters, at most 200 bytes (keeping the extension), never empty.
pub fn sanitize_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = base.chars().filter(|c| !c.is_control()).collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '.') {
        return "file".to_owned();
    }
    if cleaned.len() <= MAX_NAME_BYTES {
        return cleaned.to_owned();
    }
    let (stem, ext) = match cleaned.rfind('.') {
        Some(i) if i > 0 && cleaned.len() - i <= 16 => cleaned.split_at(i),
        _ => (cleaned, ""),
    };
    let mut cut = MAX_NAME_BYTES - ext.len();
    while !stem.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}{ext}", stem[..cut].trim_end())
}

/// The media type to record: the client's, if it is a plausible
/// `type/subtype`, else one guessed from the extension. Images are
/// identified by their bytes instead (see [`sniff_image`]).
pub fn media_type(declared: Option<&str>, name: &str) -> String {
    let declared = declared
        .and_then(|d| d.split(';').next())
        .map(|d| d.trim().to_ascii_lowercase())
        .filter(|d| is_media_type(d) && d != "application/octet-stream");
    declared.unwrap_or_else(|| guess_media_type(name).to_owned())
}

fn is_media_type(s: &str) -> bool {
    let token = |t: &str| {
        !t.is_empty()
            && t.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$&-^_.+".contains(&b))
    };
    s.len() <= 100 && s.split_once('/').is_some_and(|(a, b)| token(a) && token(b))
}

/// A media type from the file extension.
pub fn guess_media_type(name: &str) -> &'static str {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "heic" => "image/heic",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "ico" => "image/x-icon",
        "pdf" => "application/pdf",
        "txt" | "log" | "text" => "text/plain",
        "md" | "markdown" => "text/markdown",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "xml" => "application/xml",
        "json" => "application/json",
        "yaml" | "yml" => "application/yaml",
        "toml" => "application/toml",
        "js" | "mjs" | "cjs" => "text/javascript",
        "ts" | "tsx" | "jsx" | "rs" | "py" | "go" | "rb" | "java" | "kt" | "swift" | "c" | "h"
        | "cc" | "cpp" | "hpp" | "cs" | "php" | "sh" | "sql" | "diff" | "patch" => "text/plain",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "tar" => "application/x-tar",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        _ => "application/octet-stream",
    }
}

/// What the first bytes of a raster image say about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageInfo {
    /// `image/png`, `image/jpeg`, `image/gif` or `image/webp`.
    pub media_type: &'static str,
    pub width: u32,
    pub height: u32,
}

/// Identifies a PNG, JPEG, GIF or WebP image from its bytes, with its pixel
/// size. Claude accepts these four formats; anything else is `None`.
pub fn sniff_image(bytes: &[u8]) -> Option<ImageInfo> {
    let info = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        png(bytes)
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        jpeg(bytes)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        gif(bytes)
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        webp(bytes)
    } else {
        None
    }?;
    (info.width > 0 && info.height > 0).then_some(info)
}

/// Whether an image can go into Claude's message as-is: a format Claude
/// accepts, within the API's size limit and [`INLINE_IMAGE_MAX_SIDE`].
pub fn inline_image(bytes: &[u8]) -> Option<ImageInfo> {
    let info = sniff_image(bytes)?;
    (base64_len(bytes.len()) <= INLINE_IMAGE_MAX_BASE64
        && info.width <= INLINE_IMAGE_MAX_SIDE
        && info.height <= INLINE_IMAGE_MAX_SIDE)
        .then_some(info)
}

/// The length of `n` bytes in base64.
pub fn base64_len(n: usize) -> usize {
    n.div_ceil(3) * 4
}

fn be16(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at + 2)?;
    Some(u32::from(u16::from_be_bytes([s[0], s[1]])))
}

fn le16(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at + 2)?;
    Some(u32::from(u16::from_le_bytes([s[0], s[1]])))
}

fn le24(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at + 3)?;
    Some(u32::from(s[0]) | u32::from(s[1]) << 8 | u32::from(s[2]) << 16)
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at + 4)?;
    Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn png(b: &[u8]) -> Option<ImageInfo> {
    // The first chunk is IHDR: width and height, big-endian.
    if b.get(12..16) != Some(b"IHDR") {
        return None;
    }
    Some(ImageInfo {
        media_type: "image/png",
        width: be32(b, 16)?,
        height: be32(b, 20)?,
    })
}

fn gif(b: &[u8]) -> Option<ImageInfo> {
    Some(ImageInfo {
        media_type: "image/gif",
        width: le16(b, 6)?,
        height: le16(b, 8)?,
    })
}

fn webp(b: &[u8]) -> Option<ImageInfo> {
    let (width, height) = match b.get(12..16)? {
        // Lossy: a VP8 key frame, 14-bit sizes after its start code.
        b"VP8 " => {
            if b.get(23..26) != Some(&[0x9d, 0x01, 0x2a]) {
                return None;
            }
            (le16(b, 26)? & 0x3fff, le16(b, 28)? & 0x3fff)
        }
        // Lossless: 14 bits each of width - 1 and height - 1.
        b"VP8L" => {
            if b.get(20) != Some(&0x2f) {
                return None;
            }
            let s = b.get(21..25)?;
            let bits = u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
            ((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1)
        }
        // Extended: 24 bits each of canvas width - 1 and height - 1.
        b"VP8X" => (le24(b, 24)? + 1, le24(b, 27)? + 1),
        _ => return None,
    };
    Some(ImageInfo {
        media_type: "image/webp",
        width,
        height,
    })
}

fn jpeg(b: &[u8]) -> Option<ImageInfo> {
    // Walk the markers to the first start-of-frame, which holds the size.
    let mut i = 2;
    loop {
        while b.get(i) == Some(&0xff) && b.get(i + 1) == Some(&0xff) {
            i += 1;
        }
        if b.get(i) != Some(&0xff) {
            return None;
        }
        let marker = *b.get(i + 1)?;
        match marker {
            0xd8 | 0x01 | 0xd0..=0xd7 => i += 2,
            // Start of scan or end of image: no frame header before it.
            0xd9 | 0xda => return None,
            0xc0..=0xcf if !matches!(marker, 0xc4 | 0xc8 | 0xcc) => {
                return Some(ImageInfo {
                    media_type: "image/jpeg",
                    height: be16(b, i + 5)?,
                    width: be16(b, i + 7)?,
                });
            }
            _ => i += 2 + be16(b, i + 2)? as usize,
        }
    }
}

/// Raster images browsers show in place; any other file is downloaded.
pub fn shows_inline(media_type: &str) -> bool {
    matches!(
        media_type,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    )
}

/// A `Content-Disposition` value: an ASCII fallback name and the UTF-8 one
/// (RFC 6266).
pub fn content_disposition(name: &str, inline: bool) -> String {
    let fallback: String = name
        .chars()
        .map(|c| {
            if c.is_ascii() && !c.is_ascii_control() && c != '"' && c != '\\' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut encoded = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&b) {
            encoded.push(b as char);
        } else {
            encoded.push_str(&format!("%{b:02X}"));
        }
    }
    let kind = if inline { "inline" } else { "attachment" };
    format!("{kind}; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

/// What a `Range` header asks of a file (RFC 9110, 14.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ByteRange {
    /// The whole file: no header, or one the server may ignore (several
    /// ranges, another unit, a malformed one).
    Whole,
    /// Bytes `start..=end`, all within the file.
    Part { start: u64, end: u64 },
    /// A range that starts past the end (`416 Range Not Satisfiable`).
    Unsatisfiable,
}

/// The bytes a `Range` header asks for, of a file of `size` bytes. Media
/// players ask for one range at a time, and Safari plays a video only from
/// a server that answers them.
pub fn byte_range(header: Option<&str>, size: u64) -> ByteRange {
    let Some(set) = header.and_then(|h| {
        let (unit, set) = h.split_once('=')?;
        unit.trim().eq_ignore_ascii_case("bytes").then_some(set)
    }) else {
        return ByteRange::Whole;
    };
    let Some((first, last)) = set.split_once('-').filter(|_| !set.contains(',')) else {
        return ByteRange::Whole;
    };
    let (first, last) = (first.trim(), last.trim());
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    // Too many digits for a u64 is still past the end of any file.
    let number = |s: &str| s.parse::<u64>().unwrap_or(u64::MAX);
    if first.is_empty() {
        // `bytes=-500`: the last 500 bytes.
        if !digits(last) {
            return ByteRange::Whole;
        }
        let n = number(last);
        if n == 0 || size == 0 {
            return ByteRange::Unsatisfiable;
        }
        return ByteRange::Part {
            start: size.saturating_sub(n),
            end: size - 1,
        };
    }
    // `bytes=500-999`, or `bytes=500-` to the end.
    if !digits(first) || !(last.is_empty() || digits(last)) {
        return ByteRange::Whole;
    }
    let start = number(first);
    let end = if last.is_empty() {
        u64::MAX
    } else {
        number(last)
    };
    if end < start {
        ByteRange::Whole
    } else if start >= size {
        ByteRange::Unsatisfiable
    } else {
        ByteRange::Part {
            start,
            end: end.min(size - 1),
        }
    }
}

/// Where bytes `start..=end` of a stored file are: its chunks `first..=last`
/// and the offset of `start` in chunk `first`. Every chunk but a file's last
/// holds [`CHUNK_BYTES`].
pub fn chunk_span(start: u64, end: u64) -> (u64, u64, usize) {
    let chunk = CHUNK_BYTES as u64;
    (start / chunk, end / chunk, (start % chunk) as usize)
}

/// `912 B`, `245 KB`, `1.2 MB`: decimal units, as file managers show them.
pub fn human_size(bytes: u64) -> String {
    const KB: u64 = 1000;
    const MB: u64 = 1000 * KB;
    const GB: u64 = 1000 * MB;
    if bytes < KB {
        format!("{bytes} B")
    } else if bytes < MB {
        format!("{} KB", (bytes + KB / 2) / KB)
    } else if bytes < 10 * MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes < GB {
        format!("{} MB", (bytes + MB / 2) / MB)
    } else {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_bytes(w: u32, h: u32) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        b.extend_from_slice(&w.to_be_bytes());
        b.extend_from_slice(&h.to_be_bytes());
        b.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        b
    }

    fn jpeg_bytes(w: u16, h: u16) -> Vec<u8> {
        let mut b = vec![0xff, 0xd8];
        // APP0 (JFIF), 16 bytes long, then fill bytes before the next marker.
        b.extend_from_slice(&[0xff, 0xe0, 0x00, 0x10]);
        b.extend_from_slice(b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
        b.extend_from_slice(&[0xff, 0xff]);
        // DHT is not a frame header even though it is 0xc4.
        b.extend_from_slice(&[0xc4, 0x00, 0x03, 0x00]);
        // SOF2 (progressive): length, precision, height, width.
        b.extend_from_slice(&[0xff, 0xc2, 0x00, 0x11, 0x08]);
        b.extend_from_slice(&h.to_be_bytes());
        b.extend_from_slice(&w.to_be_bytes());
        b.extend_from_slice(&[0x03; 12]);
        b
    }

    #[test]
    fn sanitizes_names() {
        assert_eq!(sanitize_name("screenshot.png"), "screenshot.png");
        assert_eq!(sanitize_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_name("C:\\Users\\ada\\notes.txt"), "notes.txt");
        assert_eq!(sanitize_name("  a\u{0}b\nc.md "), "abc.md");
        assert_eq!(sanitize_name(""), "file");
        assert_eq!(sanitize_name(".."), "file");
        assert_eq!(sanitize_name("dir/"), "file");
        assert_eq!(sanitize_name(".env"), ".env");
        assert_eq!(
            sanitize_name("Skärmavbild 2026.png"),
            "Skärmavbild 2026.png"
        );
        let long = format!("{}.tar.gz", "å".repeat(150));
        let short = sanitize_name(&long);
        assert!(short.len() <= MAX_NAME_BYTES, "{}", short.len());
        assert!(short.ends_with("å.gz") || short.ends_with(".gz"), "{short}");
        assert_eq!(sanitize_name(&"x".repeat(300)).len(), MAX_NAME_BYTES);
    }

    #[test]
    fn picks_media_types() {
        assert_eq!(
            media_type(Some("text/plain; charset=utf-8"), "a"),
            "text/plain"
        );
        assert_eq!(media_type(Some("Application/PDF"), "a"), "application/pdf");
        assert_eq!(
            media_type(Some("application/octet-stream"), "a.csv"),
            "text/csv"
        );
        assert_eq!(media_type(Some("not a type"), "a.JPG"), "image/jpeg");
        assert_eq!(media_type(Some("text/<script>"), "a.txt"), "text/plain");
        assert_eq!(media_type(None, "Makefile"), "application/octet-stream");
        assert_eq!(media_type(None, "lib.rs"), "text/plain");
    }

    #[test]
    fn sniffs_png() {
        let info = sniff_image(&png_bytes(1280, 720));
        assert_eq!(
            info,
            Some(ImageInfo {
                media_type: "image/png",
                width: 1280,
                height: 720
            })
        );
        assert_eq!(sniff_image(&png_bytes(0, 10)), None);
        assert_eq!(sniff_image(b"\x89PNG\r\n\x1a\n\0\0"), None);
    }

    #[test]
    fn sniffs_jpeg_past_other_markers() {
        let info = sniff_image(&jpeg_bytes(3024, 4032));
        assert_eq!(
            info.map(|i| (i.media_type, i.width, i.height)),
            Some(("image/jpeg", 3024, 4032))
        );
        // Truncated before the frame header.
        assert_eq!(sniff_image(&jpeg_bytes(10, 10)[..30]), None);
        // A scan before any frame header.
        assert_eq!(sniff_image(&[0xff, 0xd8, 0xff, 0xda, 0, 2]), None);
    }

    #[test]
    fn sniffs_gif_and_webp() {
        let gif = b"GIF89a\x40\x01\xf0\x00\x00\x00\x00";
        assert_eq!(
            sniff_image(gif).map(|i| (i.width, i.height)),
            Some((320, 240))
        );
        let mut lossy = b"RIFF\0\0\0\0WEBPVP8 \0\0\0\0\0\0\0\x9d\x01\x2a".to_vec();
        lossy.extend_from_slice(&[0x80, 0x02, 0xe0, 0x01]);
        assert_eq!(
            sniff_image(&lossy).map(|i| (i.media_type, i.width, i.height)),
            Some(("image/webp", 640, 480))
        );
        // Lossless: width 400 and height 300, stored minus one in 14-bit fields.
        let bits: u32 = 399 | (299 << 14);
        let mut lossless = b"RIFF\0\0\0\0WEBPVP8L\0\0\0\0\x2f".to_vec();
        lossless.extend_from_slice(&bits.to_le_bytes());
        assert_eq!(
            sniff_image(&lossless).map(|i| (i.width, i.height)),
            Some((400, 300))
        );
        let mut extended = b"RIFF\0\0\0\0WEBPVP8X\0\0\0\0\0\0\0\0".to_vec();
        extended.extend_from_slice(&[0xff, 0x0f, 0x00, 0x37, 0x08, 0x00]);
        assert_eq!(
            sniff_image(&extended).map(|i| (i.width, i.height)),
            Some((4096, 2104))
        );
        assert_eq!(sniff_image(b"RIFF\0\0\0\0WAVEfmt "), None);
        assert_eq!(
            sniff_image(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"),
            None
        );
    }

    #[test]
    fn inlines_only_images_claude_accepts() {
        assert!(inline_image(&png_bytes(2000, 1200)).is_some());
        assert!(inline_image(&png_bytes(2001, 1200)).is_none());
        assert!(inline_image(&jpeg_bytes(1200, 2400)).is_none());
        assert!(inline_image(b"%PDF-1.7").is_none());
        let mut big = png_bytes(800, 600);
        big.resize(INLINE_IMAGE_MAX_BASE64 / 4 * 3 + 1, 0);
        assert!(inline_image(&big).is_none());
        big.truncate(INLINE_IMAGE_MAX_BASE64 / 4 * 3);
        assert!(inline_image(&big).is_some());
    }

    #[test]
    fn content_dispositions() {
        assert_eq!(
            content_disposition("shot.png", true),
            "inline; filename=\"shot.png\"; filename*=UTF-8''shot.png"
        );
        assert_eq!(
            content_disposition("Skärm \"1\".pdf", false),
            "attachment; filename=\"Sk_rm _1_.pdf\"; filename*=UTF-8''Sk%C3%A4rm%20%221%22.pdf"
        );
        assert!(shows_inline("image/webp"));
        assert!(!shows_inline("image/svg+xml"));
        assert!(!shows_inline("text/html"));
    }

    #[test]
    fn reads_byte_ranges() {
        use ByteRange::*;
        let r = |h: &str| byte_range(Some(h), 1000);
        // What Safari and Chrome ask for before and while playing.
        assert_eq!(r("bytes=0-1"), Part { start: 0, end: 1 });
        assert_eq!(r("bytes=0-"), Part { start: 0, end: 999 });
        assert_eq!(
            r("bytes=500-"),
            Part {
                start: 500,
                end: 999
            }
        );
        assert_eq!(
            r("bytes=900-2000"),
            Part {
                start: 900,
                end: 999
            }
        );
        assert_eq!(
            r("bytes=999-999"),
            Part {
                start: 999,
                end: 999
            }
        );
        assert_eq!(
            r("bytes=-100"),
            Part {
                start: 900,
                end: 999
            }
        );
        assert_eq!(r("bytes=-5000"), Part { start: 0, end: 999 });
        assert_eq!(r("Bytes = 10-20"), Part { start: 10, end: 20 });
        assert_eq!(
            r("bytes=0-99999999999999999999999"),
            Part { start: 0, end: 999 }
        );
        // Past the end.
        assert_eq!(r("bytes=1000-"), Unsatisfiable);
        assert_eq!(r("bytes=1000-1001"), Unsatisfiable);
        assert_eq!(r("bytes=99999999999999999999999-"), Unsatisfiable);
        assert_eq!(r("bytes=-0"), Unsatisfiable);
        assert_eq!(byte_range(Some("bytes=0-"), 0), Unsatisfiable);
        assert_eq!(byte_range(Some("bytes=-1"), 0), Unsatisfiable);
        // Answered with the whole file, as servers may.
        assert_eq!(byte_range(None, 1000), Whole);
        assert_eq!(r("bytes=0-1,5-6"), Whole);
        assert_eq!(r("items=0-1"), Whole);
        assert_eq!(r("bytes=5-1"), Whole);
        assert_eq!(r("bytes=-"), Whole);
        assert_eq!(r("bytes=+1-2"), Whole);
        assert_eq!(r("bytes=a-b"), Whole);
        assert_eq!(r("bytes 0-1"), Whole);
        assert_eq!(r(""), Whole);
    }

    #[test]
    fn finds_ranges_in_chunks() {
        let c = CHUNK_BYTES as u64;
        assert_eq!(chunk_span(0, 1), (0, 0, 0));
        assert_eq!(chunk_span(0, c - 1), (0, 0, 0));
        assert_eq!(chunk_span(c - 1, c), (0, 1, CHUNK_BYTES - 1));
        assert_eq!(chunk_span(c, 3 * c + 5), (1, 3, 0));
        assert_eq!(chunk_span(2 * c + 7, 2 * c + 7), (2, 2, 7));
    }

    #[test]
    fn formats_sizes() {
        assert_eq!(human_size(912), "912 B");
        assert_eq!(human_size(245_300), "245 KB");
        assert_eq!(human_size(1_234_567), "1.2 MB");
        assert_eq!(human_size(MAX_FILE_BYTES), "20 MB");
        assert_eq!(human_size(3_200_000_000), "3.2 GB");
    }
}
