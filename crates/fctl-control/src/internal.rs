//! The Worker ⇄ Durable Object contract: internal requests and their bodies.
//!
//! Durable Objects are reachable only through stubs, so these requests are
//! trusted; the Worker never forwards client headers to them (it builds new
//! requests), and identity travels in `X-Fctl-*` headers it sets itself.

use crate::error::{AppError, AppResult, internal};
use fctl_proto::api::PlanLimit;
use fctl_proto::event::{Attachment, UserMessage};
use fctl_proto::runner::StopReason;
use fctl_proto::session::{RunnerRef, Session, SessionRole, SessionStatus, UserRef};
use fctl_proto::uplink::Delivery;
use fctl_proto::{
    ApiError, AttachmentId, CredentialId, DeliveryId, LoginJobId, Millis, ProjectId, SessionId,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use worker::{Env, Headers, Method, Request, RequestInit, Response, Stub};

pub const HDR_RUNNER: &str = "X-Fctl-Runner";
pub const HDR_VIEWER: &str = "X-Fctl-Viewer";

// ───────────────────────────── SessionObject ─────────────────────────────

#[derive(Serialize, Deserialize)]
pub struct InitSession {
    pub session: Session,
    pub prompt: UserMessage,
}

#[derive(Serialize, Deserialize)]
pub struct Deliver {
    pub delivery: Delivery,
    /// Idempotency key from the client; repeats return the first delivery.
    #[serde(default)]
    pub client_key: Option<String>,
    /// Stored for the transcript (the user message or interrupt).
    pub event: fctl_proto::event::EventBody,
}

#[derive(Serialize, Deserialize)]
pub struct Delivered {
    pub delivery_id: DeliveryId,
}

/// A status change decided by the control plane (queue, stop, failure).
#[derive(Serialize, Deserialize, Default)]
pub struct ControlState {
    pub status: Option<SessionStatus>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub runner: Option<RunnerRef>,
    #[serde(default)]
    pub clear_runner: bool,
    #[serde(default)]
    pub title: Option<String>,
    /// A new description; empty clears it.
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub archived: Option<bool>,
}

/// Who opens a viewer socket.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ViewerIdentity {
    pub user: UserRef,
    pub role: SessionRole,
}

// ───────────────────────────── FleetObject ─────────────────────────────

#[derive(Serialize, Deserialize)]
pub struct Schedule {
    pub session_id: SessionId,
}

#[derive(Serialize, Deserialize)]
pub struct Stop {
    pub session_id: SessionId,
    pub reason: StopReason,
    pub destroy: bool,
}

#[derive(Serialize, Deserialize)]
pub struct NotifySession {
    pub session: Session,
}

#[derive(Serialize, Deserialize)]
pub struct Activity {
    pub session_id: SessionId,
    pub project_id: ProjectId,
    pub kind: String,
    pub weight: u32,
}

/// Who opens a feed socket.
#[derive(Serialize, Deserialize, Clone)]
pub struct FeedIdentity {
    pub user_id: String,
    pub admin: bool,
    pub projects: Vec<String>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct LiveRunners {
    pub online: Vec<String>,
    pub last_seen: std::collections::BTreeMap<String, Millis>,
    /// What online runners had free at their last heartbeat.
    #[serde(default)]
    pub free: std::collections::BTreeMap<String, Free>,
}

/// Free memory and disk, in MiB.
#[derive(Serialize, Deserialize, Clone, Copy, Default)]
pub struct Free {
    pub memory_mb: Option<u64>,
    pub disk_mb: Option<u64>,
}

#[derive(Serialize, Deserialize)]
pub struct LoginStart {
    pub job_id: LoginJobId,
    pub user: UserRef,
}

#[derive(Serialize, Deserialize)]
pub struct LoginCode {
    pub job_id: LoginJobId,
    pub code: String,
}

/// A credential's plan usage changed: for its owner's feeds (`/usage`).
#[derive(Serialize, Deserialize)]
pub struct ClaudeUsage {
    pub user_id: String,
    pub credential_id: CredentialId,
    pub usage: Vec<PlanLimit>,
}

// ───────────────────────────── NotifyObject ─────────────────────────────

/// What happened to a session, for one of the people who follow it
/// (`/event`).
#[derive(Serialize, Deserialize)]
pub struct Notify {
    pub user_id: String,
    pub event: fctl_domain::notify::Event,
}

// ───────────────────────────── FilesObject ─────────────────────────────

/// A new upload, sent to `/store` in the `meta` query parameter; the bytes
/// are the request body.
#[derive(Serialize, Deserialize)]
pub struct StoreFile {
    pub id: AttachmentId,
    /// Already sanitized.
    pub name: String,
    /// The client's type or one guessed from the name; the object replaces it
    /// for images it recognizes.
    pub media_type: String,
    pub uploaded_by: String,
}

/// Uploads that a message in `session_id` sends: from now on they belong to
/// that session (again for the same session is fine).
#[derive(Serialize, Deserialize)]
pub struct ClaimFiles {
    pub session_id: SessionId,
    pub user_id: String,
    pub ids: Vec<AttachmentId>,
}

#[derive(Serialize, Deserialize)]
pub struct ClaimedFiles {
    /// In the order of the ids.
    pub attachments: Vec<Attachment>,
}

// ───────────────────────────── calling ─────────────────────────────

pub fn session_stub(env: &Env, session_id: &str) -> AppResult<Stub> {
    Ok(env
        .durable_object("SESSION")?
        .id_from_name(session_id)?
        .get_stub()?)
}

/// The FilesObject of a project.
pub fn files_stub(env: &Env, project_id: &str) -> AppResult<Stub> {
    Ok(env
        .durable_object("FILES")?
        .id_from_name(project_id)?
        .get_stub()?)
}

/// Claims uploads for a message in `session_id` and returns them; nothing to
/// do without ids.
pub async fn claim_files(
    env: &Env,
    project_id: &str,
    session_id: &SessionId,
    user_id: &str,
    ids: &[AttachmentId],
) -> AppResult<Vec<Attachment>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let stub = files_stub(env, project_id)?;
    let out: ClaimedFiles = post(
        &stub,
        "/claim",
        &ClaimFiles {
            session_id: session_id.clone(),
            user_id: user_id.to_owned(),
            ids: ids.to_vec(),
        },
    )
    .await?;
    Ok(out.attachments)
}

