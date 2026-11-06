//! An image in an OCI image layout archive, as BuildKit writes one
//! (`--output type=oci`): its index, the manifest for this machine, its
//! config and its layers, each checked against its digest. The layers stay
//! the compressed archives they are: a VM unpacks them, as it does those a
//! pull downloads.

use super::registry::{self, Descriptor, Listed, Resolved};
use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

/// The largest index, manifest or config read whole.
const MAX_JSON: u64 = 4 * 1024 * 1024;

/// An image read from an archive: what a pull resolves, and its layers'
/// files, in order.
pub struct Image {
    pub resolved: Resolved,
    pub layers: Vec<PathBuf>,
}

/// Reads the OCI archive `archive`, keeping its blobs in `dir`, for the
/// image of this machine's architecture `arch`.
pub fn read(archive: &Path, dir: &Path, arch: &str) -> Result<Image> {
    std::fs::create_dir_all(dir)?;
    let blobs = extract(archive, dir)?;
    let blob = |digest: &str| -> Result<PathBuf> {
        let path = blobs
            .get(digest)
            .with_context(|| format!("the image's archive has no {digest}"))?;
        if registry::file_digest(path)? != digest {
            bail!("{digest} in the image's archive doesn't match its digest");
        }
        Ok(path.clone())
    };
    let small = |digest: &str| -> Result<Vec<u8>> {
        let path = blob(digest)?;
        if std::fs::metadata(&path)?.len() > MAX_JSON {
            bail!("{digest} in the image's archive is too large");
        }
        Ok(std::fs::read(path)?)
    };
    let index =
        std::fs::read(dir.join("index.json")).context("the image's archive has no index.json")?;
    let mut listed = registry::read_manifest(&index)?;
    // An index of manifests, as BuildKit writes it, or of indexes, one for
    // each platform.
    for _ in 0..3 {
        let manifests = match listed {
            Listed::Image { config, layers } => return image(config, layers, &small, &blob),
            Listed::Index(manifests) => manifests,
        };
        let wanted = registry::architecture(arch);
        let chosen = match manifests.as_slice() {
            [(digest, _)] => digest.clone(),
            many => many
                .iter()
                .find(|(_, p)| {
                    p.as_ref()
                        .is_some_and(|(os, a)| os == "linux" && a == wanted)
                })
                .map(|(d, _)| d.clone())
                .with_context(|| format!("the image's archive has no image for linux/{wanted}"))?,
        };
        listed = registry::read_manifest(&small(&chosen)?)?;
    }
    bail!("the image's archive nests its indexes too deep")
}

fn image(
    config: Descriptor,
    layers: Vec<Descriptor>,
    small: &dyn Fn(&str) -> Result<Vec<u8>>,
    blob: &dyn Fn(&str) -> Result<PathBuf>,
) -> Result<Image> {
    let (read, diff_ids) = registry::read_config(&small(&config.digest)?)?;
    let files = layers
        .iter()
        .map(|l| blob(&l.digest))
        .collect::<Result<Vec<_>>>()?;
    Ok(Image {
        resolved: Resolved {
            id: config.digest,
            config: read,
            layers,
            diff_ids,
        },
        layers: files,
    })
}

