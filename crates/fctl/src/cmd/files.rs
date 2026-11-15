//! `fctl files`: the files sent in a session, with its messages or as it
//! started: list them, and download them. An autopilot reads those of the
//! sessions it reads this way, through its session's relay, and passes them
//! on with `fctl run --attach` and `fctl send --attach`.

use crate::config;
use crate::style;
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use fctl_domain::files;
use fctl_proto::api::SessionFile;
use std::path::{Path, PathBuf};

#[derive(Subcommand)]
pub enum FilesCmd {
    /// List the files sent in a session, oldest first.
    #[command(visible_alias = "ls")]
    List {
        /// A session: its id (ses_…), its URL, or project#number.
        session: String,
    },
    /// Download files sent in a session, each under its name.
    Get(GetArgs),
}

#[derive(Args)]
pub struct GetArgs {
    /// A session: its id (ses_…), its URL, or project#number.
    pub session: String,
    /// The files, by name (the newest of that name) or by id (att_…).
    #[arg(value_name = "FILE")]
    pub files: Vec<String>,
    /// Every file sent in it (the newest of each name).
    #[arg(long, conflicts_with = "files")]
    pub all: bool,
    /// The directory to write them to (default: the current one).
    #[arg(long, value_name = "DIR")]
    pub dir: Option<PathBuf>,
    /// Write over files of the same names there: the names are those of
    /// whoever sent the files.
    #[arg(long)]
    pub force: bool,
}

pub async fn run(server: Option<&str>, cmd: FilesCmd, json: bool) -> Result<()> {
    match cmd {
        FilesCmd::List { session } => list(server, &session, json).await,
        FilesCmd::Get(args) => get(server, args, json).await,
    }
}

async fn list(server: Option<&str>, reference: &str, json: bool) -> Result<()> {
    let api = config::api(server)?;
    let s = super::sessions::resolve(&api, reference).await?;
    let found = api.session_files(s.id.as_str()).await?;
    if json {
        return style::print_json(&found);
    }
    if found.files.is_empty() {
        println!("No files were sent in {}#{}.", s.project_slug, s.number);
        return Ok(());
    }
    for f in &found.files {
        println!(
            "{:<40} {:>8}  {:<10} {}",
            style::plain(&f.file.name),
            files::human_size(f.file.size),
            style::ago(f.sent_at),
            style::dim(f.file.id.as_str())
        );
    }
    Ok(())
}

/// A file written, as `--json` prints it.
#[derive(serde::Serialize)]
struct Written<'a> {
    #[serde(flatten)]
    file: &'a SessionFile,
    path: String,
}

async fn get(server: Option<&str>, args: GetArgs, json: bool) -> Result<()> {
    if args.files.is_empty() && !args.all {
        bail!(
            "Name the files to download, or pass --all: `fctl files list {}` lists them",
            args.session
        );
    }
    let api = config::api(server)?;
    let s = super::sessions::resolve(&api, &args.session).await?;
    let found = api.session_files(s.id.as_str()).await?;
    let reference = format!("{}#{}", s.project_slug, s.number);
    let wanted = if args.all {
        newest_of_each(&found.files)
    } else {
        pick(&found.files, &args.files).map_err(|missing| {
            anyhow::anyhow!(
                "No file {missing} was sent in {reference}: `fctl files list {reference}` lists them"
            )
        })?
    };
    if let Some(name) = clashing_name(&wanted) {
        bail!(
            "Two of them would be written as {}: get them one at a time, each with a --dir of its own",
            name.display()
        );
    }
    let dir = args.dir.unwrap_or_else(|| PathBuf::from("."));
    // Nothing is written over unasked, before anything is downloaded.
    if !args.force
        && let Some(path) = wanted
            .iter()
            .map(|f| dir.join(file_name(&f.file.name)))
            .find(|p| p.symlink_metadata().is_ok())
    {
        bail!(
            "{} exists: pass --force to write over it, or --dir to write somewhere else",
            path.display()
        );
    }
    if !wanted.is_empty() {
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let mut written = Vec::with_capacity(wanted.len());
    for f in wanted {
        let bytes = api
            .attachment(s.id.as_str(), f.file.id.as_str())
            .await
            .with_context(|| format!("downloading {}", f.file.name))?;
        let path = dir.join(file_name(&f.file.name));
        write(&path, &bytes, args.force).with_context(|| format!("writing {}", path.display()))?;
        if !json {
            println!(
                "{} {} {}",
                style::accent("✓"),
                path.display(),
                style::dim(&files::human_size(bytes.len() as u64))
            );
        }
        written.push(Written {
            file: f,
            path: path.display().to_string(),
        });
    }
    if json {
        return style::print_json(&written);
    }
    if written.is_empty() {
        println!("No files were sent in {reference}.");
    }
    Ok(())
}

/// The files `asked` names, by id or name (the newest of that name), in
/// the order asked, each once; or the first name that matches none.
fn pick<'a>(sent: &'a [SessionFile], asked: &[String]) -> Result<Vec<&'a SessionFile>, String> {
    let mut out: Vec<&SessionFile> = Vec::with_capacity(asked.len());
    for a in asked {
        let found = sent
            .iter()
            .rev()
            .find(|f| f.file.id.as_str() == a)
            .or_else(|| sent.iter().rev().find(|f| &f.file.name == a))
            .ok_or_else(|| a.clone())?;
        if !out.iter().any(|f| f.file.id == found.file.id) {
            out.push(found);
        }
    }
    Ok(out)
}

