//! Keeping the default sandbox image current on a host, and removing the
//! builds and versions of it that sessions no longer start from.
//!
//! hyperstructure/sandbox's Image workflow rebuilds the newest release's
//! image under the same tag when Claude Code has a newer release, since fctl
//! turns off Claude Code's auto-updater. A host on the default image pulls
//! its tag again when it starts and every [`REFRESH_EVERY`], so that new
//! sessions start from the newer build, but never over a build of the
//! host's own: only the workflow sets [`REVISION_LABEL`].
//!
//! A published build that a newer one replaces keeps a name of its own,
//! `…:0.5.0-replaced-<its ID>`. [`clean_up`] removes those, the images of
//! other versions, and `latest` once the version's image is here. It
//! removes them without force, so Docker keeps the ones a sandbox uses
//! until the sweep removes the sandbox.

use crate::config::{HostConfig, IMAGE_REPOSITORY};
use crate::driver::{Driver, ImageInfo};
use anyhow::Result;
use std::future::Future;
use std::time::Duration;

/// How often a host on the default image pulls it again. Its tag changes at
/// most about once a day, and a pull of an unchanged tag fetches only its
/// manifest.
pub const REFRESH_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// The commit of hyperstructure/sandbox an image was built from. Only its
/// Image workflow sets it: an image without it was built on the host.
pub const REVISION_LABEL: &str = "org.opencontainers.image.revision";

/// The version of Claude Code in a published image.
pub const CLAUDE_CODE_LABEL: &str = "sh.fctl.claude-code.version";

/// What the name of a build that a newer one replaced has after its tag.
const REPLACED: &str = "-replaced-";

/// Whether the Image workflow built `image`, rather than the host (`fctl
/// host image build`) or someone on it. An image whose description can't
/// be read counts as built here, so the host leaves it alone.
pub fn published(image: &ImageInfo) -> bool {
    !image.id.is_empty()
        && image
            .labels
            .get(REVISION_LABEL)
            .is_some_and(|r| !r.is_empty())
}

/// ` (Claude Code 2.1.302)` for a published image, or nothing.
pub fn claude_code(image: &ImageInfo) -> String {
    image
        .labels
        .get(CLAUDE_CODE_LABEL)
        .map(|version| format!(" (Claude Code {version})"))
        .unwrap_or_default()
}

/// The host's own pull, when it starts and every [`REFRESH_EVERY`]. It pulls
/// an image that isn't here yet, so that the first session after an upgrade
/// doesn't wait for it, or the fallback while the image can't be pulled:
/// sessions start from it then. On the default image it pulls the tag
/// again, so that new sessions start from its newest build, unless the
/// build here is the host's own. What it does goes to the log.
pub async fn refresh(driver: &Driver, cfg: &HostConfig) {
    let image = cfg.image();
    match driver.image(&image).await {
        Err(e) => tracing::warn!("looking for {image}: {e:#}"),
        Ok(None) => {
            // Never over a fallback that is here: sessions start from it.
            let fallback = match cfg.fallback_image() {
                Some(f) if !driver.has_image(&f).await => Some(f),
                _ => None,
            };
            tracing::info!("pulling {image}");
            let log = |line: String| tracing::info!("{line}");
            match driver
                .pull_or_fallback(&image, fallback.as_deref(), false, &log)
                .await
            {
                Ok(pulled) => tracing::info!("pulled {pulled}"),
                Err(e) => tracing::warn!("pulling {image}: {e:#} (sessions try again)"),
            }
        }
        Ok(Some(here)) if cfg.image.is_none() && published(&here) => {
            match replacing(driver, &image, driver.pull(&image, false)).await {
                Ok(Replaced {
                    newer: Some(newer), ..
                }) => tracing::info!("pulled a newer build of {image}{}", claude_code(&newer)),
                Ok(_) => tracing::debug!("{image} is up to date"),
                Err(e) => tracing::warn!(
                    "pulling {image} again: {e:#} (sessions start from the build here)"
                ),
            }
        }
        // Built here, or another image: the host leaves it as it is.
        Ok(Some(_)) => tracing::debug!("{image} is not the published image; not pulling it"),
    }
}

