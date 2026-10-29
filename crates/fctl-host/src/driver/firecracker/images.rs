//! The images a Firecracker host keeps: for each, the root filesystem its
//! layers make (read only, shared by the sandboxes made from it) and what
//! its config says about running it; and the names that refer to them, as
//! Docker's tags do.

use anyhow::{Context, Result};
use fctl_domain::image::ImageRef;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// An image here.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Image {
    /// Its config's digest, as Docker's image ID.
    pub id: String,
    /// The name it was pulled by.
    pub reference: String,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub workdir: Option<String>,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    /// What its layers unpacked to.
    #[serde(default)]
    pub files: u64,
    #[serde(default)]
    pub bytes: u64,
    pub pulled_ms: i64,
}

/// A reference as the store keeps it: `ubuntu` is
/// `docker.io/library/ubuntu:latest`.
pub fn canonical(reference: &str) -> Option<String> {
    let r = ImageRef::parse(reference)?;
    Some(match (&r.digest, &r.tag) {
        (Some(d), Some(t)) => format!("{}:{t}@{d}", r.name),
        (Some(d), None) => format!("{}@{d}", r.name),
        (None, Some(t)) => format!("{}:{t}", r.name),
        (None, None) => format!("{}:latest", r.name),
    })
}

/// The images under `dir`: `refs.json`, and a directory for each image
/// named after its ID's hex digits, with `image.json` and `rootfs.ext4`.
#[derive(Clone, Debug)]
pub struct Store {
    pub dir: PathBuf,
}

impl Store {
    fn refs_path(&self) -> PathBuf {
        self.dir.join("refs.json")
    }

    pub fn refs(&self) -> BTreeMap<String, String> {
        std::fs::read(self.refs_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save_refs(&self, refs: &BTreeMap<String, String>) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let tmp = self.refs_path().with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(refs)?)?;
        std::fs::rename(&tmp, self.refs_path())?;
        Ok(())
    }

    /// The directory of the image `id`.
    pub fn image_dir(&self, id: &str) -> PathBuf {
        let hex = id.split_once(':').map_or(id, |(_, h)| h);
        let safe: String = hex.chars().filter(char::is_ascii_alphanumeric).collect();
        self.dir.join(safe)
    }

    pub fn rootfs(&self, id: &str) -> PathBuf {
        self.image_dir(id).join("rootfs.ext4")
    }

    /// The image `id`, when its root filesystem is here.
    pub fn image(&self, id: &str) -> Option<Image> {
        let dir = self.image_dir(id);
        if !dir.join("rootfs.ext4").is_file() {
            return None;
        }
        let bytes = std::fs::read(dir.join("image.json")).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// The image `reference` names here.
    pub fn lookup(&self, reference: &str) -> Option<Image> {
        let key = canonical(reference)?;
        let id = self.refs().get(&key)?.clone();
        self.image(&id)
    }

    /// Keeps `image`, whose root filesystem is at `rootfs`, and names it
    /// `reference`.
    pub fn add(&self, image: &Image, rootfs: &Path) -> Result<()> {
        let dir = self.image_dir(&image.id);
        let tmp = dir.with_extension("new");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp)?;
        std::fs::rename(rootfs, tmp.join("rootfs.ext4"))?;
        std::fs::write(tmp.join("image.json"), serde_json::to_vec_pretty(image)?)?;
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::rename(&tmp, &dir)?;
        self.tag(&image.reference, &image.id)
    }

    /// Names the image `id` `reference`, in place of what that named.
    pub fn tag(&self, reference: &str, id: &str) -> Result<()> {
        let key = canonical(reference)
            .with_context(|| format!("{reference} isn't an image reference"))?;
        let mut refs = self.refs();
        refs.insert(key, id.to_owned());
        self.save_refs(&refs)
    }

