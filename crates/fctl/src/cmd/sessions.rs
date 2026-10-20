//! `fctl run`, `sessions`, `show`, `edit`, `logs`, `send`, `interrupt`, `stop`, `resume`,
//! `follow`, `unfollow`, `open`.

use crate::config;
use crate::render::Renderer;
use crate::style;
use anyhow::{Context, Result, bail};
use clap::Args;
use fctl_client::Api;
use fctl_domain::files;
use fctl_proto::agent::{SESSION_SOCKET_ENV, SessionReply, SessionRequest};
use fctl_proto::api::{CreateSessionRequest, UpdateSessionRequest};
use fctl_proto::session::Session;
use fctl_proto::{AttachmentId, SessionId};
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

#[derive(Args)]
pub struct RunArgs {
    /// What the agent should do (or pipe it in, or use --file).
    pub prompt: Vec<String>,
    /// The project (slug or id); default from `fctl projects use`.
    #[arg(short, long)]
    pub project: Option<String>,
    /// Read the prompt from a file.
    #[arg(short, long)]
    pub file: Option<std::path::PathBuf>,
    /// Session title (default: from the prompt).
    #[arg(long)]
    pub title: Option<String>,
    /// Branch to work on (default: fctl/<number>-<title>).
    #[arg(long)]
    pub branch: Option<String>,
    /// Branch to start from (default: the project's default branch).
    #[arg(long)]
    pub base: Option<String>,
    /// Claude model (opus, sonnet, fable, or a full model name).
    #[arg(long)]
    pub model: Option<String>,
    /// Effort: low, medium, high, xhigh or max.
    #[arg(long)]
    pub effort: Option<String>,
    /// Start it and return instead of following along.
    #[arg(short, long)]
    pub detach: bool,
    /// Send a file with the task (repeat for more, up to 10).
    #[arg(short, long = "attach", value_name = "FILE")]
    pub attach: Vec<PathBuf>,
}

#[derive(Args)]
#[command(group(
    clap::ArgGroup::new("change")
        .required(true)
        .multiple(true)
        .args(["title", "description"])
))]
pub struct EditArgs {
    /// A session: its id (ses_…), its URL, or project#number. In an fctl
    /// sandbox, leave it out to edit the session the sandbox runs.
    pub session: Option<String>,
    /// The new title: what the work is, in at most 80 characters.
    #[arg(long)]
    pub title: Option<String>,
    /// What the session is about, in a sentence or two ("" clears it).
    #[arg(long)]
    pub description: Option<String>,
}

#[derive(Args)]
pub struct ListArgs {
    /// Only this project's sessions (slug or id).
    #[arg(short, long)]
    pub project: Option<String>,
    /// Only this workspace's sessions (slug or id).
    #[arg(short, long, conflicts_with = "project")]
    pub workspace: Option<String>,
    /// Only active sessions.
    #[arg(short, long)]
    pub active: bool,
    /// How many to show.
    #[arg(long, default_value_t = 30)]
    pub limit: u32,
}

/// Resolves `ses_…`, a URL ending in it, or `project#12`.
pub async fn resolve(api: &Api, reference: &str) -> Result<Session> {
    let r = reference.trim();
    if let Some(id) = r.rsplit('/').next().filter(|s| s.starts_with("ses_")) {
        return Ok(api.session(id).await?);
    }
    if let Some((project, number)) = r.split_once('#') {
        let n: u32 = number
            .parse()
            .context("use project#number, like my-app#12")?;
        let list = api.sessions(Some(project), None, 200).await?;
        return list
            .sessions
            .into_iter()
            .find(|s| s.number == n)
            .with_context(|| {
                format!("{project} has no session #{n} (or it is older than the last 200)")
            });
    }
    bail!("Refer to a session by id (ses_…) or as project#number")
}

pub fn project_or_default(p: Option<String>) -> Result<String> {
    p.or_else(|| config::CliConfig::load().project).context(
        "Which project? Pass --project, or set a default with `fctl projects use <project>`.",
    )
}