/// The newest file of each name, oldest first: an older one of the same
/// name would be written over.
fn newest_of_each(sent: &[SessionFile]) -> Vec<&SessionFile> {
    sent.iter()
        .enumerate()
        .filter(|(i, f)| !sent[i + 1..].iter().any(|g| g.file.name == f.file.name))
        .map(|(_, f)| f)
        .collect()
}

/// Writes `bytes` to a new file at `path`, or over one there with `force`.
fn write(path: &Path, bytes: &[u8], force: bool) -> std::io::Result<()> {
    use std::io::Write;
    if force {
        return std::fs::write(path, bytes);
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(bytes)
}

/// The name two of `files` would both be written as, if any.
fn clashing_name(files: &[&SessionFile]) -> Option<PathBuf> {
    let names: Vec<PathBuf> = files.iter().map(|f| file_name(&f.file.name)).collect();
    names
        .iter()
        .enumerate()
        .find(|(i, n)| names[i + 1..].contains(n))
        .map(|(_, n)| n.clone())
}

/// The name to write a file under: the control plane's, without anything
/// that would make it a path elsewhere.
fn file_name(name: &str) -> PathBuf {
    let clean = files::sanitize_name(name);
    match Path::new(&clean).file_name() {
        Some(n) if n == clean.as_str() => PathBuf::from(clean),
        _ => PathBuf::from("file"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fctl_proto::AttachmentId;
    use fctl_proto::event::Attachment;

    fn sent(id: &str, name: &str, at: i64) -> SessionFile {
        SessionFile {
            file: Attachment {
                id: AttachmentId(id.into()),
                name: name.into(),
                media_type: "application/octet-stream".into(),
                size: 3,
                width: None,
                height: None,
            },
            sent_at: at,
        }
    }

    fn ids(files: &[&SessionFile]) -> Vec<String> {
        files.iter().map(|f| f.file.id.0.clone()).collect()
    }

    #[test]
    fn picks_by_id_or_the_newest_of_a_name() {
        let all = [
            sent("att_1", "model.3mf", 1),
            sent("att_2", "plate.gcode.3mf", 2),
            sent("att_3", "model.3mf", 3),
        ];
        let asked =
            |names: &[&str]| -> Vec<String> { names.iter().map(|n| (*n).to_owned()).collect() };
        assert_eq!(
            pick(&all, &asked(&["model.3mf"])).map(|f| ids(&f)),
            Ok(vec!["att_3".to_owned()])
        );
        assert_eq!(
            pick(&all, &asked(&["att_1", "plate.gcode.3mf", "att_1"])).map(|f| ids(&f)),
            Ok(vec!["att_1".to_owned(), "att_2".to_owned()])
        );
        assert_eq!(
            pick(&all, &asked(&["model.stl"])).map(|f| ids(&f)),
            Err("model.stl".to_owned())
        );
        assert_eq!(
            ids(&newest_of_each(&all)),
            vec!["att_2".to_owned(), "att_3".to_owned()]
        );
    }

    #[test]
    fn writes_no_file_over_another() -> anyhow::Result<()> {
        let all = [
            sent("att_1", "model.3mf", 1),
            sent("att_2", "plate.gcode.3mf", 2),
            sent("att_3", "model.3mf", 3),
        ];
        // Two files of one name in one go would write over each other.
        assert_eq!(clashing_name(&[&all[0], &all[1]]), None);
        assert_eq!(
            clashing_name(&[&all[0], &all[1], &all[2]]),
            Some(PathBuf::from("model.3mf"))
        );
        // A file there stays, unless asked.
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("AGENTS.md");
        std::fs::write(&path, "mine")?;
        assert!(write(&path, b"theirs", false).is_err());
        assert_eq!(std::fs::read_to_string(&path)?, "mine");
        write(&path, b"theirs", true)?;
        assert_eq!(std::fs::read_to_string(&path)?, "theirs");
        let new = dir.path().join("model.3mf");
        write(&new, b"abc", false)?;
        assert_eq!(std::fs::read(&new)?, b"abc");
        Ok(())
    }

    #[test]
    fn writes_a_file_only_under_its_name() {
        assert_eq!(file_name("model.3mf"), PathBuf::from("model.3mf"));
        assert_eq!(
            file_name("../../.ssh/authorized_keys"),
            PathBuf::from("authorized_keys")
        );
        assert_eq!(file_name("/etc/passwd"), PathBuf::from("passwd"));
        assert_eq!(file_name(".."), PathBuf::from("file"));
        assert_eq!(file_name(""), PathBuf::from("file"));
        assert_eq!(file_name("a\\b.png"), PathBuf::from("b.png"));
    }
}
