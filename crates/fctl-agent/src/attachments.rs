//! Attachments in the sandbox. The host sends each file's bytes before the
//! message that refers to it; they are saved as `~/attachments/<id>/<name>`,
//! outside the repository, and each message lists where its files are.
//! Images Claude accepts as they are also go into the message itself.

use crate::prompt::{self, FileNote};
use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use fctl_domain::files;
use fctl_proto::AttachmentId;
use fctl_proto::event::Attachment;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Where an attachment is saved under `dir` (`None` for a malformed id).
pub fn path(dir: &Path, att: &Attachment) -> Option<PathBuf> {
    AttachmentId::is_valid(att.id.as_str()).then(|| {
        dir.join(att.id.as_str())
            .join(files::sanitize_name(&att.name))
    })
}

/// Saves an attachment from its base64 bytes.
pub fn save(dir: &Path, att: &Attachment, data: &str) -> Result<PathBuf> {
    let bytes = STANDARD.decode(data).context("decoding the file")?;
    let path = path(dir, att).context("malformed attachment id")?;
    let parent = path.parent().context("no directory")?;
    std::fs::create_dir_all(parent)?;
    let tmp = parent.join(".fctl-partial");
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

/// What a message says about its files.
pub struct Prepared {
    /// The list of files, for after the message's text (empty without files).
    pub note: String,
    /// Image blocks for the images that go into the message.
    pub images: Vec<Value>,
}

/// Looks up a message's files under `dir` and picks the images Claude sees
/// directly, in order, within [`files::INLINE_MESSAGE_MAX_BASE64`].
pub fn prepare(dir: Option<&Path>, attachments: &[Attachment]) -> Prepared {
    let mut notes = Vec::with_capacity(attachments.len());
    let mut images = Vec::new();
    let mut budget = files::INLINE_MESSAGE_MAX_BASE64;
    for att in attachments {
        let saved = dir.and_then(|d| path(d, att)).filter(|p| p.is_file());
        let image = saved.as_deref().and_then(|p| inline(p, budget));
        let inline = image.is_some();
        if let Some((block, len)) = image {
            budget -= len;
            images.push(block);
        }
        notes.push(FileNote {
            attachment: att,
            path: saved,
            inline,
        });
    }
    Prepared {
        note: prompt::files_note(&notes),
        images,
    }
}

/// An image block for the file, if Claude accepts it as it is and its
/// base64 fits in `budget`; with that length.
fn inline(path: &Path, budget: usize) -> Option<(Value, usize)> {
    let len = std::fs::metadata(path).ok()?.len() as usize;
    if files::base64_len(len) > budget.min(files::INLINE_IMAGE_MAX_BASE64) {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let info = files::inline_image(&bytes)?;
    let block = json!({
        "type": "image",
        "source": { "type": "base64", "media_type": info.media_type, "data": STANDARD.encode(&bytes) }
    });
    Some((block, files::base64_len(bytes.len())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, pad: usize) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        b.extend_from_slice(&w.to_be_bytes());
        b.extend_from_slice(&h.to_be_bytes());
        b.resize(b.len() + pad, 0);
        b
    }

    fn att(id: &str, name: &str, size: usize) -> Attachment {
        Attachment {
            id: AttachmentId(id.into()),
            name: name.into(),
            media_type: "image/png".into(),
            size: size as u64,
            width: None,
            height: None,
        }
    }

    #[test]
    fn saves_and_describes_files() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let small = png(800, 600, 1000);
        let wide = png(3024, 1964, 1000);
        let a = att("att_1", "shot.png", small.len());
        let b = att("att_2", "../wide.png", wide.len());
        let c = att("att_3", "notes.txt", 10);
        let saved = save(dir.path(), &a, &STANDARD.encode(&small))?;
        assert_eq!(saved, dir.path().join("att_1/shot.png"));
        assert_eq!(std::fs::read(&saved)?, small);
        save(dir.path(), &b, &STANDARD.encode(&wide))?;
        assert!(dir.path().join("att_2/wide.png").is_file());

        let p = prepare(Some(dir.path()), &[a, b, c]);
        assert_eq!(p.images.len(), 1, "only the small image goes inline");
        assert_eq!(p.images[0]["source"]["media_type"], "image/png");
        assert_eq!(p.images[0]["source"]["data"], STANDARD.encode(&small));
        assert!(
            p.note
                .contains("att_1/shot.png (image/png, 1 KB; shown below)"),
            "{}",
            p.note
        );
        assert!(
            p.note.contains("att_2/wide.png (image/png, 1 KB)\n"),
            "{}",
            p.note
        );
        assert!(
            p.note
                .ends_with("- notes.txt (image/png, 10 B): it did not reach the sandbox"),
            "{}",
            p.note
        );
        Ok(())
    }

    #[test]
    fn rejects_bad_ids_and_respects_the_budget() -> Result<()> {
        let dir = tempfile::tempdir()?;
        assert!(path(dir.path(), &att("att_../x", "a", 1)).is_none());
        assert!(save(dir.path(), &att("../../x", "a", 1), "AAAA").is_err());
        // Two images that each fit, but not together.
        let big = png(100, 100, files::INLINE_MESSAGE_MAX_BASE64 / 4 * 3 - 200);
        let a = att("att_a", "a.png", big.len());
        let b = att("att_b", "b.png", big.len());
        save(dir.path(), &a, &STANDARD.encode(&big))?;
        save(dir.path(), &b, &STANDARD.encode(&big))?;
        let p = prepare(Some(dir.path()), &[a, b]);
        assert_eq!(p.images.len(), 1);
        assert_eq!(prepare(None, &[]).note, "");
        Ok(())
    }
}