/// Where files upload: a project (for a new session) or a session.
pub enum UploadTo<'a> {
    Project(&'a str),
    Session(&'a str),
}

/// Uploads files to send with a message; returns their ids.
pub async fn upload(api: &Api, to: UploadTo<'_>, paths: &[PathBuf]) -> Result<Vec<AttachmentId>> {
    if paths.len() > files::MAX_FILES_PER_MESSAGE {
        bail!(
            "Attach at most {} files to one message",
            files::MAX_FILES_PER_MESSAGE
        );
    }
    let mut ids = Vec::with_capacity(paths.len());
    for path in paths {
        let size = std::fs::metadata(path)
            .with_context(|| format!("reading {}", path.display()))?
            .len();
        if size > files::MAX_FILE_BYTES {
            bail!(
                "{} is {}; files can be at most {}",
                path.display(),
                files::human_size(size),
                files::human_size(files::MAX_FILE_BYTES)
            );
        }
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".into());
        let media_type = Some(files::guess_media_type(&name));
        let att = match to {
            UploadTo::Project(p) => api.upload_attachment(p, &name, media_type, bytes).await,
            UploadTo::Session(s) => {
                api.upload_session_attachment(s, &name, media_type, bytes)
                    .await
            }
        }
        .with_context(|| format!("uploading {}", path.display()))?;
        ids.push(att.id);
    }
    Ok(ids)
}

pub async fn start(server: Option<&str>, args: RunArgs, json: bool) -> Result<()> {
    let api = config::api(server)?;
    let project = project_or_default(args.project)?;
    let prompt = if let Some(f) = &args.file {
        std::fs::read_to_string(f).with_context(|| format!("reading {}", f.display()))?
    } else if !args.prompt.is_empty() {
        args.prompt.join(" ")
    } else if !std::io::stdin().is_terminal() {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        s
    } else if !args.attach.is_empty() {
        String::new()
    } else {
        bail!("What should the agent do? `fctl run -p {project} \"Fix the flaky login test\"`");
    };
    let attachments = upload(&api, UploadTo::Project(&project), &args.attach).await?;
    let session = api
        .create_session(
            &project,
            &CreateSessionRequest {
                prompt,
                title: args.title,
                base_branch: args.base,
                branch: args.branch,
                model: args.model,
                effort: args.effort,
                attachments,
            },
        )
        .await?;
    if json {
        return style::print_json(&session);
    }
    println!(
        "{} {} {}",
        style::accent("✓"),
        style::bold(&format!(
            "{}#{} {}",
            session.project_slug, session.number, session.title
        )),
        style::dim(&format!("{}/s/{}", api.base(), session.id))
    );
    if args.detach {
        return Ok(());
    }
    crate::cmd::attach::follow(&api, &session.id.0, false).await
}

pub async fn list(server: Option<&str>, args: ListArgs, json: bool) -> Result<()> {
    let api = config::api(server)?;
    let project = match &args.workspace {
        Some(_) => None,
        None => args.project.or_else(|| config::CliConfig::load().project),
    };
    let status = args.active.then_some("active");
    let list = api
        .sessions_in(
            args.workspace.as_deref(),
            project.as_deref(),
            status,
            args.limit,
        )
        .await?;
    if json {
        return style::print_json(&list);
    }
    if list.sessions.is_empty() {
        println!("No sessions. Start one with `fctl run`.");
    }
    for s in &list.sessions {
        let when = style::ago(s.last_activity_at.unwrap_or(s.created_at));
        let reference = format!("{}#{}", s.project_slug, s.number);
        println!(
            "{:<20} {:<18} {:<44} {}",
            style::bold(&reference),
            style::status(s.status),
            truncate(&s.title, 44),
            style::dim(&when)
        );
    }
    Ok(())
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_owned()
    } else {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    }
}

