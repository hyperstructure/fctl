//! AWS Signature Version 4, as S3 takes it in a request's headers
//! (docs.aws.amazon.com/AmazonS3/latest/API/sig-v4-header-based-auth.html):
//! what a host signs its requests to the bucket that keeps its sessions'
//! snapshots with (docs/architecture.md#session-snapshots). S3-compatible
//! services (R2, MinIO, Google Cloud Storage's XML API) take the same.

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

/// Keys that sign requests: an access key, its secret, and the session
/// token of temporary ones (a role's).
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("access_key_id", &self.access_key_id)
            .finish_non_exhaustive()
    }
}

/// A request to sign.
#[derive(Clone, Debug)]
pub struct Request<'a> {
    pub method: &'a str,
    /// The host it goes to: `examplebucket.s3.amazonaws.com`.
    pub host: &'a str,
    /// Its path as is, not yet encoded: `/test$file.text`.
    pub path: &'a str,
    /// Its query's pairs as they are, not yet encoded.
    pub query: &'a [(&'a str, &'a str)],
    /// Headers besides `host` and the `x-amz-` ones this adds, which are
    /// signed too.
    pub headers: &'a [(&'a str, &'a str)],
    /// The SHA-256 of its body in hex, or `UNSIGNED-PAYLOAD`.
    pub payload_sha256: &'a str,
}

/// The SHA-256 of an empty body, in hex.
pub const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// A body's SHA-256 in hex.
pub fn sha256_hex(body: &[u8]) -> String {
    hex::encode(Sha256::digest(body))
}

