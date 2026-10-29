//! The private keys of the instance's GitHub Apps, which the control plane
//! signs their JWTs with: PKCS#8 PEM, which WebCrypto imports
//! (docs/self-hosting.md). A key pasted into a form or a prompt can arrive
//! on one line, its line breaks turned into spaces, dropped or written as
//! `\n`: [`der`] reads it however its lines were broken.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

const BEGIN: &str = "-----BEGIN PRIVATE KEY-----";
const END: &str = "-----END PRIVATE KEY-----";

/// Why a stored key isn't one fctl reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyError {
    /// PKCS#1 (`BEGIN RSA PRIVATE KEY`), as GitHub gives keys out.
    Pkcs1,
    /// Not all of a PEM private key: its `BEGIN PRIVATE KEY` or
    /// `END PRIVATE KEY` line is missing, or there is nothing between them.
    Incomplete,
    /// Something other than base64 between those lines.
    NotBase64,
}

impl KeyError {
    /// What's wrong with the key in the variable `var`.
    pub fn message(self, var: &str) -> String {
        match self {
            Self::Pkcs1 => format!(
                "{var} must be PKCS#8 (convert with: openssl pkcs8 -topk8 -nocrypt -in key.pem)"
            ),
            Self::Incomplete => format!(
                "{var} isn't all of a PEM private key, from its BEGIN PRIVATE KEY line to its END PRIVATE KEY line"
            ),
            Self::NotBase64 => format!(
                "{var} isn't a PEM private key: there is more than base64 between its BEGIN PRIVATE KEY and END PRIVATE KEY lines"
            ),
        }
    }
}

/// The DER of PEM private key `pem`, whatever became of its line breaks;
/// also of the base64 alone, without the lines around it.
pub fn der(pem: &str) -> Result<Vec<u8>, KeyError> {
    if pem.contains("BEGIN RSA PRIVATE KEY") {
        return Err(KeyError::Pkcs1);
    }
    let body = match pem.split_once(BEGIN) {
        Some((_, rest)) => rest.split_once(END).ok_or(KeyError::Incomplete)?.0,
        None if !pem.contains("-----") => pem,
        None => return Err(KeyError::Incomplete),
    };
    // Base64 has no backslash: one is a line break written as `\n`.
    let body: String = body
        .replace("\\r", "")
        .replace("\\n", "")
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if body.is_empty() {
        return Err(KeyError::Incomplete);
    }
    STANDARD.decode(body).map_err(|_| KeyError::NotBase64)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A key's base64, in lines of 64 as PEM has it: not a real key, which
    /// only WebCrypto checks.
    const BODY: [&str; 3] = [
        "MIIBVQIBADANBgkqhkiG9w0BAQEFAASCAT8wggE7AgEAAkEAu1SU1LfVLPHCozMx",
        "H2Mo4lgOEePzNm0tRgeLezV6ffAt0gunVTLw7onLRnrq0/IzW7yWR7QkrmBL7jTK",
        "EOrFxAIDAQAB",
    ];

    fn bytes() -> Vec<u8> {
        STANDARD.decode(BODY.concat()).unwrap_or_default()
    }

    #[test]
    fn a_key_reads_however_its_lines_were_broken() {
        let lines = [&[BEGIN][..], &BODY, &[END]].concat();
        assert!(!bytes().is_empty());
        for pem in [
            // As GitHub and openssl write it, with a line break at the end.
            format!("{}\n", lines.join("\n")),
            lines.join("\r\n"),
            // Pasted on one line: the line breaks turned into spaces, or dropped.
            lines.join(" "),
            lines.concat(),
            // Kept as `\n`, as an environment file writes them, quotes and all.
            format!("\"{}\\n\"", lines.join("\\n")),
            lines.join("\\r\\n"),
            // The base64 alone.
            BODY.join("\n"),
        ] {
            assert_eq!(der(&pem), Ok(bytes()), "{pem}");
        }
    }

    #[test]
    fn what_isnt_a_whole_pkcs8_key_says_why() {
        assert_eq!(
            der("-----BEGIN RSA PRIVATE KEY-----\nMIIE\n-----END RSA PRIVATE KEY-----"),
            Err(KeyError::Pkcs1)
        );
        // Only the first line, as a prompt that reads one line keeps it.
        assert_eq!(der(BEGIN), Err(KeyError::Incomplete));
        assert_eq!(
            der(&format!("{BEGIN}\n{}", BODY[0])),
            Err(KeyError::Incomplete)
        );
        assert_eq!(der(&format!("{BEGIN}\n{END}")), Err(KeyError::Incomplete));
        assert_eq!(der(""), Err(KeyError::Incomplete));
        assert_eq!(
            der("-----BEGIN CERTIFICATE-----"),
            Err(KeyError::Incomplete)
        );
        assert_eq!(
            der(&format!("{BEGIN}\nnot a key!\n{END}")),
            Err(KeyError::NotBase64)
        );
        assert_eq!(
            KeyError::Pkcs1.message("GITHUB_APP_PRIVATE_KEY"),
            "GITHUB_APP_PRIVATE_KEY must be PKCS#8 (convert with: openssl pkcs8 -topk8 -nocrypt -in key.pem)"
        );
    }
}