pub async fn show(server: Option<&str>, reference: &str, json: bool) -> Result<()> {
    let api = config::api(server)?;
    let s = resolve(&api, reference).await?;
    if json {
        return style::print_json(&s);
    }
    println!(
        "{} {}",
        style::bold(&format!("{}#{}", s.project_slug, s.number)),
        s.title
    );
    if let Some(d) = &s.description {
        println!("{}", style::dim(d));
    }
    println!(
        "status:   {}{}",
        style::status(s.status),
        s.status_reason
            .as_deref()
            .map(|r| style::dim(&format!(" — {r}")))
            .unwrap_or_default()
    );
    println!("by:       {}", s.created_by.name);
    if let Some(b) = &s.branch {
        println!(
            "branch:   {b}{}",
            s.base_branch
                .as_deref()
                .map(|x| style::dim(&format!(" (from {x})")))
                .unwrap_or_default()
        );
    }
    if let Some(pr) = &s.pull_request {
        println!("pr:       #{} {}", pr.number, pr.url);
    }
    if let Some(r) = &s.runner {
        println!("runner:   {}", r.name);
    }
    if let Some(m) = &s.model {
        println!("model:    {m}");
    }
    if let Some(following) = s.following {
        println!("follow:   {}", if following { "yes" } else { "no" });
    }
    let st = &s.stats;
    println!(
        "work:     {} turns · {} tool calls · {} files (+{} −{})",
        st.turns, st.tool_calls, st.files_changed, st.additions, st.deletions
    );
    if let Some(rc) = s
        .remote_control
        .as_ref()
        .and_then(|r| r.session_url.as_ref())
    {
        println!("claude:   {rc}");
    }
    println!("{}", style::dim(&format!("{}/s/{}", api.base(), s.id)));
    Ok(())
}

/// `fctl edit`. In the sandbox of the session it edits (an agent naming its
/// own session) it asks fctl-agent over the session socket and needs no
/// sign-in; anywhere else it goes through the API as the signed-in person.
pub async fn edit(server: Option<&str>, args: EditArgs, json: bool) -> Result<()> {
    if let Some((socket, id)) = this_sandbox(args.session.as_deref()) {
        return edit_here(&socket, id, args, json).await;
    }
    let reference = args
        .session
        .as_deref()
        .context("Which session? `fctl edit my-app#12 --title \"Fix the flaky login test\"`")?;
    let api = config::api(server)?;
    let s = resolve(&api, reference).await?;
    let s = api
        .update_session(
            &s.id.0,
            &UpdateSessionRequest {
                title: args.title,
                description: args.description,
                archived: None,
            },
        )
        .await?;
    if json {
        return style::print_json(&s);
    }
    print_edited(
        &format!("{}#{}", s.project_slug, s.number),
        &s.title,
        s.description.as_deref(),
    );
    Ok(())
}

pub(crate) fn this_sandbox(reference: Option<&str>) -> Option<(PathBuf, String)> {
    sandbox_session(
        std::env::var_os(SESSION_SOCKET_ENV),
        std::env::var("FCTL_SESSION_ID").ok(),
        reference,
    )
}

/// The session socket and the session's id, when this runs in an fctl
/// sandbox and `reference` is that sandbox's session (or absent).
fn sandbox_session(
    socket: Option<std::ffi::OsString>,
    id: Option<String>,
    reference: Option<&str>,
) -> Option<(PathBuf, String)> {
    let socket = socket.filter(|s| !s.is_empty())?;
    let id = id.filter(|id| !id.is_empty())?;
    let here = reference.is_none_or(|r| r.trim().rsplit('/').next() == Some(id.as_str()));
    here.then(|| (PathBuf::from(socket), id))
}