/// Extracts `index.json` and the blobs (`blobs/sha256/<hex>`) of the
/// archive to `dir`, and nothing else: the digests and their files.
fn extract(archive: &Path, dir: &Path) -> Result<HashMap<String, PathBuf>> {
    let file =
        std::fs::File::open(archive).with_context(|| format!("opening {}", archive.display()))?;
    let mut tar = tar::Archive::new(file);
    let mut blobs = HashMap::new();
    for entry in tar.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path()?.into_owned();
        let path = path.strip_prefix("./").unwrap_or(&path).to_owned();
        let name = path.to_string_lossy();
        if name == "index.json" {
            let mut index = Vec::new();
            (&mut entry).take(MAX_JSON).read_to_end(&mut index)?;
            std::fs::write(dir.join("index.json"), index)?;
            continue;
        }
        let Some(hex) = name.strip_prefix("blobs/sha256/") else {
            continue;
        };
        // The digest names the file: only its hex digits.
        if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let to = dir.join(hex);
        let mut out = std::fs::File::create(&to)?;
        std::io::copy(&mut (&mut entry).take(registry::MAX_LAYER), &mut out)?;
        blobs.insert(format!("sha256:{hex}"), to);
    }
    Ok(blobs)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An OCI archive as BuildKit writes one, of `files` (path, bytes).
    fn archive(path: &Path, files: &[(String, Vec<u8>)]) -> Result<()> {
        let mut tar = tar::Builder::new(std::fs::File::create(path)?);
        for (name, bytes) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, name, bytes.as_slice())?;
        }
        tar.finish()?;
        Ok(())
    }

    fn blob(bytes: &[u8]) -> (String, String) {
        let digest = registry::digest_of(bytes);
        let name = format!("blobs/sha256/{}", &digest["sha256:".len()..]);
        (digest, name)
    }

    #[test]
    fn reads_the_image_buildkit_wrote() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let layer = b"a layer's archive".to_vec();
        let (layer_digest, layer_name) = blob(&layer);
        let config = br#"{"architecture":"arm64","config":{"User":"nobody","Env":["BUILT=yes"],"WorkingDir":"/tmp"}}"#.to_vec();
        let (config_digest, config_name) = blob(&config);
        let manifest = format!(
            r#"{{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","config":{{"mediaType":"application/vnd.oci.image.config.v1+json","digest":"{config_digest}","size":{}}},"layers":[{{"mediaType":"application/vnd.oci.image.layer.v1.tar+gzip","digest":"{layer_digest}","size":{}}}]}}"#,
            config.len(),
            layer.len()
        )
        .into_bytes();
        let (manifest_digest, manifest_name) = blob(&manifest);
        let index = format!(
            r#"{{"schemaVersion":2,"mediaType":"application/vnd.oci.image.index.v1+json","manifests":[{{"mediaType":"application/vnd.oci.image.manifest.v1+json","digest":"{manifest_digest}","size":{}}}]}}"#,
            manifest.len()
        )
        .into_bytes();
        let tar = dir.path().join("image.tar");
        let entries = |config_bytes: Vec<u8>| {
            vec![
                (
                    "oci-layout".to_owned(),
                    br#"{"imageLayoutVersion":"1.0.0"}"#.to_vec(),
                ),
                ("index.json".to_owned(), index.clone()),
                (manifest_name.clone(), manifest.clone()),
                (config_name.clone(), config_bytes),
                (layer_name.clone(), layer.clone()),
                // Not a blob's name: left alone.
                (
                    "blobs/sha256/../../oops".replace("../../", "x"),
                    b"not a blob".to_vec(),
                ),
            ]
        };
        archive(&tar, &entries(config.clone()))?;
        let image = read(&tar, &dir.path().join("blobs"), "aarch64")?;
        assert_eq!(image.resolved.id, config_digest);
        assert_eq!(image.resolved.config.user.as_deref(), Some("nobody"));
        assert_eq!(image.resolved.layers.len(), 1);
        assert_eq!(std::fs::read(&image.layers[0])?, layer);
        assert!(!dir.path().join("blobs/oops").exists());
        // A blob that doesn't match its digest is refused.
        let tampered = dir.path().join("tampered.tar");
        archive(&tampered, &entries(br#"{"config":{}}"#.to_vec()))?;
        let e = read(&tampered, &dir.path().join("t"), "aarch64")
            .err()
            .map(|e| e.to_string());
        assert!(
            e.as_deref()
                .is_some_and(|e| e.contains("doesn't match its digest")),
            "{e:?}"
        );
        Ok(())
    }
}
