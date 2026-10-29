//! A typed client for the control plane's REST API (`/api/v1`).

use crate::http::{Error, HttpClient, Result};
use fctl_proto::api::*;
use fctl_proto::event::{Attachment, PublishedFile};
use fctl_proto::session::{Memory, Project, Session};
use fctl_proto::tickets::{
    ConnectLinearRequest, LinearLink, LinearTeamList, MyLinear, ProjectTickets, WorkspaceLinear,
};
use fctl_proto::{ApiError, AttachmentId, ErrorCode};
use futures::future::BoxFuture;
use http::Method;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::future::Future;
use std::sync::Arc;

/// Asks the person to confirm it's them when the server answers that a
/// request needs it first (`verification_required`, with what it said),
/// and confirms with the client it gets: whether they did, so that the
/// request goes again. Without one, the error is the answer.
pub type Verifier = Arc<dyn Fn(Api, ApiError) -> BoxFuture<'static, Result<bool>> + Send + Sync>;

#[derive(Clone)]
pub struct Api {
    base: String,
    token: Option<String>,
    http: HttpClient,
    verifier: Option<Verifier>,
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
            verifier: None,
        }
    }

    /// The same client, asking `verifier` when a sensitive action needs the
    /// person to confirm it's them first, then sending it again.
    pub fn with_verifier(mut self, verifier: Verifier) -> Self {
        self.verifier = Some(verifier);
        self
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

    /// Sends a request with `send`, and once more after the person
    /// confirmed it's them, when the server asked for that and there is a
    /// [`Verifier`] to ask them.
    async fn confirming<T, F, Fut>(&self, send: F) -> Result<T>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        match send().await {
            Err(Error::Api(e)) if e.code == ErrorCode::VerificationRequired => {
                let Some(verifier) = self.verifier.clone() else {
                    return Err(Error::Api(e));
                };
                let api = Self {
                    verifier: None,
                    ..self.clone()
                };
                if verifier(api, e.clone()).await? {
                    send().await
                } else {
                    Err(Error::Api(e))
                }
            }
            answer => answer,
        }
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = self.url(path);
        self.confirming(|| {
            self.http
                .json::<(), T>(Method::GET, &url, self.token(), None)
        })
        .await
    }

    async fn post<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        let url = self.url(path);
        self.confirming(|| self.http.json(Method::POST, &url, self.token(), Some(body)))
            .await
    }

    async fn patch<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        let url = self.url(path);
        self.confirming(|| {
            self.http
                .json(Method::PATCH, &url, self.token(), Some(body))
        })
        .await
    }

    async fn put<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        let url = self.url(path);
        self.confirming(|| self.http.json(Method::PUT, &url, self.token(), Some(body)))
            .await
    }

    async fn delete<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = self.url(path);
        self.confirming(|| {
            self.http
                .json::<(), T>(Method::DELETE, &url, self.token(), None)
        })
        .await
    }

    // ── instance and auth ──

    pub async fn instance(&self) -> Result<InstanceInfo> {
        self.get("/instance").await
    }

    /// Asks for early access to the instance, without an account, where
    /// it takes requests (`InstanceInfo.features.early_access`). The
    /// answer's token withdraws the request.
    pub async fn request_access(&self, email: &str) -> Result<AccessRequested> {
        self.post(
            "/access-requests",
            &RequestAccessRequest {
                email: email.to_owned(),
            },
        )
        .await
    }

    /// Withdraws a request for early access, with the token its answer had.
    pub async fn withdraw_access_request(&self, token: &str) -> Result<Ok> {
        self.post(
            "/access-requests/withdraw",
            &WithdrawAccessRequest {
                token: token.to_owned(),
            },
        )
        .await
    }

    pub async fn me(&self) -> Result<Me> {
        self.get("/auth/me").await
    }

    /// Changes your profile; fields left out stay as they are.
    pub async fn update_me(&self, body: &UpdateMeRequest) -> Result<Me> {
        self.patch("/auth/me", body).await
    }

    /// Uploads your picture: a PNG, JPEG, GIF or WebP image.
    pub async fn set_avatar(&self, bytes: Vec<u8>) -> Result<Me> {
        let body = self
            .raw(
                Method::PUT,
                "/me/avatar",
                Some(bytes),
                Some("application/octet-stream"),
            )
            .await?;
        serde_json::from_slice(&body).map_err(|e| Error::Decode(e.to_string()))
    }

    /// Removes your picture: your initials show instead.
    pub async fn remove_avatar(&self) -> Result<Me> {
        self.delete("/me/avatar").await
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

    // ── confirming it's you ──

    /// Whether sensitive actions ask you to confirm it's you, with what, and
    /// until when this CLI or token counts as confirmed.
    pub async fn verification(&self) -> Result<Verification> {
        self.get("/auth/verify").await
    }

    /// Confirms it's you, with a code from your authenticator app or a
    /// recovery code: this CLI or token counts as confirmed for 15 minutes.
    pub async fn verify(&self, code: &str) -> Result<Verification> {
        self.post(
            "/auth/verify",
            &VerifyRequest {
                code: Some(code.to_owned()),
                passkey: None,
            },
        )
        .await
    }

    /// A code to approve in the browser, where passkeys work, which
    /// confirms it's you for this CLI or token; [`Api::device_poll`] says
    /// when it's done (`verified`).
    pub async fn verify_in_browser(&self, client_name: &str) -> Result<DeviceStartResponse> {
        self.post(
            "/auth/verify/device",
            &DeviceStartRequest {
                client_name: client_name.to_owned(),
            },
        )
        .await
    }

    // ── your account ──

    /// Your personal data, as the JSON document `GET /me/export` streams.
    pub async fn export(&self) -> Result<Vec<u8>> {
        // Transcripts can be large: allow far longer than a normal request.
        let http = HttpClient::new(std::time::Duration::from_secs(600));
        let url = self.url("/me/export");
        self.confirming(|| async {
            let (status, _, bytes) = http
                .send(Method::GET, &url, self.token(), None, None)
                .await?;
            if !status.is_success() {
                if let Ok(e) = serde_json::from_slice::<ApiError>(&bytes) {
                    return Err(Error::Api(e));
                }
                return Err(Error::Status {
                    status: status.as_u16(),
                    body: String::from_utf8_lossy(&bytes).chars().take(500).collect(),
                });
            }
            Ok(bytes.to_vec())
        })
        .await
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

    // ── two-factor sign-in ──

    /// Your two-factor sign-in, and what the instance requires.
    pub async fn two_factor(&self) -> Result<TwoFactor> {
        self.get("/me/two-factor").await
    }

    pub async fn update_two_factor(&self, req: &UpdateTwoFactorRequest) -> Result<Me> {
        self.patch("/me/two-factor", req).await
    }

    /// A new authenticator app's secret, link and QR code, to confirm with
    /// [`Api::confirm_authenticator`]. Replacing the app you have takes
    /// `code`, from it or a recovery code.
    pub async fn start_authenticator(&self, code: Option<&str>) -> Result<AuthenticatorSetup> {
        self.post(
            "/me/two-factor/authenticator",
            &StartAuthenticatorRequest {
                code: code.map(str::to_owned),
            },
        )
        .await
    }

    pub async fn confirm_authenticator(
        &self,
        req: &ConfirmAuthenticatorRequest,
    ) -> Result<AuthenticatorAdded> {
        self.post("/me/two-factor/authenticator/confirm", req).await
    }

    /// Turns two-factor sign-in off; `code` is a current code.
    pub async fn remove_authenticator(&self, code: &str) -> Result<Me> {
        self.post(
            "/me/two-factor/authenticator/remove",
            &TwoFactorCodeRequest {
                code: code.to_owned(),
            },
        )
        .await
    }

    /// New recovery codes in place of the old ones; `code` is a current code.
    pub async fn new_recovery_codes(&self, code: &str) -> Result<RecoveryCodes> {
        self.post(
            "/me/two-factor/recovery-codes",
            &TwoFactorCodeRequest {
                code: code.to_owned(),
            },
        )
        .await
    }

    // ── passkeys ──
    //
    // Making a passkey and signing in with one take an authenticator's
    // answer (`navigator.credentials` in a browser); these carry it.

    /// What an authenticator needs to make a passkey for you.
    pub async fn passkey_options(
        &self,
        req: &PasskeyOptionsRequest,
    ) -> Result<PasskeyCreationOptions> {
        self.post("/me/passkeys/options", req).await
    }

    /// Keeps the passkey an authenticator made with [`Api::passkey_options`].
    pub async fn add_passkey(&self, req: &AddPasskeyRequest) -> Result<Me> {
        self.post("/me/passkeys", req).await
    }

    /// What an authenticator needs to sign in with a passkey.
    pub async fn passkey_sign_in_options(&self) -> Result<PasskeyRequestOptions> {
        self.post("/auth/passkey/options", &serde_json::json!({}))
            .await
    }

    /// Signs in with a passkey's answer; the session comes as a cookie.
    pub async fn passkey_sign_in(&self, req: &PasskeySignInRequest) -> Result<AuthResponse> {
        self.post("/auth/passkey", req).await
    }

    /// Renames one of your passkeys.
    pub async fn rename_passkey(&self, id: &str, name: &str) -> Result<Me> {
        self.patch(
            &format!("/me/passkeys/{}", enc(id)),
            &UpdatePasskeyRequest {
                name: name.to_owned(),
            },
        )
        .await
    }

    /// Removes one of your passkeys.
    pub async fn remove_passkey(&self, id: &str) -> Result<Me> {
        self.delete(&format!("/me/passkeys/{}", enc(id))).await
    }

    // ── notifications ──

    /// What notifies you, and the browsers that show your notifications.
    pub async fn notifications(&self) -> Result<Notifications> {
        self.get("/me/notifications").await
    }

    pub async fn update_notifications(
        &self,
        req: &UpdateNotificationsRequest,
    ) -> Result<Notifications> {
        self.patch("/me/notifications", req).await
    }

    /// Turns notifications off for one of your browsers.
    pub async fn remove_push_device(&self, id: &str) -> Result<Notifications> {
        self.delete(&format!("/me/notifications/devices/{}", enc(id)))
            .await
    }

    /// Sends a test notification to every browser of yours.
    pub async fn test_notification(&self) -> Result<PushTestResult> {
        self.post("/me/notifications/test", &serde_json::json!({}))
            .await
    }

    pub async fn tokens(&self) -> Result<TokenList> {
        self.get("/auth/tokens").await
    }

    /// A personal access token, limited to what `req` asks.
    pub async fn create_token(&self, req: &CreateTokenRequest) -> Result<CreatedToken> {
        self.post("/auth/tokens", req).await
    }

    /// Revokes a token: signs a browser or the CLI out, or ends an access
    /// token.
    pub async fn delete_token(&self, id: &str) -> Result<Ok> {
        self.delete(&format!("/auth/tokens/{}", enc(id))).await
    }

    // ── projects ──

    pub async fn projects(&self) -> Result<ProjectList> {
        self.get("/projects").await
    }

    /// The projects of one workspace (id or slug).
    pub async fn workspace_projects(&self, workspace: &str) -> Result<ProjectList> {
        self.get(&format!("/projects?workspace={}", enc(workspace)))
            .await
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

    /// The repositories a workspace's projects may use (`None`: your only
    /// workspace).
    pub async fn github_repos(&self, workspace: Option<&str>) -> Result<GithubRepoList> {
        match workspace {
            Some(w) => {
                self.get(&format!("/github/repos?workspace={}", enc(w)))
                    .await
            }
            None => self.get("/github/repos").await,
        }
    }

    /// The repositories a project's agents may use, the primary one first.
    pub async fn project_repos(&self, project: &str) -> Result<ProjectRepoList> {
        self.get(&format!("/projects/{}/repos", enc(project))).await
    }

    /// Lets a project's agents use a repository, or changes their access.
    pub async fn add_project_repo(
        &self,
        project: &str,
        req: &AddProjectRepoRequest,
    ) -> Result<ProjectRepoList> {
        self.post(&format!("/projects/{}/repos", enc(project)), req)
            .await
    }

    /// Removes a repository from a project; `name` is `None` for an
    /// owner-wide grant.
    pub async fn remove_project_repo(
        &self,
        project: &str,
        owner: &str,
        name: Option<&str>,
    ) -> Result<ProjectRepoList> {
        self.delete(&format!(
            "/projects/{}/repos/{}/{}",
            enc(project),
            enc(owner),
            enc(name.unwrap_or("*"))
        ))
        .await
    }

    /// A project's members and their roles.
    pub async fn project_members(&self, project: &str) -> Result<MemberList> {
        self.get(&format!("/projects/{}/members", enc(project)))
            .await
    }

    /// Adds someone in the project's workspace, by the address they are in
    /// it as, or changes the role of a member (maintainers).
    pub async fn add_project_member(
        &self,
        project: &str,
        req: &AddMemberRequest,
    ) -> Result<MemberList> {
        self.post(&format!("/projects/{}/members", enc(project)), req)
            .await
    }

    /// Changes a member's role (maintainers; owners for an owner's).
    pub async fn set_project_role(
        &self,
        project: &str,
        user_id: &str,
        role: fctl_proto::session::ProjectRole,
    ) -> Result<MemberList> {
        self.patch(
            &format!("/projects/{}/members/{}", enc(project), enc(user_id)),
            &SetRoleRequest { role },
        )
        .await
    }

    /// Removes someone from a project, or leaves it (your own id): the
    /// members left, or `{"ok": true}` when you left.
    pub async fn remove_project_member(
        &self,
        project: &str,
        user_id: &str,
    ) -> Result<serde_json::Value> {
        self.delete(&format!(
            "/projects/{}/members/{}",
            enc(project),
            enc(user_id)
        ))
        .await
    }

    /// The names of a project's secrets, and who set each when; never
    /// their values (maintainers).
    pub async fn secrets(&self, project: &str) -> Result<SecretList> {
        self.get(&format!("/projects/{}/secrets", enc(project)))
            .await
    }

    /// Sets a secret, which the project's sandboxes get as an environment
    /// variable (maintainers).
    pub async fn set_secret(&self, project: &str, name: &str, value: &str) -> Result<SecretList> {
        self.put(
            &format!("/projects/{}/secrets/{}", enc(project), enc(name)),
            &SetSecretRequest {
                value: value.to_owned(),
            },
        )
        .await
    }

    /// Deletes a secret (maintainers).
    pub async fn delete_secret(&self, project: &str, name: &str) -> Result<SecretList> {
        self.delete(&format!("/projects/{}/secrets/{}", enc(project), enc(name)))
            .await
    }

    // ── sessions ──

    pub async fn sessions(
        &self,
        project: Option<&str>,
        status: Option<&str>,
        limit: u32,
    ) -> Result<SessionList> {
        self.sessions_in(None, project, status, false, limit).await
    }

    /// Sessions, of one workspace when it is given, with the archived ones
    /// among them when `archived` says so.
    pub async fn sessions_in(
        &self,
        workspace: Option<&str>,
        project: Option<&str>,
        status: Option<&str>,
        archived: bool,
        limit: u32,
    ) -> Result<SessionList> {
        let mut q = format!("/sessions?limit={limit}");
        if let Some(w) = workspace {
            q.push_str(&format!("&workspace={}", enc(w)));
        }
        if let Some(p) = project {
            q.push_str(&format!("&project={}", enc(p)));
        }
        if let Some(s) = status {
            q.push_str(&format!("&status={}", enc(s)));
        }
        if archived {
            q.push_str("&archived=1");
        }
        self.get(&q).await
    }

    /// A project's sessions with its review agents' and its archived ones
    /// among them, which the other lists leave out: to find one by its
    /// number.
    pub async fn sessions_with_reviews(&self, project: &str, limit: u32) -> Result<SessionList> {
        self.get(&format!(
            "/sessions?limit={limit}&project={}&reviews=1&archived=1",
            enc(project)
        ))
        .await
    }

    pub async fn session(&self, id: &str) -> Result<Session> {
        self.get(&format!("/sessions/{}", enc(id))).await
    }

    /// The sessions whose title, description, people's messages or agent's
    /// replies contain `q`, of one workspace or project when it is given.
    pub async fn search(
        &self,
        q: &str,
        workspace: Option<&str>,
        project: Option<&str>,
        limit: u32,
    ) -> Result<SearchResults> {
        let mut path = format!("/search?q={}&limit={limit}", enc(q));
        if let Some(w) = workspace {
            path.push_str(&format!("&workspace={}", enc(w)));
        }
        if let Some(p) = project {
            path.push_str(&format!("&project={}", enc(p)));
        }
        self.get(&path).await
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

    /// An image a session's agent read, which the session keeps for a week
    /// (`kept` in the image's `source`, in the transcript).
    pub async fn kept_image(&self, session: &str, image: &str) -> Result<Vec<u8>> {
        let path = format!("/sessions/{}/images/{}", enc(session), enc(image));
        Ok(self.raw(Method::GET, &path, None, None).await?.to_vec())
    }

    // ── published screenshots ──

    /// Publishes a screenshot for a session this runner runs (with the
    /// runner token): an image anyone with the link can open until it
    /// expires.
    pub async fn runner_publish(
        &self,
        session: &str,
        name: &str,
        bytes: Vec<u8>,
    ) -> Result<PublishedFile> {
        let path = format!(
            "/runner/sessions/{}/published?name={}",
            enc(session),
            enc(name)
        );
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

    /// Deletes a published screenshot before it expires, by its token (for
    /// people who manage its session).
    pub async fn unpublish(&self, token: &str) -> Result<Ok> {
        self.delete(&format!("/published/{}", enc(token))).await
    }

    // ── project memory and retrospectives ──

    /// A project's memory: its current notes, newest first, then the stale
    /// ones.
    pub async fn memories(&self, project: &str) -> Result<MemoryList> {
        self.get(&format!("/projects/{}/memories", enc(project)))
            .await
    }

    /// One note of a project's memory.
    pub async fn memory(&self, id: &str) -> Result<Memory> {
        self.get(&format!("/memories/{}", enc(id))).await
    }

    /// A note from you for a project's agents (editors).
    pub async fn add_memory(&self, project: &str, req: &AddMemoryRequest) -> Result<Memory> {
        self.post(&format!("/projects/{}/memories", enc(project)), req)
            .await
    }

    /// Changes a note's text, or whether it is stale (editors).
    pub async fn update_memory(&self, id: &str, req: &UpdateMemoryRequest) -> Result<Memory> {
        self.patch(&format!("/memories/{}", enc(id)), req).await
    }

    /// Deletes a note (editors).
    pub async fn delete_memory(&self, id: &str) -> Result<Ok> {
        self.delete(&format!("/memories/{}", enc(id))).await
    }

    /// A project's retrospectives, newest first: of one outcome when it is
    /// given, after the one `before` names for the next page.
    pub async fn project_retrospectives(
        &self,
        project: &str,
        outcome: Option<&str>,
        before: Option<&str>,
        limit: u32,
    ) -> Result<RetrospectiveList> {
        let mut q = format!("/projects/{}/retrospectives?limit={limit}", enc(project));
        if let Some(o) = outcome {
            q.push_str(&format!("&outcome={}", enc(o)));
        }
        if let Some(b) = before {
            q.push_str(&format!("&before={}", enc(b)));
        }
        self.get(&q).await
    }

    /// A session's retrospectives, newest first.
    pub async fn session_retrospectives(&self, session: &str) -> Result<RetrospectiveList> {
        self.get(&format!("/sessions/{}/retrospectives", enc(session)))
            .await
    }

    /// Deletes a retrospective (for people who manage its session).
    pub async fn delete_retrospective(&self, id: &str) -> Result<Ok> {
        self.delete(&format!("/retrospectives/{}", enc(id))).await
    }

    // ── review agents ──

    /// A project's review agents: on or off, on whose Claude subscription,
    /// and how they review.
    pub async fn review_agents(&self, project: &str) -> Result<ReviewAgents> {
        self.get(&format!("/projects/{}/review-agents", enc(project)))
            .await
    }

    /// Turns a project's review agents on or off, takes them over or
    /// changes how they review (owners; the person they run for may turn
    /// them off).
    pub async fn update_review_agents(
        &self,
        project: &str,
        req: &UpdateReviewAgentsRequest,
    ) -> Result<ReviewAgents> {
        self.patch(&format!("/projects/{}/review-agents", enc(project)), req)
            .await
    }

    /// The apps that opened the newest pull requests in a project's
    /// repositories, those that opened most first (its owners).
    pub async fn review_bots(&self, project: &str) -> Result<ReviewBotList> {
        self.get(&format!("/projects/{}/review-agents/bots", enc(project)))
            .await
    }

    /// An app as GitHub shows it, by its name (`renovate`, `renovate[bot]`
    /// or its page), through a project's installation (its owners).
    pub async fn review_bot(&self, project: &str, name: &str) -> Result<ReviewBot> {
        self.get(&format!(
            "/projects/{}/review-agents/bots/{}",
            enc(project),
            enc(name)
        ))
        .await
    }

    /// A project's reviews, newest first, before `before` for the next page.
    pub async fn reviews(
        &self,
        project: &str,
        before: Option<i64>,
        limit: u32,
    ) -> Result<ReviewList> {
        let mut q = format!("/projects/{}/reviews?limit={limit}", enc(project));
        if let Some(b) = before {
            q.push_str(&format!("&before={b}"));
        }
        self.get(&q).await
    }

    /// The review of a review agent's session; `None` for other sessions.
    pub async fn get_review(&self, project: &str, session: &str) -> Result<Option<Review>> {
        let list: ReviewList = self
            .get(&format!(
                "/projects/{}/reviews?session={}",
                enc(project),
                enc(session)
            ))
            .await?;
        Ok(list.reviews.into_iter().next())
    }

    /// Asks for a review of a pull request's head now.
    pub async fn start_review(&self, project: &str, req: &StartReviewRequest) -> Result<Review> {
        self.post(&format!("/projects/{}/reviews", enc(project)), req)
            .await
    }

    // ── schedules ──

    /// A project's schedules, in the order they were made.
    pub async fn schedules(&self, project: &str) -> Result<ScheduleList> {
        self.get(&format!("/projects/{}/schedules", enc(project)))
            .await
    }

    /// Sets up a schedule in a project, on the caller's Claude subscription.
    pub async fn create_schedule(
        &self,
        project: &str,
        req: &CreateScheduleRequest,
    ) -> Result<Schedule> {
        self.post(&format!("/projects/{}/schedules", enc(project)), req)
            .await
    }

    /// A schedule, by id.
    pub async fn get_schedule(&self, id: &str) -> Result<Schedule> {
        self.get(&format!("/schedules/{}", enc(id))).await
    }

    /// Changes, pauses, resumes or takes over a schedule.
    pub async fn update_schedule(&self, id: &str, req: &UpdateScheduleRequest) -> Result<Schedule> {
        self.patch(&format!("/schedules/{}", enc(id)), req).await
    }

    /// Deletes a schedule; the sessions it started stay.
    pub async fn delete_schedule(&self, id: &str) -> Result<Ok> {
        self.delete(&format!("/schedules/{}", enc(id))).await
    }

    /// A schedule's runs, newest first, before `before` for the next page.
    pub async fn schedule_runs(
        &self,
        id: &str,
        before: Option<i64>,
        limit: u32,
    ) -> Result<ScheduleRunList> {
        let mut q = format!("/schedules/{}/runs?limit={limit}", enc(id));
        if let Some(b) = before {
            q.push_str(&format!("&before={b}"));
        }
        self.get(&q).await
    }

    /// Runs a schedule's task now, in a session on the caller's Claude
    /// subscription.
    pub async fn run_schedule(&self, id: &str, req: &RunScheduleRequest) -> Result<ScheduleRun> {
        self.post(&format!("/schedules/{}/runs", enc(id)), req)
            .await
    }

    /// What an agent asks about its session's project (with the runner
    /// token): its memory, or a retrospective.
    pub async fn runner_call(&self, session: &str, call: &AgentCall) -> Result<AgentAnswer> {
        self.post(&format!("/runner/sessions/{}/calls", enc(session)), call)
            .await
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

    /// Passes on to a session's agent the findings of `review` (its
    /// session) that fctl held back from it (`ReviewCheck::held_after`):
    /// the agent gets them, and those of the reviews after them, up to
    /// `fctl_domain::reviews::MAX_PASSED_ON` in a row.
    pub async fn pass_on_review(&self, session: &str, review: &str) -> Result<SendMessageResponse> {
        self.post(
            &format!("/sessions/{}/reviews/{}/pass-on", enc(session), enc(review)),
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

    /// Follows a session: its notifications reach you.
    pub async fn follow(&self, session: &str) -> Result<Session> {
        self.put(
            &format!("/sessions/{}/follow", enc(session)),
            &serde_json::json!({}),
        )
        .await
    }

    pub async fn unfollow(&self, session: &str) -> Result<Session> {
        self.delete(&format!("/sessions/{}/follow", enc(session)))
            .await
    }

    /// The people a session is shared with, besides its project's.
    pub async fn shares(&self, session: &str) -> Result<ShareList> {
        self.get(&format!("/sessions/{}/shares", enc(session)))
            .await
    }

    /// Shares a session with someone in its workspace, by the address they
    /// are in it as, or changes their role in it (the session's owners).
    pub async fn share(&self, session: &str, req: &CreateShareRequest) -> Result<ShareList> {
        self.post(&format!("/sessions/{}/shares", enc(session)), req)
            .await
    }

    /// Stops sharing a session with someone (the session's owners).
    pub async fn unshare(&self, session: &str, user_id: &str) -> Result<ShareList> {
        self.delete(&format!(
            "/sessions/{}/shares/{}",
            enc(session),
            enc(user_id)
        ))
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

    /// Registers a runner for a workspace you are in (`None`: the one you
    /// are an admin of, else the one you are a member of). With
    /// `personal`, it runs only your sessions there; absent, only yours
    /// when you are a member, everyone's when you are an admin.
    pub async fn create_runner(
        &self,
        name: &str,
        workspace: Option<&str>,
        personal: Option<bool>,
    ) -> Result<RunnerRegistration> {
        self.post(
            "/runners",
            &CreateRunnerRequest {
                name: name.to_owned(),
                workspace: workspace.map(str::to_owned),
                personal,
            },
        )
        .await
    }

    pub async fn delete_runner(&self, id: &str) -> Result<Ok> {
        self.delete(&format!("/runners/{}", enc(id))).await
    }

    pub async fn update_runner(&self, id: &str, req: &UpdateRunnerRequest) -> Result<Runner> {
        self.patch(&format!("/runners/{}", enc(id)), req).await
    }

    /// The runner whose token this client has, and the sessions it runs.
    pub async fn this_runner(&self) -> Result<RunnerDetail> {
        self.get("/runner").await
    }

    /// Drains the runner whose token this client has, or stops draining it.
    /// `stop_sessions` also stops the sessions it runs (uninstalling).
    pub async fn drain_this_runner(
        &self,
        draining: bool,
        stop_sessions: bool,
    ) -> Result<RunnerDetail> {
        self.post(
            "/runner/drain",
            &DrainRequest {
                draining,
                stop_sessions: stop_sessions.then_some(true),
            },
        )
        .await
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

    /// Connects the Claude token you keep on your runners (`fctl host
    /// claude set-token`), which you never give to fctl, and makes it your
    /// default.
    pub async fn add_claude_on_runners(&self, label: Option<String>) -> Result<Me> {
        self.post(
            "/me/claude/on-runners",
            &AddClaudeOnRunnersRequest { label },
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

    /// The slugs instance admins reserve: no new workspace or project
    /// takes one (admins).
    pub async fn reserved_slugs(&self) -> Result<ReservedSlugList> {
        self.get("/admin/reserved-slugs").await
    }

    /// Reserves a slug (admins): a conflict when it is reserved already.
    pub async fn reserve_slug(&self, slug: &str) -> Result<ReservedSlug> {
        self.post(
            "/admin/reserved-slugs",
            &ReserveSlugRequest {
                slug: slug.to_owned(),
            },
        )
        .await
    }

    /// Lets new workspaces and projects take a slug again (admins).
    pub async fn unreserve_slug(&self, slug: &str) -> Result<Ok> {
        self.delete(&format!("/admin/reserved-slugs/{}", enc(slug)))
            .await
    }

    /// Turns off a user's two-factor sign-in (they lost their phone).
    /// Changes someone's instance role or status (admins).
    pub async fn update_user(&self, user_id: &str, req: &UpdateUserRequest) -> Result<User> {
        self.patch(&format!("/admin/users/{}", enc(user_id)), req)
            .await
    }

    /// Deletes someone's account (admins), as deleting their own would.
    pub async fn delete_user(&self, user_id: &str) -> Result<Ok> {
        self.delete(&format!("/admin/users/{}", enc(user_id))).await
    }

    pub async fn reset_two_factor(&self, user_id: &str) -> Result<User> {
        self.delete(&format!("/admin/users/{}/two-factor", enc(user_id)))
            .await
    }

    pub async fn instance_settings(&self) -> Result<InstanceSettings> {
        self.get("/admin/settings").await
    }

    /// Invites an address to the instance, and to workspaces you are an
    /// admin of.
    pub async fn invite(&self, req: &CreateInviteRequest) -> Result<Invite> {
        self.post("/admin/invites", req).await
    }

    /// Who asked for early access and has no account yet (admins).
    pub async fn access_requests(&self) -> Result<AccessRequestList> {
        self.get("/admin/access-requests").await
    }

    /// Removes an early access request (admins); asking again adds it back.
    pub async fn remove_access_request(&self, id: &str) -> Result<Ok> {
        self.delete(&format!("/admin/access-requests/{}", enc(id)))
            .await
    }

    /// Hands the role of primary instance owner to an admin.
    pub async fn transfer_instance(&self, user_id: &str) -> Result<User> {
        self.post(
            "/admin/transfer",
            &TransferRequest {
                user_id: fctl_proto::UserId(user_id.to_owned()),
            },
        )
        .await
    }

    /// A page of the instance's audit log (what happened in workspaces is
    /// in theirs), newest first.
    pub async fn admin_audit(&self, before: Option<i64>) -> Result<AuditList> {
        let mut path = "/admin/audit".to_owned();
        if let Some(b) = before {
            path.push_str(&format!("?before={b}"));
        }
        self.get(&path).await
    }

    /// Every workspace on the instance.
    pub async fn admin_workspaces(&self) -> Result<AdminWorkspaceList> {
        self.get("/admin/workspaces").await
    }

    /// Gives the primary admin role of a workspace whose primary admin's
    /// account is disabled or gone to one of its members.
    pub async fn admin_workspace_primary(&self, workspace: &str, user_id: &str) -> Result<Ok> {
        self.post(
            &format!("/admin/workspaces/{}/primary", enc(workspace)),
            &TransferRequest {
                user_id: fctl_proto::UserId(user_id.to_owned()),
            },
        )
        .await
    }

    /// Deletes a workspace and everything in it.
    pub async fn admin_delete_workspace(&self, workspace: &str, name: &str) -> Result<Ok> {
        self.post(
            &format!("/admin/workspaces/{}/delete", enc(workspace)),
            &DeleteWorkspaceRequest {
                name: name.to_owned(),
            },
        )
        .await
    }

    /// The GitHub App's installations and the workspaces using them.
    pub async fn admin_github(&self) -> Result<GithubInstallations> {
        self.get("/admin/github").await
    }

    /// Takes a GitHub account from a workspace.
    pub async fn admin_disconnect_github(
        &self,
        account: &str,
        workspace: &str,
    ) -> Result<GithubInstallations> {
        self.delete(&format!(
            "/admin/github/{}/workspaces/{}",
            enc(account),
            enc(workspace)
        ))
        .await
    }

    // ── workspaces ──

    /// Your workspaces, and the invitations waiting for you.
    pub async fn workspaces(&self) -> Result<WorkspaceList> {
        self.get("/workspaces").await
    }

    pub async fn workspace(&self, id_or_slug: &str) -> Result<Workspace> {
        self.get(&format!("/workspaces/{}", enc(id_or_slug))).await
    }

    pub async fn create_workspace(&self, req: &CreateWorkspaceRequest) -> Result<Workspace> {
        self.post("/workspaces", req).await
    }

    pub async fn update_workspace(
        &self,
        id: &str,
        req: &UpdateWorkspaceRequest,
    ) -> Result<Workspace> {
        self.patch(&format!("/workspaces/{}", enc(id)), req).await
    }

    /// Deletes a workspace (its primary admin), its name typed to confirm.
    pub async fn delete_workspace(&self, id: &str, name: &str) -> Result<Ok> {
        self.post(
            &format!("/workspaces/{}/delete", enc(id)),
            &DeleteWorkspaceRequest {
                name: name.to_owned(),
            },
        )
        .await
    }

    /// Uploads a workspace's avatar image.
    pub async fn set_workspace_avatar(&self, id: &str, bytes: Vec<u8>) -> Result<Workspace> {
        let body = self
            .raw(
                Method::PUT,
                &format!("/workspaces/{}/avatar", enc(id)),
                Some(bytes),
                Some("application/octet-stream"),
            )
            .await?;
        serde_json::from_slice(&body).map_err(|e| Error::Decode(e.to_string()))
    }

    pub async fn remove_workspace_avatar(&self, id: &str) -> Result<Workspace> {
        self.delete(&format!("/workspaces/{}/avatar", enc(id)))
            .await
    }

    pub async fn workspace_members(&self, id: &str) -> Result<WorkspaceMemberList> {
        self.get(&format!("/workspaces/{}/members", enc(id))).await
    }

    pub async fn invite_to_workspace(
        &self,
        id: &str,
        req: &InviteToWorkspaceRequest,
    ) -> Result<WorkspaceInvitation> {
        self.post(&format!("/workspaces/{}/invitations", enc(id)), req)
            .await
    }

    pub async fn revoke_workspace_invitation(
        &self,
        id: &str,
        invitation: &str,
    ) -> Result<WorkspaceMemberList> {
        self.delete(&format!(
            "/workspaces/{}/invitations/{}",
            enc(id),
            enc(invitation)
        ))
        .await
    }

    pub async fn set_workspace_role(
        &self,
        id: &str,
        user_id: &str,
        role: fctl_proto::session::WorkspaceRole,
    ) -> Result<WorkspaceMemberList> {
        self.patch(
            &format!("/workspaces/{}/members/{}", enc(id), enc(user_id)),
            &UpdateWorkspaceMemberRequest { role },
        )
        .await
    }

    /// Removes someone from a workspace, or leaves it (your own id).
    pub async fn remove_workspace_member(
        &self,
        id: &str,
        user_id: &str,
    ) -> Result<serde_json::Value> {
        self.delete(&format!("/workspaces/{}/members/{}", enc(id), enc(user_id)))
            .await
    }

    /// Hands the primary admin role of a workspace to another admin.
    pub async fn transfer_workspace(&self, id: &str, user_id: &str) -> Result<WorkspaceMemberList> {
        self.post(
            &format!("/workspaces/{}/transfer", enc(id)),
            &TransferRequest {
                user_id: fctl_proto::UserId(user_id.to_owned()),
            },
        )
        .await
    }

    /// Your preferences in a workspace.
    pub async fn update_membership(
        &self,
        id: &str,
        req: &UpdateMembershipRequest,
    ) -> Result<Workspace> {
        self.patch(&format!("/workspaces/{}/membership", enc(id)), req)
            .await
    }

    /// Puts your workspaces in order (ids or slugs): the ones named first,
    /// the others after them as they were.
    pub async fn order_workspaces(&self, workspaces: &[String]) -> Result<WorkspaceList> {
        self.put(
            "/me/workspace-order",
            &WorkspaceOrderRequest {
                workspaces: workspaces.to_vec(),
            },
        )
        .await
    }

    pub async fn workspace_audit(&self, id: &str, before: Option<i64>) -> Result<AuditList> {
        let mut path = format!("/workspaces/{}/audit", enc(id));
        if let Some(b) = before {
            path.push_str(&format!("?before={b}"));
        }
        self.get(&path).await
    }

    pub async fn workspace_runners(&self, id: &str) -> Result<RunnerList> {
        self.get(&format!("/workspaces/{}/runners", enc(id))).await
    }

    /// Lets one of your runners run a workspace's sessions.
    /// Lets one of your runners run a workspace's sessions: with
    /// `personal`, only yours. Absent: only yours when you are a member,
    /// everyone's when you are an admin.
    pub async fn add_workspace_runner(
        &self,
        id: &str,
        runner: &str,
        personal: Option<bool>,
    ) -> Result<RunnerList> {
        self.put(
            &format!("/workspaces/{}/runners/{}", enc(id), enc(runner)),
            &AddRunnerRequest { personal },
        )
        .await
    }

    pub async fn remove_workspace_runner(&self, id: &str, runner: &str) -> Result<RunnerList> {
        self.delete(&format!("/workspaces/{}/runners/{}", enc(id), enc(runner)))
            .await
    }

    pub async fn workspace_github(&self, id: &str) -> Result<WorkspaceGithub> {
        self.get(&format!("/workspaces/{}/github", enc(id))).await
    }

    /// Where the app review agents post their reviews as is installed among
    /// the GitHub accounts a workspace works in.
    pub async fn workspace_review_app(&self, id: &str) -> Result<WorkspaceReviewApp> {
        self.get(&format!("/workspaces/{}/github/review-app", enc(id)))
            .await
    }

    /// The link to the GitHub sign-in that connects an account the app is
    /// installed on to a workspace, with what GitHub says you may.
    pub async fn connect_github(
        &self,
        id: &str,
        req: &ConnectGithubRequest,
    ) -> Result<ConnectGithubLink> {
        self.post(&format!("/workspaces/{}/github/connect", enc(id)), req)
            .await
    }

    pub async fn disconnect_github(&self, id: &str, account: &str) -> Result<WorkspaceGithub> {
        self.delete(&format!("/workspaces/{}/github/{}", enc(id), enc(account)))
            .await
    }

    /// The Linear workspaces a workspace works in.
    pub async fn workspace_linear(&self, id: &str) -> Result<WorkspaceLinear> {
        self.get(&format!("/workspaces/{}/linear", enc(id))).await
    }

    /// The link to the Linear sign-in that installs fctl's app in a Linear
    /// workspace and connects it (the workspace's admins, who are its
    /// admins too).
    pub async fn connect_linear(&self, id: &str, req: &ConnectLinearRequest) -> Result<LinearLink> {
        self.post(&format!("/workspaces/{}/linear/connect", enc(id)), req)
            .await
    }

    pub async fn disconnect_linear(&self, id: &str, organization: &str) -> Result<WorkspaceLinear> {
        self.delete(&format!(
            "/workspaces/{}/linear/{}",
            enc(id),
            enc(organization)
        ))
        .await
    }

    /// The teams fctl's app reaches in a Linear workspace a workspace works
    /// in, with their workflows and projects.
    pub async fn linear_teams(&self, id: &str, organization: &str) -> Result<LinearTeamList> {
        self.get(&format!(
            "/workspaces/{}/linear/{}/teams",
            enc(id),
            enc(organization)
        ))
        .await
    }

    /// Your Linear accounts, which fctl knows you by.
    pub async fn my_linear(&self) -> Result<MyLinear> {
        self.get("/me/linear").await
    }

    /// Forgets your Linear account in a Linear workspace.
    pub async fn unlink_linear(&self, organization: &str) -> Result<MyLinear> {
        self.delete(&format!("/me/linear/{}", enc(organization)))
            .await
    }

    /// Where your browser links your Linear account (signed in to fctl).
    pub fn link_linear_url(&self) -> String {
        self.url("/linear/link")
    }

    /// A project's ticket tracker as it works now.
    pub async fn project_tickets(&self, project: &str) -> Result<ProjectTickets> {
        self.get(&format!("/projects/{}/tickets", enc(project)))
            .await
    }

    /// Connects what you choose of an account your GitHub sign-in offered
    /// you (`WorkspaceGithub::offers`).
    pub async fn choose_github_repos(
        &self,
        id: &str,
        account: &str,
        req: &ChooseGithubRepos,
    ) -> Result<WorkspaceGithub> {
        self.post(
            &format!("/workspaces/{}/github/offers/{}", enc(id), enc(account)),
            req,
        )
        .await
    }

    /// Connects nothing of what your GitHub sign-in offered you of an account.
    pub async fn dismiss_github_offer(&self, id: &str, account: &str) -> Result<WorkspaceGithub> {
        self.delete(&format!(
            "/workspaces/{}/github/offers/{}",
            enc(id),
            enc(account)
        ))
        .await
    }

    pub async fn accept_invitation(&self, id: &str) -> Result<Workspace> {
        self.post(
            &format!("/invitations/{}/accept", enc(id)),
            &serde_json::json!({}),
        )
        .await
    }

    pub async fn decline_invitation(&self, id: &str) -> Result<Ok> {
        self.post(
            &format!("/invitations/{}/decline", enc(id)),
            &serde_json::json!({}),
        )
        .await
    }

    // ── email addresses ──

    pub async fn update_email(&self, email: &str, req: &UpdateEmailRequest) -> Result<Me> {
        self.patch(&format!("/me/emails/{}", enc(email)), req).await
    }

    pub async fn remove_email(&self, email: &str) -> Result<Me> {
        self.delete(&format!("/me/emails/{}", enc(email))).await
    }

    /// Accepts an invitation to the instance by connecting its address to
    /// your account.
    pub async fn connect_invite(&self, token: &str) -> Result<Me> {
        self.post(
            &format!("/invites/{}/connect", enc(token)),
            &serde_json::json!({}),
        )
        .await
    }

    pub async fn update_instance_settings(
        &self,
        req: &UpdateInstanceSettingsRequest,
    ) -> Result<InstanceSettings> {
        self.patch("/admin/settings", req).await
    }
}
