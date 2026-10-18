//! A typed client for the control plane's REST API (`/api/v1`).

use crate::http::{Error, HttpClient, Result};
use fctl_proto::AttachmentId;
use fctl_proto::api::*;
use fctl_proto::event::Attachment;
use fctl_proto::session::{Project, Session};
use http::Method;
use serde::Serialize;
use serde::de::DeserializeOwned;

#[derive(Clone)]
pub struct Api {
    base: String,
    token: Option<String>,
    http: HttpClient,
}

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

impl Api {
    /// `base` is the instance URL, e.g. `https://app.fctl.sh`.
    pub fn new(base: &str, token: Option<String>) -> Self {
        Self {
            base: base.trim_end_matches('/').to_owned(),
            token,
            http: HttpClient::default(),
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}/api/v1{path}", self.base)
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.http
            .json::<(), T>(Method::GET, &self.url(path), self.token(), None)
            .await
    }

    async fn post<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        self.http
            .json(Method::POST, &self.url(path), self.token(), Some(body))
            .await
    }

    async fn patch<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        self.http
            .json(Method::PATCH, &self.url(path), self.token(), Some(body))
            .await
    }

    async fn delete<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.http
            .json::<(), T>(Method::DELETE, &self.url(path), self.token(), None)
            .await
    }

    // ── instance and auth ──

    pub async fn instance(&self) -> Result<InstanceInfo> {
        self.get("/instance").await
    }

    pub async fn me(&self) -> Result<Me> {
        self.get("/auth/me").await
    }

    pub async fn device_start(&self, client_name: &str) -> Result<DeviceStartResponse> {
        self.post(
            "/auth/device",
            &DeviceStartRequest {
                client_name: client_name.to_owned(),
            },
        )
        .await
    }

    pub async fn device_poll(&self, device_code: &str) -> Result<DevicePollResponse> {
        self.post(
            "/auth/device/token",
            &DevicePollRequest {
                device_code: device_code.to_owned(),
            },
        )
        .await
    }

    pub async fn logout(&self) -> Result<Ok> {
        self.post("/auth/logout", &serde_json::json!({})).await
    }

    // ── your account ──

    /// Your personal data, as the JSON document `GET /me/export` streams.
    pub async fn export(&self) -> Result<Vec<u8>> {
        // Transcripts can be large: allow far longer than a normal request.
        let http = HttpClient::new(std::time::Duration::from_secs(600));
        let (status, _, bytes) = http
            .send(
                Method::GET,
                &self.url("/me/export"),
                self.token(),
                None,
                None,
            )
            .await?;
        if !status.is_success() {
            if let Ok(e) = serde_json::from_slice::<fctl_proto::ApiError>(&bytes) {
                return Err(Error::Api(e));
            }
            return Err(Error::Status {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&bytes).chars().take(500).collect(),
            });
        }
        Ok(bytes.to_vec())
    }

    /// What deleting your account would delete, and whether it can be deleted now.
    pub async fn account_deletion(&self) -> Result<AccountDeletion> {
        self.get("/me/deletion").await
    }

    /// Deletes your account; `email` is your address, typed to confirm.
    pub async fn delete_account(&self, email: &str) -> Result<Ok> {
        self.post(
            "/me/delete",
            &DeleteAccountRequest {
                email: email.to_owned(),
            },
        )
        .await
    }

    pub async fn tokens(&self) -> Result<TokenList> {
        self.get("/auth/tokens").await
    }

    pub async fn create_token(&self, name: &str, days: Option<u32>) -> Result<CreatedToken> {
        self.post(
            "/auth/tokens",
            &CreateTokenRequest {
                name: name.to_owned(),
                expires_in_days: days,
            },
        )
        .await
    }

    // ── projects ──

    pub async fn projects(&self) -> Result<ProjectList> {
        self.get("/projects").await
    }

    pub async fn project(&self, id_or_slug: &str) -> Result<Project> {
        self.get(&format!("/projects/{}", enc(id_or_slug))).await
    }

    /// Deletes a project with its sessions and their transcripts (owners only;
    /// its sessions must be stopped).
    pub async fn delete_project(&self, id: &str) -> Result<Ok> {
        self.delete(&format!("/projects/{}", enc(id))).await
    }

    pub async fn create_project(&self, req: &CreateProjectRequest) -> Result<Project> {
        self.post("/projects", req).await
    }

    pub async fn update_project(&self, id: &str, req: &UpdateProjectRequest) -> Result<Project> {
        self.patch(&format!("/projects/{}", enc(id)), req).await
    }

    pub async fn github_repos(&self) -> Result<GithubRepoList> {
        self.get("/github/repos").await
    }

    // ── sessions ──

    pub async fn sessions(
        &self,
        project: Option<&str>,
        status: Option<&str>,
        limit: u32,
    ) -> Result<SessionList> {
        let mut q = format!("/sessions?limit={limit}");
        if let Some(p) = project {
            q.push_str(&format!("&project={}", enc(p)));
        }
        if let Some(s) = status {
            q.push_str(&format!("&status={}", enc(s)));
        }
        self.get(&q).await
    }

    pub async fn session(&self, id: &str) -> Result<Session> {
        self.get(&format!("/sessions/{}", enc(id))).await
    }

    pub async fn update_session(&self, id: &str, req: &UpdateSessionRequest) -> Result<Session> {
        self.patch(&format!("/sessions/{}", enc(id)), req).await
    }

    pub async fn create_session(
        &self,
        project: &str,
        req: &CreateSessionRequest,
    ) -> Result<Session> {
        self.post(&format!("/projects/{}/sessions", enc(project)), req)
            .await
    }

    pub async fn send_message(
        &self,
        session: &str,
        text: &str,
        client_id: Option<String>,
        attachments: Vec<AttachmentId>,
    ) -> Result<SendMessageResponse> {
        self.post(
            &format!("/sessions/{}/messages", enc(session)),
            &SendMessageRequest {
                text: text.to_owned(),
                client_id,
                attachments,
            },
        )
        .await
    }

    // ── attachments ──

    /// Uploads a file to start a session with (send it by id). Unsent
    /// uploads are deleted after a day.
    pub async fn upload_attachment(
        &self,
        project: &str,
        name: &str,
        media_type: Option<&str>,
        bytes: Vec<u8>,
    ) -> Result<Attachment> {
        self.upload(
            &format!("/projects/{}", enc(project)),
            name,
            media_type,
            bytes,
        )
        .await
    }

    /// Uploads a file to send with a message or resume in a session.
    pub async fn upload_session_attachment(
        &self,
        session: &str,
        name: &str,
        media_type: Option<&str>,
        bytes: Vec<u8>,
    ) -> Result<Attachment> {
        self.upload(
            &format!("/sessions/{}", enc(session)),
            name,
            media_type,
            bytes,
        )
        .await
    }

    async fn upload(
        &self,
        scope: &str,
        name: &str,
        media_type: Option<&str>,
        bytes: Vec<u8>,
    ) -> Result<Attachment> {
        let mut path = format!("{scope}/attachments?name={}", enc(name));
        if let Some(t) = media_type {
            path.push_str(&format!("&type={}", enc(t)));
        }
        let body = self
            .raw(
                Method::POST,
                &path,
                Some(bytes),
                Some("application/octet-stream"),
            )
            .await?;
        serde_json::from_slice(&body).map_err(|e| Error::Decode(e.to_string()))
    }

    /// A file sent in a session.
    pub async fn attachment(&self, session: &str, attachment: &str) -> Result<Vec<u8>> {
        let path = format!("/sessions/{}/attachments/{}", enc(session), enc(attachment));
        Ok(self.raw(Method::GET, &path, None, None).await?.to_vec())
    }

    /// A file sent in a session this runner runs (with the runner token).
    pub async fn runner_attachment(&self, session: &str, attachment: &str) -> Result<Vec<u8>> {
        let path = format!(
            "/runner/sessions/{}/attachments/{}",
            enc(session),
            enc(attachment)
        );
        Ok(self.raw(Method::GET, &path, None, None).await?.to_vec())
    }

    /// A request with a raw body and answer; files can be large, so it may
    /// take minutes.
    async fn raw(
        &self,
        method: Method,
        path: &str,
        body: Option<Vec<u8>>,
        content_type: Option<&str>,
    ) -> Result<bytes::Bytes> {
        let http = HttpClient::new(std::time::Duration::from_secs(600));
        let (status, _, bytes) = http
            .send(method, &self.url(path), self.token(), body, content_type)
            .await?;
        if !status.is_success() {
            if let Ok(e) = serde_json::from_slice::<fctl_proto::ApiError>(&bytes) {
                return Err(Error::Api(e));
            }
            return Err(Error::Status {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&bytes).chars().take(500).collect(),
            });
        }
        Ok(bytes)
    }

    pub async fn interrupt(&self, session: &str) -> Result<SendMessageResponse> {
        self.post(
            &format!("/sessions/{}/interrupt", enc(session)),
            &serde_json::json!({}),
        )
        .await
    }

    pub async fn stop(&self, session: &str) -> Result<Session> {
        self.post(
            &format!("/sessions/{}/stop", enc(session)),
            &serde_json::json!({}),
        )
        .await
    }

    pub async fn resume(
        &self,
        session: &str,
        prompt: Option<String>,
        attachments: Vec<AttachmentId>,
    ) -> Result<Session> {
        self.post(
            &format!("/sessions/{}/resume", enc(session)),
            &ResumeSessionRequest {
                prompt,
                attachments,
            },
        )
        .await
    }

    pub async fn events(&self, session: &str, after: u64, limit: u32) -> Result<EventPage> {
        self.get(&format!(
            "/sessions/{}/events?after={after}&limit={limit}",
            enc(session)
        ))
        .await
    }

    pub fn session_ws_url(&self, session: &str, after: Option<u64>) -> String {
        let mut u = self.url(&format!("/sessions/{}/ws", enc(session)));
        if let Some(a) = after {
            u.push_str(&format!("?after={a}"));
        }
        u
    }

    // ── runners ──

    pub async fn runners(&self) -> Result<RunnerList> {
        self.get("/runners").await
    }

    pub async fn create_runner(&self, name: &str) -> Result<RunnerRegistration> {
        self.post(
            "/runners",
            &CreateRunnerRequest {
                name: name.to_owned(),
            },
        )
        .await
    }

    pub async fn delete_runner(&self, id: &str) -> Result<Ok> {
        self.delete(&format!("/runners/{}", enc(id))).await
    }

    // ── Claude ──

    pub async fn add_claude_token(&self, token: &str, label: Option<String>) -> Result<Me> {
        self.post(
            "/me/claude/token",
            &AddClaudeTokenRequest {
                token: token.to_owned(),
                label,
            },
        )
        .await
    }

    pub async fn start_claude_login(&self) -> Result<ClaudeLoginJob> {
        self.post("/me/claude/login", &serde_json::json!({})).await
    }

    pub async fn claude_login(&self, id: &str) -> Result<ClaudeLoginJob> {
        self.get(&format!("/me/claude/login/{}", enc(id))).await
    }

    pub async fn submit_claude_code(&self, id: &str, code: &str) -> Result<ClaudeLoginJob> {
        self.post(
            &format!("/me/claude/login/{}/code", enc(id)),
            &ClaudeLoginCodeRequest {
                code: code.to_owned(),
            },
        )
        .await
    }

    // ── admin ──

    pub async fn users(&self) -> Result<UserList> {
        self.get("/admin/users").await
    }

    pub async fn allowlist(&self) -> Result<AllowList> {
        self.get("/admin/allowlist").await
    }

    pub async fn allow(
        &self,
        pattern: &str,
        role: Option<fctl_proto::session::InstanceRole>,
    ) -> Result<CreatedAllow> {
        self.post(
            "/admin/allowlist",
            &CreateAllowRequest {
                pattern: pattern.to_owned(),
                role,
                note: None,
                invite: Some(true),
            },
        )
        .await
    }

    pub async fn disallow(&self, id: &str) -> Result<serde_json::Value> {
        self.delete(&format!("/admin/allowlist/{}", enc(id))).await
    }
}