/// `s` as SigV4 encodes it: every byte but letters, digits and `-._~` as
/// `%XX`, and `/` too unless `keep_slash` (a path's).
pub fn encode(s: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char);
            }
            b'/' if keep_slash => out.push('/'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn hmac(key: &[u8], data: &str) -> Vec<u8> {
    match Hmac::<Sha256>::new_from_slice(key) {
        Ok(mut mac) => {
            mac.update(data.as_bytes());
            mac.finalize().into_bytes().to_vec()
        }
        // HMAC takes keys of any length.
        Err(_) => Vec::new(),
    }
}

/// The headers that sign `req` for `region`'s `service` (`s3`) at `at_ms`
/// (milliseconds since the epoch), as
/// `(name, value)`: `x-amz-date`, `x-amz-content-sha256`,
/// `x-amz-security-token` for temporary credentials, and `authorization`.
/// Send them with the request, beside its own.
pub fn sign(
    req: &Request<'_>,
    creds: &Credentials,
    region: &str,
    service: &str,
    at_ms: i64,
) -> Vec<(String, String)> {
    // 2013-05-24T00:00:00.000Z: 20130524 and 20130524T000000Z.
    let iso: String = crate::account::iso8601(at_ms)
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    let date = iso.get(..8).unwrap_or_default().to_owned();
    let stamp = format!("{}Z", iso.get(..15).unwrap_or_default());
    let mut amz = vec![
        (
            "x-amz-content-sha256".to_owned(),
            req.payload_sha256.to_owned(),
        ),
        ("x-amz-date".to_owned(), stamp.clone()),
    ];
    if let Some(token) = &creds.session_token {
        amz.push(("x-amz-security-token".to_owned(), token.clone()));
    }
    // Lowercase names, values with their runs of spaces made one.
    let mut headers: Vec<(String, String)> = req
        .headers
        .iter()
        .map(|(n, v)| {
            (
                n.to_ascii_lowercase(),
                v.split_whitespace().collect::<Vec<_>>().join(" "),
            )
        })
        .chain(std::iter::once(("host".to_owned(), req.host.to_owned())))
        .chain(amz.iter().cloned())
        .collect();
    headers.sort();
    let canonical_headers: String = headers.iter().map(|(n, v)| format!("{n}:{v}\n")).collect();
    let signed_headers = headers
        .iter()
        .map(|(n, _)| n.as_str())
        .collect::<Vec<_>>()
        .join(";");
    let mut query: Vec<(String, String)> = req
        .query
        .iter()
        .map(|(k, v)| (encode(k, false), encode(v, false)))
        .collect();
    query.sort();
    let canonical_query = query
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    let canonical_request = format!(
        "{}\n{}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{}",
        req.method,
        encode(req.path, true),
        req.payload_sha256
    );
    let scope = format!("{date}/{region}/{service}/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{stamp}\n{scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );
    let key = [date.as_str(), region, service, "aws4_request"]
        .iter()
        .fold(
            format!("AWS4{}", creds.secret_access_key).into_bytes(),
            |key, part| hmac(&key, part),
        );
    let signature = hex::encode(hmac(&key, &string_to_sign));
    amz.push((
        "authorization".to_owned(),
        format!(
            "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
            creds.access_key_id
        ),
    ));
    amz
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example() -> Credentials {
        Credentials {
            access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            session_token: None,
        }
    }

    /// 2013-05-24T00:00:00Z, the examples' time.
    fn at() -> i64 {
        1_369_353_600_000
    }

    fn signature(headers: &[(String, String)]) -> String {
        headers
            .iter()
            .find(|(n, _)| n == "authorization")
            .and_then(|(_, v)| v.split("Signature=").nth(1))
            .unwrap_or_default()
            .to_owned()
    }

    /// AWS's examples of signing S3 requests, with their signatures.
    #[test]
    fn signs_as_aws_examples_do() {
        let get = sign(
            &Request {
                method: "GET",
                host: "examplebucket.s3.amazonaws.com",
                path: "/test.txt",
                query: &[],
                headers: &[("Range", "bytes=0-9")],
                payload_sha256: EMPTY_SHA256,
            },
            &example(),
            "us-east-1",
            "s3",
            at(),
        );
        assert_eq!(
            signature(&get),
            "f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
        let put = sign(
            &Request {
                method: "PUT",
                host: "examplebucket.s3.amazonaws.com",
                path: "/test$file.text",
                query: &[],
                headers: &[
                    ("Date", "Fri, 24 May 2013 00:00:00 GMT"),
                    ("x-amz-storage-class", "REDUCED_REDUNDANCY"),
                ],
                payload_sha256: &sha256_hex(b"Welcome to Amazon S3."),
            },
            &example(),
            "us-east-1",
            "s3",
            at(),
        );
        assert_eq!(
            signature(&put),
            "98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd"
        );
        let lifecycle = sign(
            &Request {
                method: "GET",
                host: "examplebucket.s3.amazonaws.com",
                path: "/",
                query: &[("lifecycle", "")],
                headers: &[],
                payload_sha256: EMPTY_SHA256,
            },
            &example(),
            "us-east-1",
            "s3",
            at(),
        );
        assert_eq!(
            signature(&lifecycle),
            "fea454ca298b7da1c68078a5d1bdbfbbe0d65c699e0f91ac7a200a0136783543"
        );
        let list = sign(
            &Request {
                method: "GET",
                host: "examplebucket.s3.amazonaws.com",
                path: "/",
                query: &[("prefix", "J"), ("max-keys", "2")],
                headers: &[],
                payload_sha256: EMPTY_SHA256,
            },
            &example(),
            "us-east-1",
            "s3",
            at(),
        );
        assert_eq!(
            signature(&list),
            "34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7"
        );
        let names: Vec<&str> = get.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            ["x-amz-content-sha256", "x-amz-date", "authorization"]
        );
    }

    /// Temporary credentials send and sign their session token, and the
    /// debug form leaves the secret out.
    #[test]
    fn a_session_token_goes_along_and_secrets_stay_out_of_logs() {
        let creds = Credentials {
            session_token: Some("token".into()),
            ..example()
        };
        let headers = sign(
            &Request {
                method: "GET",
                host: "h",
                path: "/k",
                query: &[],
                headers: &[],
                payload_sha256: EMPTY_SHA256,
            },
            &creds,
            "eu-north-1",
            "s3",
            at(),
        );
        assert!(
            headers
                .iter()
                .any(|(n, v)| n == "x-amz-security-token" && v == "token")
        );
        assert!(
            headers
                .iter()
                .any(|(n, v)| n == "authorization" && v.contains("x-amz-security-token"))
        );
        assert!(!format!("{creds:?}").contains("EXAMPLEKEY"));
        assert_eq!(encode("a b/c$~", true), "a%20b/c%24~");
        assert_eq!(encode("a/b", false), "a%2Fb");
    }
}