/// The NotifyObject of a person.
pub fn notify_stub(env: &Env, user_id: &str) -> AppResult<Stub> {
    Ok(env
        .durable_object("NOTIFY")?
        .id_from_name(user_id)?
        .get_stub()?)
}

pub fn fleet_stub(env: &Env) -> AppResult<Stub> {
    Ok(env
        .durable_object("FLEET")?
        .id_from_name("fleet")?
        .get_stub()?)
}

fn build(
    method: Method,
    path: &str,
    body: Option<String>,
    headers: &[(&str, &str)],
) -> AppResult<Request> {
    let h = Headers::new();
    if body.is_some() {
        h.set("Content-Type", "application/json")?;
    }
    for (k, v) in headers {
        h.set(k, v)?;
    }
    let mut init = RequestInit::new();
    init.with_method(method).with_headers(h);
    if let Some(b) = body {
        init.with_body(Some(b.into()));
    }
    Ok(Request::new_with_init(
        &format!("https://internal{path}"),
        &init,
    )?)
}

async fn decode<T: DeserializeOwned>(mut res: Response) -> AppResult<T> {
    let status = res.status_code();
    let text = res.text().await?;
    if status >= 400 {
        // DOs answer errors with an ApiError body.
        return Err(match serde_json::from_str::<ApiError>(&text) {
            Ok(e) => AppError(e),
            Err(_) => internal(format!("durable object answered {status}: {text}")),
        });
    }
    Ok(serde_json::from_str(&text)?)
}

/// Sends a request body through to a Durable Object unread (a
/// `ReadableStream`) and decodes the JSON answer.
pub async fn post_body<T: DeserializeOwned>(
    stub: &Stub,
    path: &str,
    body: wasm_bindgen::JsValue,
) -> AppResult<T> {
    let mut init = RequestInit::new();
    init.with_method(Method::Post).with_body(Some(body));
    let req = Request::new_with_init(&format!("https://internal{path}"), &init)?;
    decode(stub.fetch_with_request(req).await?).await
}

/// GETs from a Durable Object and returns its response as is (file bodies).
pub async fn fetch(stub: &Stub, path: &str) -> AppResult<Response> {
    let req = build(Method::Get, path, None, &[])?;
    Ok(stub.fetch_with_request(req).await?)
}

/// POSTs JSON to a Durable Object and decodes the JSON answer.
pub async fn post<B: Serialize, T: DeserializeOwned>(
    stub: &Stub,
    path: &str,
    body: &B,
) -> AppResult<T> {
    let req = build(Method::Post, path, Some(serde_json::to_string(body)?), &[])?;
    decode(stub.fetch_with_request(req).await?).await
}

pub async fn get<T: DeserializeOwned>(stub: &Stub, path: &str) -> AppResult<T> {
    let req = build(Method::Get, path, None, &[])?;
    decode(stub.fetch_with_request(req).await?).await
}

/// Forwards a WebSocket upgrade to a Durable Object with identity headers.
pub async fn upgrade(stub: &Stub, path: &str, headers: &[(&str, &str)]) -> AppResult<Response> {
    let mut all: Vec<(&str, &str)> = vec![("Upgrade", "websocket")];
    all.extend_from_slice(headers);
    let req = build(Method::Get, path, None, &all)?;
    Ok(stub.fetch_with_request(req).await?)
}

/// Inside a DO: an ApiError response.
pub fn do_error(e: &ApiError) -> worker::Result<Response> {
    Ok(crate::http::error_response(e))
}

/// Inside a DO: a JSON response.
pub fn do_json<T: Serialize>(body: &T) -> worker::Result<Response> {
    let text = serde_json::to_string(body).map_err(|e| worker::Error::RustError(e.to_string()))?;
    let h = Headers::new();
    h.set("Content-Type", "application/json")?;
    Ok(Response::ok(text)?.with_headers(h))
}