/// What `fctl host image pull` pulled.
#[derive(Debug)]
pub struct Pulled {
    /// The host's image, or the fallback while that can't be pulled.
    pub image: String,
    /// The build it brought, when it replaced one that was here.
    pub newer: Option<ImageInfo>,
    /// The name the published build it replaced keeps until [`clean_up`]
    /// removes it.
    pub replaced: Option<String>,
}

/// `fctl host image pull`: pulls the host's image again, over a build of
/// the host's own too, or the fallback while the image can't be pulled and
/// isn't here. `progress` shows the runtime's own output.
pub async fn pull(
    driver: &Driver,
    cfg: &HostConfig,
    progress: bool,
    log: &(dyn Fn(String) + Send + Sync),
) -> Result<Pulled> {
    let image = cfg.image();
    if cfg.image.is_none() && driver.has_image(&image).await {
        let replaced = replacing(driver, &image, driver.pull(&image, progress)).await?;
        return Ok(Pulled {
            image,
            newer: replaced.newer,
            replaced: replaced.name,
        });
    }
    // Another image, or one that isn't here yet.
    let fallback = cfg.fallback_image();
    let pulled = driver
        .pull_or_fallback(&image, fallback.as_deref(), progress, log)
        .await?;
    Ok(Pulled {
        image: pulled,
        newer: None,
        replaced: None,
    })
}

/// What [`replacing`] did.
#[derive(Debug)]
pub struct Replaced {
    /// The image's build now, when it is a newer one.
    pub newer: Option<ImageInfo>,
    /// The name the published build it replaced keeps until [`clean_up`]
    /// removes it.
    pub name: Option<String>,
}

/// Runs `replace`, a pull or a build that may give `image` a newer build.
/// A published build of the default image that it replaces keeps a name
/// ([`replaced_name`]), so that [`clean_up`] can remove it once no sandbox
/// uses it: Docker would keep it without a name, and Apple's `container`
/// as content no image refers to.
pub async fn replacing(
    driver: &Driver,
    image: &str,
    replace: impl Future<Output = Result<()>>,
) -> Result<Replaced> {
    let before = driver.image(image).await?;
    let name = before
        .as_ref()
        .filter(|b| published(b) && tag_of(image).is_some())
        .map(|b| replaced_name(image, &b.id));
    if let Some(name) = &name {
        driver.tag_image(image, name).await?;
    }
    let result = replace.await;
    let after = driver.image(image).await.ok().flatten();
    let newer = after.filter(|a| before.as_ref().is_none_or(|b| b.id != a.id));
    // Without a newer build, the name is a second one for the build here.
    if newer.is_none()
        && let Some(name) = &name
        && let Err(e) = driver.remove_image(name).await
    {
        tracing::debug!("removing {name}: {e:#}");
    }
    result?;
    let name = name.filter(|_| newer.is_some());
    Ok(Replaced { newer, name })
}

/// The name a build of `image` keeps once a newer one replaced it, after
/// the start of its ID: `…:0.5.0-replaced-4f1c0ab2d3e5`.
pub fn replaced_name(image: &str, id: &str) -> String {
    let id = id.split_once(':').map_or(id, |(_, hex)| hex);
    let short: String = id
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(12)
        .collect();
    format!("{image}{REPLACED}{short}")
}

