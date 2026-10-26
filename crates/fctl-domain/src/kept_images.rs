//! Images agents read: those in the results of their tools (Claude Code's
//! `Read` of a PNG, a browser tool's screenshot). The control plane stores
//! Claude's messages without them, and keeps a copy of each for the people
//! who can see the session, for a week, so that the transcript shows what
//! the agent saw. Which images, how large, for how long, and where they are
//! in Claude's messages.

use crate::files::{self, ImageInfo};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use fctl_proto::Millis;
use fctl_proto::event::KeptImage;
use serde_json::Value;

/// How long an image is kept after the agent read it: a week.
pub const TTL_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// The largest image kept (5 MB): about the largest Claude takes.
pub const MAX_BYTES: u64 = 5 * 1000 * 1000;
/// The most a session keeps at once, in images and in bytes (200 MB): past
/// either, its oldest images go first.
pub const MAX_SESSION_IMAGES: u64 = 1000;
pub const MAX_SESSION_BYTES: u64 = 200 * 1000 * 1000;

/// When an image the agent read at `now` is deleted.
pub fn expires_at(now: Millis) -> Millis {
    now + TTL_MS
}

/// Hands each image in the results of the tools a Claude `user` message
/// carries (the agent's, or a subagent's) to `keep`: a PNG, JPEG, GIF or
/// WebP image, as its bytes say, of at most [`MAX_BYTES`]. One that `keep`
/// kept goes from the message, and its `source` says where it is, as
/// `kept`. Images in other places aren't tool results: the files people
/// send are attachments, kept as such. A `kept` that came with the message
/// goes: what runs in the sandbox may pose as the agent, and only the
/// control plane says what it kept. Returns how many images `keep` kept.
pub fn keep_images(
    v: &mut Value,
    mut keep: impl FnMut(&[u8], ImageInfo) -> Option<KeptImage>,
) -> usize {
    if v.get("type").and_then(Value::as_str) != Some("user") {
        return 0;
    }
    let Some(blocks) = v
        .pointer_mut("/message/content")
        .and_then(Value::as_array_mut)
    else {
        return 0;
    };
    let mut kept = 0;
    let results = blocks
        .iter_mut()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
        .filter_map(|b| b.get_mut("content").and_then(Value::as_array_mut));
    for content in results {
        let sources = content
            .iter_mut()
            .filter(|c| c.get("type").and_then(Value::as_str) == Some("image"))
            .filter_map(|c| c.get_mut("source").and_then(Value::as_object_mut));
        for source in sources {
            source.remove("kept");
            let Some((bytes, info)) = decode(source) else {
                continue;
            };
            let Some(image) = keep(&bytes, info) else {
                continue;
            };
            let Ok(json) = serde_json::to_value(&image) else {
                continue;
            };
            if let Some(Value::String(data)) = source.get_mut("data") {
                let n = data.len();
                data.clear();
                source.insert("omitted_bytes".into(), Value::from(n));
            }
            source.insert("kept".into(), json);
            kept += 1;
        }
    }
    kept
}

