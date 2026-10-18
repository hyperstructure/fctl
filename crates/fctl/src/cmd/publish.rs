//! `fctl publish` and `fctl unpublish`: screenshots for pull requests, at
//! links on the control plane that anyone who has one can open until they
//! expire. Agents publish from their sandbox, through fctl-agent; people who
//! manage a session delete its screenshots sooner.

use crate::config;
use crate::style;
use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use clap::Args;
use fctl_domain::{files, published};
use fctl_proto::SessionId;
use fctl_proto::agent::{SessionReply, SessionRequest};
use fctl_proto::event::PublishedFile;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Args)]
pub struct PublishArgs {
    /// PNG, JPEG, GIF or WebP images of at most 5 MB each.
    #[arg(required = true, value_name = "FILE")]
    pub files: Vec<PathBuf>,
}

/// A published screenshot and the Markdown that shows it (`--json`).
#[derive(serde::Serialize)]
struct Published<'a> {
    #[serde(flatten)]
    file: &'a PublishedFile,
    markdown: String,
}

pub async fn publish(args: PublishArgs, json: bool) -> Result<()> {
    let Some((socket, id)) = super::sessions::this_sandbox(None) else {
        bail!(
            "fctl publish works in the sandbox of an fctl session, where agents publish screenshots for their pull requests"
        );
    };
    let mut done = Vec::with_capacity(args.files.len());
    for path in &args.files {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        if bytes.len() as u64 > published::MAX_BYTES {
            bail!(
                "{} is {}: screenshots can be at most {}, the most GitHub shows",
                path.display(),
                files::human_size(bytes.len() as u64),
                files::human_size(published::MAX_BYTES)
            );
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "screenshot".into());
        let request = SessionRequest::Publish {
            session_id: Some(SessionId(id.clone())),
            name,
            data: STANDARD.encode(&bytes),
        };
        // fctl-agent waits up to two minutes for the host to upload it.
        let reply = tokio::time::timeout(
            Duration::from_secs(180),
            super::sessions::ask_agent(&socket, &request),
        )
        .await
        .context("fctl-agent did not answer in time")??;
        let file = match reply {
            SessionReply::Published { file } => file,
            SessionReply::Error { error } => bail!("{}: {}", path.display(), error.message),
            SessionReply::Edited { .. } => bail!("fctl-agent answered another request"),
        };
        if !json {
            print_published(&file);
        }
        done.push(file);
    }
    if json {
        let list: Vec<Published> = done
            .iter()
            .map(|file| Published {
                file,
                markdown: published::markdown(file),
            })
            .collect();
        return style::print_json(&serde_json::json!({ "published": list }));
    }
    Ok(())
}

fn print_published(file: &PublishedFile) {
    let mut facts = Vec::new();
    if let (Some(w), Some(h)) = (file.width, file.height) {
        facts.push(format!("{w}×{h}"));
    }
    facts.push(files::human_size(file.size));
    println!(
        "{} Published {} {} {}",
        style::accent("✓"),
        style::bold(&file.name),
        style::dim(&format!("({})", facts.join(", "))),
        lasts(file.expires_at, now_ms())
    );
    println!("  {}", published::markdown(file));
}

/// How long a link works from now: `for 30 days`, `for 24 hours`.
fn lasts(expires_at: i64, now: i64) -> String {
    let hours = ((expires_at - now) as f64 / 3_600_000.0).round().max(1.0) as i64;
    match hours {
        1 => "for an hour".to_owned(),
        h if h <= 36 => format!("for {h} hours"),
        h => format!("for {} days", (h as f64 / 24.0).round() as i64),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub async fn unpublish(server: Option<&str>, links: &[String], json: bool) -> Result<()> {
    let api = config::api(server)?;
    for link in links {
        let token = published::token_in(link)
            .with_context(|| format!("{link} is not a link to a published screenshot"))?;
        api.unpublish(token).await?;
        if !json {
            println!("{} Deleted {link}", style::accent("✓"));
        }
    }
    if json {
        return style::print_json(&fctl_proto::api::Ok { ok: true });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_how_long_links_work() {
        let day = 86_400_000;
        assert_eq!(lasts(30 * day, 0), "for 30 days");
        assert_eq!(lasts(day, 0), "for 24 hours");
        assert_eq!(lasts(day + 60_000, 30_000), "for 24 hours");
        assert_eq!(lasts(2 * day, 0), "for 2 days");
        assert_eq!(lasts(3_600_000, 0), "for an hour");
        assert_eq!(lasts(0, 0), "for an hour");
    }

    #[test]
    fn json_has_the_markdown_beside_the_file() -> Result<()> {
        let file = PublishedFile {
            url: "https://fctl.test/api/v1/published/abc/after.png".into(),
            name: "after.png".into(),
            media_type: "image/png".into(),
            size: 10,
            width: Some(2),
            height: Some(1),
            expires_at: 5,
        };
        let v = serde_json::to_value(Published {
            file: &file,
            markdown: published::markdown(&file),
        })?;
        assert_eq!(v["url"], file.url);
        assert_eq!(
            v["markdown"],
            "![after.png](https://fctl.test/api/v1/published/abc/after.png)"
        );
        Ok(())
    }
}