/// Removes the default image's names that sessions no longer start from
/// ([`removable`]), without force: Docker keeps an image that a sandbox
/// uses, stopped ones too, until the sweep removes the sandbox. A host on
/// another image leaves them alone. Returns the names it removed.
pub async fn clean_up(driver: &Driver, cfg: &HostConfig) -> Vec<String> {
    if cfg.image.is_some() || !driver.uses_images() {
        return Vec::new();
    }
    let names = match driver.images(IMAGE_REPOSITORY).await {
        Ok(names) => names,
        Err(e) => {
            tracing::warn!("listing the images of {IMAGE_REPOSITORY}: {e:#}");
            return Vec::new();
        }
    };
    let mut here = Vec::new();
    for name in names {
        if let Ok(Some(info)) = driver.image(&name).await {
            here.push((name, info));
        }
    }
    let mut removed = Vec::new();
    for name in removable(&cfg.image(), cfg.fallback_image().as_deref(), &here) {
        match driver.remove_image(&name).await {
            Ok(()) => removed.push(name),
            // A sandbox still uses it.
            Err(e) => tracing::debug!("keeping {name}: {e:#}"),
        }
    }
    removed
}

/// Which of `here`, the default image's names on this host with what each
/// holds, [`clean_up`] removes on a host whose sessions start from `image`,
/// or from `fallback` while `image` isn't here: the images of other
/// versions, builds that a newer one replaced, and `fallback` once `image`
/// is here. Builds without the [`REVISION_LABEL`] stay, and so do tags
/// that hosts don't pull, like `0.5`, which projects may use.
pub fn removable(image: &str, fallback: Option<&str>, here: &[(String, ImageInfo)]) -> Vec<String> {
    let id_of = |name: &str| {
        here.iter()
            .find(|(n, _)| n == name)
            .map(|(_, info)| info.id.as_str())
    };
    let has_image = id_of(image).is_some();
    // A name for the build sessions start from isn't a replaced build's
    // yet: a pull is under way, or one stopped before it removed the name.
    let current = id_of(image).or_else(|| fallback.and_then(id_of));
    here.iter()
        .filter(|(name, info)| {
            let Some(tag) = tag_of(name) else {
                return false;
            };
            if name == image || !published(info) {
                false
            } else if Some(name.as_str()) == fallback {
                has_image
            } else if tag.contains(REPLACED) {
                current != Some(info.id.as_str())
            } else {
                is_version(tag)
            }
        })
        .map(|(name, _)| name.clone())
        .collect()
}

/// A name that [`clean_up`] removed, as people know it: a replaced build
/// by the name it had.
pub fn describe(name: &str) -> String {
    match name.split_once(REPLACED) {
        Some((image, _)) => format!("a build of {image} that a newer one replaced"),
        None => name.to_owned(),
    }
}

/// The tag of one of the default image's names: `0.5.0` for
/// `ghcr.io/hyperstructure/sandbox:0.5.0`.
fn tag_of(name: &str) -> Option<&str> {
    name.strip_prefix(IMAGE_REPOSITORY)?.strip_prefix(':')
}