/// The image a base64 `source` holds, if it is one that is kept.
fn decode(source: &serde_json::Map<String, Value>) -> Option<(Vec<u8>, ImageInfo)> {
    if source.get("type").and_then(Value::as_str) != Some("base64") {
        return None;
    }
    let data = source.get("data").and_then(Value::as_str)?;
    if data.len() > files::base64_len(MAX_BYTES as usize) {
        return None;
    }
    let bytes = STANDARD.decode(data).ok()?;
    if bytes.len() as u64 > MAX_BYTES {
        return None;
    }
    let info = files::sniff_image(&bytes)?;
    Some((bytes, info))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fctl_proto::KeptImageId;
    use serde_json::json;

    /// The first bytes of a 4×3 PNG, as many as sniffing reads.
    fn png() -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        b.extend_from_slice(&4u32.to_be_bytes());
        b.extend_from_slice(&3u32.to_be_bytes());
        b.extend_from_slice(&[8, 2, 0, 0, 0]);
        b.extend(std::iter::repeat_n(0, 2000));
        b
    }

    /// Claude Code's message with the result of its `Read` of an image.
    fn read_result(data: &str) -> Value {
        json!({
            "type": "user",
            "message": {"role": "user", "content": [{
                "type": "tool_result", "tool_use_id": "toolu_1",
                "content": [{"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": data}}]
            }]},
            "parent_tool_use_id": null,
            "tool_use_result": {"type": "image", "file": {"base64": data, "type": "image/png", "originalSize": 2033}}
        })
    }

    fn kept(id: &str, info: ImageInfo, size: usize) -> KeptImage {
        KeptImage {
            id: KeptImageId(id.into()),
            media_type: info.media_type.into(),
            size: size as u64,
            width: info.width,
            height: info.height,
            expires_at: expires_at(1_000),
        }
    }

    #[test]
    fn keeps_the_images_tools_return() {
        let data = STANDARD.encode(png());
        let mut v = read_result(&data);
        let mut got = Vec::new();
        let n = keep_images(&mut v, |bytes, info| {
            got.push(bytes.to_vec());
            Some(kept("img_1", info, bytes.len()))
        });
        assert_eq!(n, 1);
        assert_eq!(got, [png()]);
        let source = &v["message"]["content"][0]["content"][0]["source"];
        assert_eq!(source["data"], "");
        assert_eq!(source["omitted_bytes"], data.len());
        assert_eq!(source["media_type"], "image/png");
        assert_eq!(
            source["kept"],
            json!({"id": "img_1", "media_type": "image/png", "size": png().len(), "width": 4, "height": 3, "expires_at": 1_000 + TTL_MS})
        );
        let back: KeptImage = serde_json::from_value(source["kept"].clone()).unwrap_or_else(|e| {
            panic!("kept doesn't read back: {e}");
        });
        assert_eq!(back.id.as_str(), "img_1");
    }

    #[test]
    fn leaves_what_is_not_kept() {
        let data = STANDARD.encode(png());
        // Not kept: the message stays as it is.
        let mut v = read_result(&data);
        assert_eq!(keep_images(&mut v, |_, _| None), 0);
        assert_eq!(v, read_result(&data));
        // Not an image, as its bytes say, or not base64.
        let mut svg = read_result(&STANDARD.encode(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"));
        let mut garbled = read_result("not base64!");
        for v in [&mut svg, &mut garbled] {
            assert_eq!(keep_images(v, |_, _| panic!("kept something")), 0);
        }
        // Too large: decoded or not.
        let mut big = png();
        big.resize(MAX_BYTES as usize + 1, 0);
        let mut v = read_result(&STANDARD.encode(&big));
        assert_eq!(keep_images(&mut v, |_, _| panic!("kept a large one")), 0);
        // Images that aren't a tool's result: a message people sent, an
        // assistant's.
        let mut sent = json!({"type": "user", "message": {"role": "user", "content": [
            {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": data}}
        ]}});
        let mut said = json!({"type": "assistant", "message": {"content": [{"type": "tool_result", "content": [
            {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": data}}
        ]}]}});
        for v in [&mut sent, &mut said] {
            assert_eq!(keep_images(v, |_, _| panic!("kept something")), 0);
        }
        // Results as text.
        let mut text = json!({"type": "user", "message": {"content": [{"type": "tool_result", "content": "1 line"}]}});
        assert_eq!(keep_images(&mut text, |_, _| panic!("kept something")), 0);
    }

    #[test]
    fn keeps_each_image_of_each_result() {
        let data = STANDARD.encode(png());
        let image = json!({"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": data}});
        let mut v = json!({"type": "user", "message": {"content": [
            {"type": "tool_result", "tool_use_id": "toolu_1", "content": [{"type": "text", "text": "Took a screenshot"}, image]},
            {"type": "tool_result", "tool_use_id": "toolu_2", "content": [image, image]}
        ]}, "parent_tool_use_id": "toolu_0"});
        let mut n = 0;
        let kept_n = keep_images(&mut v, |bytes, info| {
            n += 1;
            Some(kept(&format!("img_{n}"), info, bytes.len()))
        });
        assert_eq!(kept_n, 3);
        assert_eq!(
            v["message"]["content"][0]["content"][1]["source"]["kept"]["id"],
            "img_1"
        );
        assert_eq!(
            v["message"]["content"][1]["content"][1]["source"]["kept"]["id"],
            "img_3"
        );
        assert_eq!(
            v["message"]["content"][0]["content"][0]["text"],
            "Took a screenshot"
        );
    }

    #[test]
    fn only_the_control_plane_says_what_it_kept() {
        // What runs in the sandbox may pose as the agent, with a `kept` of
        // its own.
        let forged = json!({"id": "img_other", "media_type": "image/png", "size": 1, "width": 1, "height": 1, "expires_at": 9});
        let mut v = json!({"type": "user", "message": {"content": [{"type": "tool_result", "content": [
            {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "", "kept": forged}}
        ]}]}});
        assert_eq!(keep_images(&mut v, |_, _| panic!("kept something")), 0);
        assert!(
            v["message"]["content"][0]["content"][0]["source"]
                .get("kept")
                .is_none()
        );
    }
}