async fn edit_here(socket: &Path, id: String, args: EditArgs, json: bool) -> Result<()> {
    let request = SessionRequest::Edit {
        session_id: Some(SessionId(id)),
        title: args.title,
        description: args.description,
    };
    let reply = tokio::time::timeout(Duration::from_secs(15), ask_agent(socket, &request))
        .await
        .context("fctl-agent did not answer in time")??;
    match reply {
        SessionReply::Edited {
            ref project_slug,
            number,
            ref title,
            ref description,
            ..
        } => {
            if json {
                // The fields a session has, without the reply's tag.
                let mut v = serde_json::to_value(&reply)?;
                if let Some(o) = v.as_object_mut() {
                    o.remove("type");
                }
                return style::print_json(&v);
            }
            print_edited(
                &format!("{project_slug}#{number}"),
                title,
                description.as_deref(),
            );
            Ok(())
        }
        SessionReply::Error { error } => bail!("{}", error.message),
        SessionReply::Published { .. } => bail!("fctl-agent answered another request"),
    }
}

pub(crate) async fn ask_agent(socket: &Path, request: &SessionRequest) -> Result<SessionReply> {
    let stream = UnixStream::connect(socket)
        .await
        .with_context(|| format!("fctl-agent is not answering on {}", socket.display()))?;
    let (read, mut write) = stream.into_split();
    let mut line = serde_json::to_string(request)?;
    line.push('\n');
    write.write_all(line.as_bytes()).await?;
    let mut reply = String::new();
    BufReader::new(read).read_line(&mut reply).await?;
    serde_json::from_str(reply.trim()).context("reading fctl-agent's answer")
}

fn print_edited(reference: &str, title: &str, description: Option<&str>) {
    println!("{} {} {title}", style::accent("✓"), style::bold(reference));
    if let Some(d) = description {
        println!("  {}", style::dim(d));
    }
}

pub async fn logs(server: Option<&str>, reference: &str, follow: bool, json: bool) -> Result<()> {
    let api = config::api(server)?;
    let s = resolve(&api, reference).await?;
    let mut r = Renderer::default();
    r.json = json;
    let mut after = 0u64;
    loop {
        let page = api.events(&s.id.0, after, 1000).await?;
        for ev in &page.events {
            after = after.max(ev.seq);
            r.event(ev);
        }
        if !page.has_more {
            break;
        }
    }
    if follow {
        return crate::cmd::attach::follow_after(&api, &s.id.0, true, Some(after), r).await;
    }
    Ok(())
}

pub async fn send(
    server: Option<&str>,
    reference: &str,
    message: &str,
    attach: &[PathBuf],
    json: bool,
) -> Result<()> {
    let api = config::api(server)?;
    let s = resolve(&api, reference).await?;
    let text = if message.trim().is_empty() && !std::io::stdin().is_terminal() {
        let mut t = String::new();
        std::io::stdin().read_to_string(&mut t)?;
        t
    } else {
        message.to_owned()
    };
    let attachments = upload(&api, UploadTo::Session(&s.id.0), attach).await?;
    let r = api.send_message(&s.id.0, &text, None, attachments).await?;
    if json {
        return style::print_json(&r);
    }
    let resumes = if s.status.resumes_for_messages() {
        "; the session resumes for it"
    } else {
        ""
    };
    println!(
        "{} Sent to {}#{}{resumes}",
        style::accent("✓"),
        s.project_slug,
        s.number
    );
    Ok(())
}

pub async fn interrupt(server: Option<&str>, reference: &str) -> Result<()> {
    let api = config::api(server)?;
    let s = resolve(&api, reference).await?;
    api.interrupt(&s.id.0).await?;
    println!("Interrupted {}#{}", s.project_slug, s.number);
    Ok(())
}

pub async fn stop(server: Option<&str>, reference: &str, json: bool) -> Result<()> {
    let api = config::api(server)?;
    let s = resolve(&api, reference).await?;
    let s = api.stop(&s.id.0).await?;
    if json {
        return style::print_json(&s);
    }
    println!(
        "{} {}#{}",
        style::status(s.status),
        s.project_slug,
        s.number
    );
    Ok(())
}