    /// Takes the name `reference` away; returns the image it named.
    pub fn untag(&self, reference: &str) -> Result<Option<String>> {
        let Some(key) = canonical(reference) else {
            return Ok(None);
        };
        let mut refs = self.refs();
        let id = refs.remove(&key);
        if id.is_some() {
            self.save_refs(&refs)?;
        }
        Ok(id)
    }

    /// The names of `repository`'s images (`repository:tag`).
    pub fn names(&self, repository: &str) -> Vec<String> {
        let Some(name) = ImageRef::parse(repository).map(|r| r.name) else {
            return Vec::new();
        };
        self.refs()
            .into_keys()
            .filter(|k| {
                k.strip_prefix(&name)
                    .and_then(|rest| rest.strip_prefix(':'))
                    .is_some_and(|tag| !tag.is_empty() && !tag.contains('@'))
            })
            .collect()
    }

    /// Whether a name refers to the image `id`.
    pub fn named(&self, id: &str) -> bool {
        self.refs().values().any(|v| v == id)
    }

    /// Removes the image `id`'s files.
    pub fn remove(&self, id: &str) -> Result<()> {
        let dir = self.image_dir(id);
        match std::fs::remove_dir_all(&dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(e).with_context(|| format!("removing {}", dir.display()))
            }
            _ => Ok(()),
        }
    }

    /// The IDs of the images here.
    pub fn ids(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter(|e| e.path().join("image.json").is_file())
            .filter_map(|e| {
                let bytes = std::fs::read(e.path().join("image.json")).ok()?;
                serde_json::from_slice::<Image>(&bytes).ok().map(|i| i.id)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_are_kept_as_docker_reads_them() {
        assert_eq!(
            canonical("ubuntu").as_deref(),
            Some("docker.io/library/ubuntu:latest")
        );
        assert_eq!(
            canonical("ghcr.io/hyperstructure/sandbox:0.26.0").as_deref(),
            Some("ghcr.io/hyperstructure/sandbox:0.26.0")
        );
        let pinned = format!("ghcr.io/acme/agent@sha256:{}", "a".repeat(64));
        assert_eq!(canonical(&pinned), Some(pinned.clone()));
        assert_eq!(canonical("-not an image"), None);
    }

    #[test]
    fn names_refer_to_images_as_tags_do() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = Store {
            dir: dir.path().join("images"),
        };
        let image = Image {
            id: format!("sha256:{}", "1".repeat(64)),
            reference: "ghcr.io/hyperstructure/sandbox:0.26.0".into(),
            user: Some("agent".into()),
            pulled_ms: 1,
            ..Image::default()
        };
        let rootfs = dir.path().join("rootfs.tmp");
        std::fs::write(&rootfs, b"ext4")?;
        store.add(&image, &rootfs)?;
        assert_eq!(
            store.lookup("ghcr.io/hyperstructure/sandbox:0.26.0"),
            Some(image.clone())
        );
        assert_eq!(store.lookup("ghcr.io/hyperstructure/sandbox:latest"), None);
        store.tag("ghcr.io/hyperstructure/sandbox:latest", &image.id)?;
        store.tag("ghcr.io/hyperstructure/sandbox", &image.id)?;
        assert_eq!(
            store.names("ghcr.io/hyperstructure/sandbox"),
            [
                "ghcr.io/hyperstructure/sandbox:0.26.0",
                "ghcr.io/hyperstructure/sandbox:latest"
            ]
        );
        assert!(
            store
                .names("ghcr.io/hyperstructure/sandbox-kernel")
                .is_empty()
        );
        assert_eq!(
            store.untag("ghcr.io/hyperstructure/sandbox:0.26.0")?,
            Some(image.id.clone())
        );
        assert!(store.named(&image.id));
        assert_eq!(
            store.untag("ghcr.io/hyperstructure/sandbox:latest")?,
            Some(image.id.clone())
        );
        assert!(!store.named(&image.id));
        assert_eq!(store.ids(), std::slice::from_ref(&image.id));
        store.remove(&image.id)?;
        assert_eq!(store.image(&image.id), None);
        assert!(store.ids().is_empty());
        Ok(())
    }
}
