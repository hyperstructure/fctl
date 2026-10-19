//! QR codes, for the links that add an authenticator app: the web app draws
//! them and the CLI prints them, from the same modules.

use qrcode::{Color, EcLevel, QrCode};

/// The QR code of `text` (error correction M: a seventh of it can be
/// damaged), as one string per row, `1` for a dark module and `0` for a
/// light one. Scanners need a light quiet zone of four modules around it,
/// which this leaves out. None when the text is too long for a QR code.
pub fn rows(text: &str) -> Option<Vec<String>> {
    let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M).ok()?;
    let width = code.width();
    let colors = code.to_colors();
    Some(
        colors
            .chunks(width)
            .map(|row| {
                row.iter()
                    .map(|c| if *c == Color::Dark { '1' } else { '0' })
                    .collect()
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_square_with_finder_patterns() {
        let link = "otpauth://totp/fctl:ada%40example.com?secret=JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP&issuer=fctl";
        let rows = rows(link).unwrap_or_default();
        let n = rows.len();
        // Versions grow by four modules from 21.
        assert!(n >= 21 && (n - 21).is_multiple_of(4), "{n}");
        assert!(
            rows.iter()
                .all(|r| r.len() == n && r.bytes().all(|b| b == b'0' || b == b'1'))
        );
        // The finder pattern in the top left: a dark ring, a light ring, a
        // dark center.
        assert_eq!(&rows[0][..7], "1111111");
        assert_eq!(&rows[1][..7], "1000001");
        assert_eq!(&rows[3][..7], "1011101");
        assert_eq!(&rows[6][..7], "1111111");
        // And in the top right and bottom left.
        assert_eq!(&rows[0][n - 7..], "1111111");
        assert_eq!(&rows[n - 1][..7], "1111111");
    }

    #[test]
    fn too_long_is_none() {
        assert!(rows(&"x".repeat(5000)).is_none());
    }
}