pub async fn resume(
    server: Option<&str>,
    reference: &str,
    message: &str,
    attach: &[PathBuf],
    json: bool,
) -> Result<()> {
    let api = config::api(server)?;
    let s = resolve(&api, reference).await?;
    let prompt = (!message.trim().is_empty()).then(|| message.to_owned());
    let attachments = upload(&api, UploadTo::Session(&s.id.0), attach).await?;
    let s = api.resume(&s.id.0, prompt, attachments).await?;
    if json {
        return style::print_json(&s);
    }
    println!(
        "{} Resuming {}#{}",
        style::accent("✓"),
        s.project_slug,
        s.number
    );
    Ok(())
}

/// `fctl follow` (`follow`) and `fctl unfollow`.
pub async fn follow(server: Option<&str>, reference: &str, follow: bool, json: bool) -> Result<()> {
    let api = config::api(server)?;
    let s = resolve(&api, reference).await?;
    let s = if follow {
        api.follow(&s.id.0).await?
    } else {
        api.unfollow(&s.id.0).await?
    };
    if json {
        return style::print_json(&s);
    }
    let name = format!("{}#{}", s.project_slug, s.number);
    if follow {
        println!(
            "{} Following {name}: you get a notification when it waits for your reply, pauses on a usage limit or fails",
            style::accent("✓")
        );
        let devices = api
            .notifications()
            .await
            .map(|n| n.devices.len())
            .unwrap_or(1);
        if devices == 0 {
            println!(
                "{}",
                style::dim(&format!(
                    "No browser gets your notifications yet. Turn them on in Settings: {}/settings",
                    api.base()
                ))
            );
        }
    } else {
        println!("{} Not following {name} any more", style::accent("✓"));
    }
    Ok(())
}

pub async fn open(server: Option<&str>, target: Option<&str>, claude: bool) -> Result<()> {
    let api = config::api(server)?;
    let url = match target {
        None => api.base().to_owned(),
        Some(t) if t.starts_with("ses_") || t.contains('#') => {
            let s = resolve(&api, t).await?;
            if claude {
                let url = s.remote_control.and_then(|r| r.session_url).context(
                    "This session has no Remote Control link (it needs a full Claude login)",
                )?;
                // The Claude app opens the session itself; without one, the browser does.
                if let Some(app) = fctl_domain::remote_control::claude_app_url(&url)
                    && crate::cmd::auth::open_claude_app(&app)
                {
                    return Ok(());
                }
                url
            } else {
                format!("{}/s/{}", api.base(), s.id)
            }
        }
        Some(p) => {
            let p = api.project(p).await?;
            format!("{}/p/{}", api.base(), p.slug)
        }
    };
    if !crate::cmd::auth::open_browser(&url) {
        println!("{url}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_go_to_fctl_agent_only_for_its_own_session() {
        let here = |reference: Option<&str>| {
            sandbox_session(
                Some("/home/agent/.fctl/agent.session.sock".into()),
                Some("ses_01ABC".into()),
                reference,
            )
            .map(|(socket, id)| (socket.display().to_string(), id))
        };
        let socket = "/home/agent/.fctl/agent.session.sock".to_owned();
        assert_eq!(here(None), Some((socket.clone(), "ses_01ABC".into())));
        assert_eq!(
            here(Some("ses_01ABC")),
            Some((socket.clone(), "ses_01ABC".into()))
        );
        assert_eq!(
            here(Some("https://app.fctl.sh/s/ses_01ABC")),
            Some((socket, "ses_01ABC".into()))
        );
        assert_eq!(here(Some("my-app#12")), None);
        assert_eq!(here(Some("ses_01XYZ")), None);
        // Outside a sandbox, or with half of its environment.
        assert_eq!(sandbox_session(None, Some("ses_01ABC".into()), None), None);
        assert_eq!(
            sandbox_session(Some("/s.sock".into()), Some(String::new()), None),
            None
        );
    }
}