/// Whether `tag` is a release's version, like `0.5.0` or `0.6.0-rc.1`:
/// the tags hosts pull.
fn is_version(tag: &str) -> bool {
    let release = tag.split_once('-').map_or(tag, |(release, _)| release);
    let numbers: Vec<&str> = release.split('.').collect();
    numbers.len() == 3
        && numbers
            .iter()
            .all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::container::Flavor;
    use crate::driver::fake::{FakeCli, id};
    use std::collections::BTreeMap;

    const LATEST: &str = "ghcr.io/hyperstructure/sandbox:latest";

    fn name(tag: &str) -> String {
        format!("{IMAGE_REPOSITORY}:{tag}")
    }

    fn info(id: &str, published: bool) -> ImageInfo {
        let labels = if published {
            BTreeMap::from([(REVISION_LABEL.to_owned(), "12685ad".to_owned())])
        } else {
            BTreeMap::new()
        };
        ImageInfo {
            id: id.into(),
            labels,
        }
    }

    #[test]
    fn removes_what_sessions_no_longer_start_from() {
        let image = name("0.5.0");
        let here = [
            (image.clone(), info("sha256:aa", true)),
            (name("latest"), info("sha256:bb", true)),
            (name("0.4.0"), info("sha256:cc", true)),
            (name("0.6.0-rc.1"), info("sha256:dd", true)),
            (name("0.3.0"), info("sha256:ee", false)),
            (name("0.5"), info("sha256:aa", true)),
            (name("dev"), info("sha256:ff", true)),
            (name("0.5.0-replaced-0011"), info("sha256:0011", true)),
            (name("0.5.0-replaced-aa"), info("sha256:aa", true)),
            ("ghcr.io/acme/agent:0.4.0".into(), info("sha256:99", true)),
        ];
        assert_eq!(
            removable(&image, Some(LATEST), &here),
            [
                name("latest"),
                name("0.4.0"),
                name("0.6.0-rc.1"),
                name("0.5.0-replaced-0011"),
            ]
        );
        // A build without the label stays, even as `latest`.
        let here = [
            (image.clone(), info("sha256:aa", true)),
            (name("latest"), info("sha256:bb", false)),
        ];
        assert!(removable(&image, Some(LATEST), &here).is_empty());
    }

    #[test]
    fn keeps_latest_while_the_versions_image_is_not_here() {
        let image = name("0.5.0");
        let mut here = vec![
            (name("latest"), info("sha256:bb", true)),
            (name("0.4.0"), info("sha256:cc", true)),
            // A pull of `latest` under way.
            (name("latest-replaced-bb"), info("sha256:bb", true)),
        ];
        assert_eq!(removable(&image, Some(LATEST), &here), [name("0.4.0")]);
        here.push((image.clone(), info("sha256:aa", true)));
        assert_eq!(
            removable(&image, Some(LATEST), &here),
            [name("latest"), name("0.4.0"), name("latest-replaced-bb")]
        );
    }

    #[test]
    fn knows_a_version_when_it_sees_one() {
        for tag in ["0.5.0", "10.20.30", "0.6.0-rc.1"] {
            assert!(is_version(tag), "{tag}");
        }
        for tag in [
            "0.5", "latest", "1.2.3.4", "v0.5.0", "0.5.x", "", "..", "0.5.0.",
        ] {
            assert!(!is_version(tag), "{tag}");
        }
    }

    #[test]
    fn names_a_replaced_build_after_its_id() {
        let image = name("0.5.0");
        let id = "sha256:4f1c0ab2d3e5f60718293a4b5c6d7e8f";
        assert_eq!(
            replaced_name(&image, id),
            format!("{image}-replaced-4f1c0ab2d3e5")
        );
        // Podman's IDs have no algorithm.
        assert_eq!(
            replaced_name(&image, "4f1c0ab2d3e5f607"),
            format!("{image}-replaced-4f1c0ab2d3e5")
        );
        assert_eq!(
            describe(&format!("{image}-replaced-4f1c0ab2d3e5")),
            format!("a build of {image} that a newer one replaced")
        );
        assert_eq!(describe(&name("0.4.0")), name("0.4.0"));
    }

    /// The host's image, a stand-in CLI without images, and its driver.
    fn setup(flavor: Flavor) -> Result<(String, FakeCli, Driver)> {
        let cli = FakeCli::empty()?;
        let driver = cli.host_driver(flavor);
        Ok((crate::config::default_image(), cli, driver))
    }

    #[tokio::test]
    async fn the_host_pulls_a_newer_build_and_removes_the_old_one_once_unused() -> Result<()> {
        let cfg = HostConfig::default();
        let (image, cli, driver) = setup(Flavor::Docker)?;
        let (old, new) = (id("old"), id("new"));
        cli.have(&image, &old, Some("2.1.302"))?;
        cli.publish(&image, &new, "2.1.310")?;
        cli.uses("fctl-ses-1", &old)?;
        refresh(&driver, &cfg).await;
        let kept = replaced_name(&image, &old);
        assert_eq!(
            cli.images()?,
            [(image.clone(), new.clone()), (kept.clone(), old.clone())]
        );
        // A stopped sandbox keeps the build it was made from.
        assert!(clean_up(&driver, &cfg).await.is_empty());
        assert_eq!(cli.images()?.len(), 2);
        // Once the sweep removed it, the build goes.
        cli.drop_container("fctl-ses-1")?;
        assert_eq!(clean_up(&driver, &cfg).await, [kept]);
        assert_eq!(cli.images()?, [(image.clone(), new.clone())]);
        Ok(())
    }

    #[tokio::test]
    async fn the_host_leaves_a_current_build_as_it_is() -> Result<()> {
        let cfg = HostConfig::default();
        let (image, cli, driver) = setup(Flavor::Docker)?;
        let build = id("build");
        cli.have(&image, &build, Some("2.1.302"))?;
        cli.publish(&image, &build, "2.1.302")?;
        refresh(&driver, &cfg).await;
        assert_eq!(cli.images()?, [(image.clone(), build.clone())]);
        let kept = replaced_name(&image, &build);
        assert_eq!(
            cli.calls(),
            [
                format!("image inspect {image}"),
                format!("image inspect {image}"),
                format!("image tag {image} {kept}"),
                format!("pull {image}"),
                format!("image inspect {image}"),
                format!("image rm {kept}"),
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn only_a_person_pulls_over_a_build_of_the_hosts_own() -> Result<()> {
        let cfg = HostConfig::default();
        let (image, cli, driver) = setup(Flavor::Docker)?;
        let (built, published) = (id("built"), id("published"));
        cli.have(&image, &built, None)?;
        cli.publish(&image, &published, "2.1.310")?;
        refresh(&driver, &cfg).await;
        assert_eq!(cli.images()?, [(image.clone(), built.clone())]);
        assert!(!cli.calls().iter().any(|c| c.contains("pull")));
        assert!(clean_up(&driver, &cfg).await.is_empty());

        let log = |_: String| {};
        let pulled = pull(&driver, &cfg, false, &log).await?;
        assert_eq!(pulled.image, image);
        let newer = pulled.newer.map(|n| n.id);
        assert_eq!(newer.as_deref(), Some(published.as_str()));
        // The build it replaced is the person's: it has no name to remove it by.
        assert_eq!(pulled.replaced, None);
        assert_eq!(cli.images()?, [(image, published)]);
        Ok(())
    }

    #[tokio::test]
    async fn a_build_over_the_published_image_leaves_it_to_the_clean_up() -> Result<()> {
        let cfg = HostConfig::default();
        let (image, cli, driver) = setup(Flavor::Docker)?;
        let published = id("published");
        cli.have(&image, &published, Some("2.1.302"))?;
        let failed = async { Err::<(), _>(anyhow::anyhow!("the build failed")) };
        assert!(replacing(&driver, &image, failed).await.is_err());
        assert_eq!(cli.images()?, [(image.clone(), published.clone())]);

        // What `fctl host image build` does to the image's name.
        let build = async { cli.have(&image, &id("built"), None) };
        let replaced = replacing(&driver, &image, build).await?;
        let kept = replaced_name(&image, &published);
        assert_eq!(replaced.name.as_deref(), Some(kept.as_str()));
        assert_eq!(clean_up(&driver, &cfg).await, [kept.as_str()]);
        assert_eq!(cli.images()?, [(image, id("built"))]);
        Ok(())
    }

    #[tokio::test]
    async fn latest_goes_once_the_versions_image_is_here() -> Result<()> {
        let cfg = HostConfig::default();
        let (image, cli, driver) = setup(Flavor::Docker)?;
        // A release's image is published within the hour after the release.
        cli.publish(LATEST, &id("previous release"), "2.1.302")?;
        refresh(&driver, &cfg).await;
        assert_eq!(cli.images()?, [(LATEST.to_owned(), id("previous release"))]);
        assert!(clean_up(&driver, &cfg).await.is_empty());

        cli.publish(&image, &id("release"), "2.1.310")?;
        refresh(&driver, &cfg).await;
        assert_eq!(clean_up(&driver, &cfg).await, [LATEST]);
        assert_eq!(cli.images()?, [(image, id("release"))]);
        Ok(())
    }

    #[tokio::test]
    async fn after_an_upgrade_the_versions_before_go() -> Result<()> {
        let cfg = HostConfig::default();
        let (image, cli, driver) = setup(Flavor::Docker)?;
        cli.have(&image, &id("current"), Some("2.1.310"))?;
        cli.publish(&image, &id("current"), "2.1.310")?;
        cli.have(&name("0.0.1"), &id("0.0.1"), Some("2.1.200"))?;
        cli.have(&name("0.0.2"), &id("0.0.2"), Some("2.1.250"))?;
        cli.have(&name("0.0.3"), &id("0.0.3"), None)?;
        cli.have(&name("0.0"), &id("0.0.2"), Some("2.1.250"))?;
        cli.uses("fctl-ses-1", &id("0.0.1"))?;
        refresh(&driver, &cfg).await;
        // `0.0` names the build of 0.0.2 too, so removing `0.0.2` untags it.
        assert_eq!(clean_up(&driver, &cfg).await, [name("0.0.2")]);
        assert_eq!(
            cli.images()?,
            [
                (name("0.0"), id("0.0.2")),
                (name("0.0.1"), id("0.0.1")),
                (name("0.0.3"), id("0.0.3")),
                (image, id("current")),
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn apple_removes_a_replaced_build_that_a_sandbox_was_made_from() -> Result<()> {
        // Each container has a copy of the image's filesystem.
        let cfg = HostConfig::default();
        let (image, cli, driver) = setup(Flavor::Apple)?;
        let (old, new) = (id("old"), id("new"));
        cli.have(&image, &old, Some("2.1.302"))?;
        cli.publish(&image, &new, "2.1.310")?;
        cli.uses("fctl-ses-1", &old)?;
        let log = |_: String| {};
        let pulled = pull(&driver, &cfg, false, &log).await?;
        let kept = replaced_name(&image, &old);
        assert_eq!(pulled.replaced.as_deref(), Some(kept.as_str()));
        assert_eq!(clean_up(&driver, &cfg).await, [kept.as_str()]);
        assert_eq!(cli.images()?, [(image.clone(), new)]);
        let calls = cli.calls();
        for call in [
            format!("image tag {image} {kept}"),
            "image list --quiet".to_owned(),
            format!("image delete {kept}"),
        ] {
            assert!(calls.contains(&call), "{call} in {calls:?}");
        }
        let pulled =
            |c: &String| c.starts_with("image pull --platform linux/") && c.ends_with(&image);
        assert!(calls.iter().any(pulled), "{calls:?}");
        Ok(())
    }

    #[tokio::test]
    async fn hosts_on_another_image_leave_the_default_ones_alone() -> Result<()> {
        let cfg = HostConfig {
            image: Some("ghcr.io/acme/agent:1".into()),
            ..HostConfig::default()
        };
        let (image, cli, driver) = setup(Flavor::Docker)?;
        cli.have("ghcr.io/acme/agent:1", &id("acme"), Some("2.1.302"))?;
        cli.publish("ghcr.io/acme/agent:1", &id("acme 2"), "2.1.310")?;
        cli.have(&image, &id("default"), Some("2.1.302"))?;
        cli.have(&name("0.0.1"), &id("0.0.1"), Some("2.1.200"))?;
        refresh(&driver, &cfg).await;
        assert!(clean_up(&driver, &cfg).await.is_empty());
        assert_eq!(cli.images()?.len(), 3);
        assert!(!cli.calls().iter().any(|c| c.contains("pull")));
        Ok(())
    }
}
