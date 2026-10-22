// The REST client. Types come from the Rust protocol (src/gen, `cargo xtask gen`).
import type * as G from "@/gen";

export class ApiFailure extends Error {
  readonly status: number;
  readonly code: G.ErrorCode | "network";
  readonly field?: string;
  readonly retryAfter?: number;
  constructor(status: number, body: Partial<G.ApiError>) {
    super(body.message ?? "Something went wrong");
    this.status = status;
    this.code = (body.code as G.ErrorCode) ?? "internal";
    this.field = body.field;
    this.retryAfter = body.retry_after;
  }
}

/**
 * Asks the person to confirm it's them, with a passkey or a code, when a
 * sensitive action needs it first (`verification_required`): whether they
 * did. The app's dialog registers itself (components/two-factor/verify.tsx).
 */
export type Confirmer = () => Promise<boolean>;
let confirmer: Confirmer | undefined;

/** Registers what asks the person to confirm it's them; returns what unregisters it. */
export function setConfirmer(c: Confirmer): () => void {
  confirmer = c;
  return () => {
    if (confirmer === c) confirmer = undefined;
  };
}

/**
 * Before something that can't go again by itself, as a download the browser
 * makes: asks the person to confirm it's them when a sensitive action would
 * ask now. Whether it may go on.
 */
export async function confirmFirst(): Promise<boolean> {
  const v = await api.verification();
  if (!v.required || v.verified) return true;
  return (await confirmer?.()) ?? false;
}

async function request<T>(method: string, path: string, body?: unknown, again = true): Promise<T> {
  let res: Response;
  try {
    res = await fetch(`/api/v1${path}`, {
      method,
      credentials: "same-origin",
      headers:
        body === undefined
          ? { Accept: "application/json" }
          : { Accept: "application/json", "Content-Type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw new ApiFailure(0, { code: "unavailable", message: "You appear to be offline." });
  }
  if (res.status === 204) return undefined as T;
  const text = await res.text();
  const data = text ? JSON.parse(text) : undefined;
  if (!res.ok) {
    const failure = new ApiFailure(res.status, data ?? {});
    // The instance now requires two-factor sign-in of this account: the app
    // shows the page that sets it up (router.tsx).
    if (failure.code === "two_factor_setup_required") window.dispatchEvent(new Event(TWO_FACTOR_REQUIRED));
    // A sensitive action goes again once the person confirmed it's them.
    if (failure.code === "verification_required" && again && confirmer && (await confirmer()))
      return request<T>(method, path, body, false);
    throw failure;
  }
  return data as T;
}

/** Dispatched on `window` when a request was refused until two-factor sign-in is set up. */
export const TWO_FACTOR_REQUIRED = "fctl:two-factor-required";

const enc = encodeURIComponent;

/** Where a file uploads: a project (to start a session) or a session (a message in it). */
export type UploadScope = { project: string } | { session: string };

/**
 * Uploads a file to send with a message (XHR, for progress). The body is
 * `application/octet-stream`; the name and type travel in the query.
 */
function upload(
  scope: UploadScope,
  file: Blob,
  name: string,
  onProgress?: (fraction: number) => void,
): { promise: Promise<G.Attachment>; abort: () => void } {
  const xhr = new XMLHttpRequest();
  const q = new URLSearchParams({ name });
  if (file.type) q.set("type", file.type);
  const base = "project" in scope ? `/projects/${enc(scope.project)}` : `/sessions/${enc(scope.session)}`;
  const promise = new Promise<G.Attachment>((resolve, reject) => {
    xhr.open("POST", `/api/v1${base}/attachments?${q}`);
    xhr.setRequestHeader("Accept", "application/json");
    xhr.setRequestHeader("Content-Type", "application/octet-stream");
    xhr.upload.onprogress = (e) => {
      if (e.lengthComputable && e.total > 0) onProgress?.(e.loaded / e.total);
    };
    xhr.onload = () => {
      let data: unknown;
      try {
        data = xhr.responseText ? JSON.parse(xhr.responseText) : undefined;
      } catch {
        data = undefined;
      }
      if (xhr.status >= 200 && xhr.status < 300) resolve(data as G.Attachment);
      else reject(new ApiFailure(xhr.status, (data as Partial<G.ApiError>) ?? {}));
    };
    xhr.onerror = () =>
      reject(new ApiFailure(0, { code: "unavailable", message: "You appear to be offline." }));
    xhr.onabort = () => reject(new ApiFailure(0, { code: "unavailable", message: "Upload cancelled" }));
    xhr.send(file);
  });
  return { promise, abort: () => xhr.abort() };
}

export const api = {
  instance: () => request<G.InstanceInfo>("GET", "/instance"),
  setup: (b: G.SetupRequest) => request<G.AuthResponse>("POST", "/setup", b),
  invitePreview: (token: string) => request<G.InvitePreview>("GET", `/invites/${enc(token)}`),
  /** Asks for early access, without an account (where `features.early_access`). */
  requestAccess: (b: G.RequestAccessRequest) => request<G.AccessRequested>("POST", "/access-requests", b),
  /** Withdraws it, with the token its answer had. */
  withdrawAccess: (b: G.WithdrawAccessRequest) => request<G.Ok>("POST", "/access-requests/withdraw", b),
  signup: (b: G.SignupRequest) => request<G.AuthResponse>("POST", "/auth/signup", b),
  login: (b: G.LoginRequest) => request<G.AuthResponse>("POST", "/auth/login", b),
  logout: () => request<G.Ok>("POST", "/auth/logout", {}),
  me: () => request<G.Me>("GET", "/auth/me"),
  updateMe: (b: G.UpdateMeRequest) => request<G.Me>("PATCH", "/auth/me", b),
  changePassword: (b: G.ChangePasswordRequest) => request<G.Ok>("POST", "/auth/password", b),
  forgotPassword: (b: G.ForgotPasswordRequest) => request<G.Ok>("POST", "/auth/password/forgot", b),
  resetPassword: (b: G.ResetPasswordRequest) => request<G.AuthResponse>("POST", "/auth/password/reset", b),
  tokens: () => request<G.TokenList>("GET", "/auth/tokens"),
  createToken: (b: G.CreateTokenRequest) => request<G.CreatedToken>("POST", "/auth/tokens", b),
  deleteToken: (id: string) => request<G.Ok>("DELETE", `/auth/tokens/${enc(id)}`),
  unlinkIdentity: (id: string) => request<G.Me>("DELETE", `/auth/identities/${enc(id)}`),
  /** Whether sensitive actions ask you to confirm it's you, with what, and whether this browser has. */
  verification: () => request<G.Verification>("GET", "/auth/verify"),
  /** Confirms it's you with a code or a passkey: this browser isn't asked again for 15 minutes. */
  verify: (b: G.VerifyRequest) => request<G.Verification>("POST", "/auth/verify", b),
  /** Options to confirm it's you with one of your passkeys. */
  verifyPasskeyOptions: () => request<G.PasskeyRequestOptions>("POST", "/auth/verify/passkey", {}),
  /** The sign-in this browser waits to finish with a second factor. */
  twoFactorChallenge: () => request<G.TwoFactorChallenge>("GET", "/auth/two-factor/challenge"),
  verifyTwoFactor: (b: G.VerifyTwoFactorRequest) =>
    request<G.AuthResponse>("POST", "/auth/two-factor/verify", b),
  cancelTwoFactor: () => request<G.Ok>("POST", "/auth/two-factor/cancel", {}),
  updateTwoFactor: (b: G.UpdateTwoFactorRequest) => request<G.Me>("PATCH", "/me/two-factor", b),
  /** Replacing the app you have takes `code`, from it or a recovery code. */
  startAuthenticator: (b: G.StartAuthenticatorRequest = {}) =>
    request<G.AuthenticatorSetup>("POST", "/me/two-factor/authenticator", b),
  confirmAuthenticator: (b: G.ConfirmAuthenticatorRequest) =>
    request<G.AuthenticatorAdded>("POST", "/me/two-factor/authenticator/confirm", b),
  removeAuthenticator: (b: G.TwoFactorCodeRequest) =>
    request<G.Me>("POST", "/me/two-factor/authenticator/remove", b),
  newRecoveryCodes: (b: G.TwoFactorCodeRequest) =>
    request<G.RecoveryCodes>("POST", "/me/two-factor/recovery-codes", b),
  /** What the browser needs to make a passkey; takes the password unless this browser just signed in. */
  passkeyOptions: (b: G.PasskeyOptionsRequest = {}) =>
    request<G.PasskeyCreationOptions>("POST", "/me/passkeys/options", b),
  addPasskey: (b: G.AddPasskeyRequest) => request<G.Me>("POST", "/me/passkeys", b),
  renamePasskey: (id: string, name: string) => request<G.Me>("PATCH", `/me/passkeys/${enc(id)}`, { name }),
  removePasskey: (id: string) => request<G.Me>("DELETE", `/me/passkeys/${enc(id)}`),
  passkeySignInOptions: () => request<G.PasskeyRequestOptions>("POST", "/auth/passkey/options", {}),
  passkeySignIn: (b: G.PasskeySignInRequest) => request<G.AuthResponse>("POST", "/auth/passkey", b),
  /** Options to finish the waiting sign-in with a passkey instead of a code. */
  twoFactorPasskeyOptions: () => request<G.PasskeyRequestOptions>("POST", "/auth/two-factor/passkey", {}),
  accountDeletion: () => request<G.AccountDeletion>("GET", "/me/deletion"),
  deleteAccount: (b: G.DeleteAccountRequest) => request<G.Ok>("POST", "/me/delete", b),
  device: (code: string) => request<G.DeviceInfo>("GET", `/auth/device/${enc(code)}`),
  decideDevice: (code: string, approve: boolean) =>
    request<G.Ok>("POST", `/auth/device/${enc(code)}`, { approve }),

  projects: () => request<G.ProjectList>("GET", "/projects"),
  project: (id: string) => request<G.Project>("GET", `/projects/${enc(id)}`),
  createProject: (b: G.CreateProjectRequest) => request<G.Project>("POST", "/projects", b),
  updateProject: (id: string, b: G.UpdateProjectRequest) =>
    request<G.Project>("PATCH", `/projects/${enc(id)}`, b),
  deleteProject: (id: string) => request<G.Ok>("DELETE", `/projects/${enc(id)}`),
  members: (id: string) => request<G.MemberList>("GET", `/projects/${enc(id)}/members`),
  addMember: (id: string, b: G.AddMemberRequest) =>
    request<G.MemberList>("POST", `/projects/${enc(id)}/members`, b),
  setMemberRole: (id: string, user: string, b: G.SetRoleRequest) =>
    request<G.MemberList>("PATCH", `/projects/${enc(id)}/members/${enc(user)}`, b),
  removeMember: (id: string, user: string) =>
    request<G.MemberList>("DELETE", `/projects/${enc(id)}/members/${enc(user)}`),
  secrets: (id: string) => request<G.SecretList>("GET", `/projects/${enc(id)}/secrets`),
  setSecret: (id: string, name: string, value: string) =>
    request<G.SecretList>("PUT", `/projects/${enc(id)}/secrets/${enc(name)}`, { value }),
  deleteSecret: (id: string, name: string) =>
    request<G.SecretList>("DELETE", `/projects/${enc(id)}/secrets/${enc(name)}`),
  projectRepos: (id: string) => request<G.ProjectRepoList>("GET", `/projects/${enc(id)}/repos`),
  addProjectRepo: (id: string, b: G.AddProjectRepoRequest) =>
    request<G.ProjectRepoList>("POST", `/projects/${enc(id)}/repos`, b),
  /** `name` absent: the owner-wide grant. */
  removeProjectRepo: (id: string, owner: string, name?: string) =>
    request<G.ProjectRepoList>("DELETE", `/projects/${enc(id)}/repos/${enc(owner)}/${enc(name ?? "*")}`),
  /** A project's memory: its current notes, newest first, then the stale ones; with `session`, only its agent's. */
  memories: (project: string, q: { session?: string } = {}) =>
    request<G.MemoryList>(
      "GET",
      `/projects/${enc(project)}/memories${q.session ? `?session=${enc(q.session)}` : ""}`,
    ),
  addMemory: (project: string, b: G.AddMemoryRequest) =>
    request<G.Memory>("POST", `/projects/${enc(project)}/memories`, b),
  updateMemory: (id: string, b: G.UpdateMemoryRequest) =>
    request<G.Memory>("PATCH", `/memories/${enc(id)}`, b),
  deleteMemory: (id: string) => request<G.Ok>("DELETE", `/memories/${enc(id)}`),
  /** A project's retrospectives, newest first, with how many ended each way. */
  retrospectives: (
    project: string,
    q: { outcome?: G.RetrospectiveOutcome; before?: string; limit?: number } = {},
  ) => {
    const p = new URLSearchParams();
    if (q.outcome) p.set("outcome", q.outcome);
    if (q.before) p.set("before", q.before);
    if (q.limit) p.set("limit", String(q.limit));
    return request<G.RetrospectiveList>("GET", `/projects/${enc(project)}/retrospectives?${p}`);
  },
  sessionRetrospectives: (id: string) =>
    request<G.RetrospectiveList>("GET", `/sessions/${enc(id)}/retrospectives`),
  deleteRetrospective: (id: string) => request<G.Ok>("DELETE", `/retrospectives/${enc(id)}`),
  github: () => request<G.GithubInfo>("GET", "/github"),
  /** The repositories a workspace's projects may use. */
  githubRepos: (workspace: string) =>
    request<G.GithubRepoList>("GET", `/github/repos?workspace=${enc(workspace)}`),

  sessions: (
    q: {
      project?: string;
      workspace?: string;
      status?: string;
      limit?: number;
      before?: number;
      mine?: boolean;
    } = {},
  ) => {
    const p = new URLSearchParams();
    if (q.project) p.set("project", q.project);
    if (q.workspace) p.set("workspace", q.workspace);
    if (q.status) p.set("status", q.status);
    if (q.limit) p.set("limit", String(q.limit));
    if (q.before) p.set("before", String(q.before));
    if (q.mine) p.set("mine", "1");
    return request<G.SessionList>("GET", `/sessions?${p}`);
  },
  session: (id: string) => request<G.Session>("GET", `/sessions/${enc(id)}`),
  createSession: (project: string, b: G.CreateSessionRequest) =>
    request<G.Session>("POST", `/projects/${enc(project)}/sessions`, b),
  updateSession: (id: string, b: G.UpdateSessionRequest) =>
    request<G.Session>("PATCH", `/sessions/${enc(id)}`, b),
  sendMessage: (id: string, b: G.SendMessageRequest) =>
    request<G.SendMessageResponse>("POST", `/sessions/${enc(id)}/messages`, b),
  uploadAttachment: upload,
  interrupt: (id: string) => request<G.SendMessageResponse>("POST", `/sessions/${enc(id)}/interrupt`, {}),
  stop: (id: string) => request<G.Session>("POST", `/sessions/${enc(id)}/stop`, {}),
  resume: (id: string, prompt?: string, attachments?: G.AttachmentId[]) =>
    request<G.Session>("POST", `/sessions/${enc(id)}/resume`, { prompt, attachments }),
  events: (id: string, q: { before?: number; after?: number; limit?: number }) => {
    const p = new URLSearchParams();
    if (q.before) p.set("before", String(q.before));
    if (q.after) p.set("after", String(q.after));
    p.set("limit", String(q.limit ?? 300));
    return request<G.EventPage>("GET", `/sessions/${enc(id)}/events?${p}`);
  },
  /** Deletes a published screenshot before it expires, by its link's token. */
  unpublish: (token: string) => request<G.Ok>("DELETE", `/published/${enc(token)}`),
  follow: (id: string) => request<G.Session>("PUT", `/sessions/${enc(id)}/follow`, {}),
  unfollow: (id: string) => request<G.Session>("DELETE", `/sessions/${enc(id)}/follow`),
  shares: (id: string) => request<G.ShareList>("GET", `/sessions/${enc(id)}/shares`),
  share: (id: string, b: G.CreateShareRequest) =>
    request<G.ShareList>("POST", `/sessions/${enc(id)}/shares`, b),
  unshare: (id: string, user: string) =>
    request<G.ShareList>("DELETE", `/sessions/${enc(id)}/shares/${enc(user)}`),

  notifications: () => request<G.Notifications>("GET", "/me/notifications"),
  updateNotifications: (b: G.UpdateNotificationsRequest) =>
    request<G.Notifications>("PATCH", "/me/notifications", b),
  addPushDevice: (b: G.AddPushDeviceRequest) => request<G.PushDevice>("POST", "/me/notifications/devices", b),
  removePushDevice: (id: string) =>
    request<G.Notifications>("DELETE", `/me/notifications/devices/${enc(id)}`),
  testNotification: () => request<G.PushTestResult>("POST", "/me/notifications/test", {}),

  claudeToken: (b: G.AddClaudeTokenRequest) => request<G.Me>("POST", "/me/claude/token", b),
  claudeLogin: () => request<G.ClaudeLoginJob>("POST", "/me/claude/login", {}),
  claudeLoginJob: (id: string) => request<G.ClaudeLoginJob>("GET", `/me/claude/login/${enc(id)}`),
  claudeLoginCode: (id: string, code: string) =>
    request<G.ClaudeLoginJob>("POST", `/me/claude/login/${enc(id)}/code`, { code }),
  claudeDefault: (id: string) => request<G.Me>("POST", `/me/claude/${enc(id)}/default`, {}),
  claudeRemove: (id: string) => request<G.Me>("DELETE", `/me/claude/${enc(id)}`),

  runners: () => request<G.RunnerList>("GET", "/runners"),
  createRunner: (b: G.CreateRunnerRequest) => request<G.RunnerRegistration>("POST", "/runners", b),
  updateRunner: (id: string, b: G.UpdateRunnerRequest) =>
    request<G.Runner>("PATCH", `/runners/${enc(id)}`, b),
  deleteRunner: (id: string) => request<G.Ok>("DELETE", `/runners/${enc(id)}`),
  rotateRunner: (id: string) => request<G.RunnerRegistration>("POST", `/runners/${enc(id)}/token`, {}),

  users: () => request<G.UserList>("GET", "/admin/users"),
  updateUser: (id: string, b: G.UpdateUserRequest) => request<G.User>("PATCH", `/admin/users/${enc(id)}`, b),
  deleteUser: (id: string) => request<G.Ok>("DELETE", `/admin/users/${enc(id)}`),
  /** The link, or, where the instance sends email, `emailed`: then it went to them alone. */
  resetLink: (id: string) =>
    request<{ url?: string; emailed?: boolean; expires_in: number }>(
      "POST",
      `/admin/users/${enc(id)}/reset-link`,
      {},
    ),
  resetTwoFactor: (id: string) => request<G.User>("DELETE", `/admin/users/${enc(id)}/two-factor`),
  allowlist: () => request<G.AllowList>("GET", "/admin/allowlist"),
  allow: (b: G.CreateAllowRequest) => request<G.CreatedAllow>("POST", "/admin/allowlist", b),
  disallow: (id: string) =>
    request<{ ok: boolean; disabled: string[]; disconnected?: string[] }>(
      "DELETE",
      `/admin/allowlist/${enc(id)}`,
    ),
  createInvite: (b: G.CreateInviteRequest) => request<G.Invite>("POST", "/admin/invites", b),
  revokeInvite: (id: string) => request<G.Ok>("DELETE", `/admin/invites/${enc(id)}`),
  accessRequests: () => request<G.AccessRequestList>("GET", "/admin/access-requests"),
  removeAccessRequest: (id: string) => request<G.Ok>("DELETE", `/admin/access-requests/${enc(id)}`),
  transferInstance: (userId: string) => request<G.User>("POST", "/admin/transfer", { user_id: userId }),
  adminWorkspaces: () => request<G.AdminWorkspaceList>("GET", "/admin/workspaces"),
  adminDeleteWorkspace: (id: string, name: string) =>
    request<G.Ok>("POST", `/admin/workspaces/${enc(id)}/delete`, { name }),
  adminWorkspacePrimary: (id: string, userId: string) =>
    request<G.Ok>("POST", `/admin/workspaces/${enc(id)}/primary`, { user_id: userId }),
  adminGithub: () => request<G.GithubInstallations>("GET", "/admin/github"),
  adminDisconnectGithub: (account: string, workspace: string) =>
    request<G.GithubInstallations>("DELETE", `/admin/github/${enc(account)}/workspaces/${enc(workspace)}`),
  audit: (before?: number) => request<G.AuditList>("GET", `/admin/audit${before ? `?before=${before}` : ""}`),
  instanceSettings: () => request<G.InstanceSettings>("GET", "/admin/settings"),
  updateInstanceSettings: (b: G.UpdateInstanceSettingsRequest) =>
    request<G.InstanceSettings>("PATCH", "/admin/settings", b),
};

/** Workspaces: theirs, their people, runners, GitHub accounts and logs. */
export const workspaces = {
  list: () => request<G.WorkspaceList>("GET", "/workspaces"),
  get: (id: string) => request<G.Workspace>("GET", `/workspaces/${enc(id)}`),
  create: (b: G.CreateWorkspaceRequest) => request<G.Workspace>("POST", "/workspaces", b),
  update: (id: string, b: G.UpdateWorkspaceRequest) =>
    request<G.Workspace>("PATCH", `/workspaces/${enc(id)}`, b),
  remove: (id: string, name: string) => request<G.Ok>("POST", `/workspaces/${enc(id)}/delete`, { name }),
  /** Uploads its avatar image (already made square and small by the caller). */
  setAvatar: async (id: string, image: Blob): Promise<G.Workspace> => {
    const res = await fetch(`/api/v1/workspaces/${enc(id)}/avatar`, {
      method: "PUT",
      credentials: "same-origin",
      headers: { Accept: "application/json", "Content-Type": "application/octet-stream" },
      body: image,
    });
    const data = await res.json().catch(() => ({}));
    if (!res.ok) throw new ApiFailure(res.status, data);
    return data as G.Workspace;
  },
  removeAvatar: (id: string) => request<G.Workspace>("DELETE", `/workspaces/${enc(id)}/avatar`),
  members: (id: string) => request<G.WorkspaceMemberList>("GET", `/workspaces/${enc(id)}/members`),
  setRole: (id: string, user: string, role: G.WorkspaceRole) =>
    request<G.WorkspaceMemberList>("PATCH", `/workspaces/${enc(id)}/members/${enc(user)}`, { role }),
  /** Removes someone, or leaves (your own id). */
  removeMember: (id: string, user: string) =>
    request<G.WorkspaceMemberList | G.Ok>("DELETE", `/workspaces/${enc(id)}/members/${enc(user)}`),
  invite: (id: string, b: G.InviteToWorkspaceRequest) =>
    request<G.WorkspaceInvitation>("POST", `/workspaces/${enc(id)}/invitations`, b),
  revokeInvitation: (id: string, invitation: string) =>
    request<G.WorkspaceMemberList>("DELETE", `/workspaces/${enc(id)}/invitations/${enc(invitation)}`),
  transfer: (id: string, userId: string) =>
    request<G.WorkspaceMemberList>("POST", `/workspaces/${enc(id)}/transfer`, { user_id: userId }),
  updateMembership: (id: string, b: G.UpdateMembershipRequest) =>
    request<G.Workspace>("PATCH", `/workspaces/${enc(id)}/membership`, b),
  /** Puts your workspaces in order: the ones named first, the others after them as they were. */
  order: (workspaces: string[]) =>
    request<G.WorkspaceList>("PUT", "/me/workspace-order", { workspaces } satisfies G.WorkspaceOrderRequest),
  audit: (id: string, before?: number) =>
    request<G.AuditList>("GET", `/workspaces/${enc(id)}/audit${before ? `?before=${before}` : ""}`),
  runners: (id: string) => request<G.RunnerList>("GET", `/workspaces/${enc(id)}/runners`),
  // Without `personal`: only your sessions as a member, everyone's as an admin.
  createRunner: (id: string, name: string, personal?: boolean) =>
    request<G.RunnerRegistration>("POST", `/workspaces/${enc(id)}/runners`, {
      name,
      personal,
    } satisfies G.CreateRunnerRequest),
  addRunner: (id: string, runner: string, personal?: boolean) =>
    request<G.RunnerList>("PUT", `/workspaces/${enc(id)}/runners/${enc(runner)}`, {
      personal,
    } satisfies G.AddRunnerRequest),
  removeRunner: (id: string, runner: string) =>
    request<G.RunnerList>("DELETE", `/workspaces/${enc(id)}/runners/${enc(runner)}`),
  github: (id: string) => request<G.WorkspaceGithub>("GET", `/workspaces/${enc(id)}/github`),
  // The link to the GitHub sign-in that connects the account.
  connectGithub: (id: string, b: G.ConnectGithubRequest) =>
    request<G.ConnectGithubLink>("POST", `/workspaces/${enc(id)}/github/connect`, b),
  disconnectGithub: (id: string, account: string) =>
    request<G.WorkspaceGithub>("DELETE", `/workspaces/${enc(id)}/github/${enc(account)}`),
  // Connects what you chose of what your GitHub sign-in offered you of the account.
  chooseGithub: (id: string, account: string, b: G.ChooseGithubRepos) =>
    request<G.WorkspaceGithub>("POST", `/workspaces/${enc(id)}/github/offers/${enc(account)}`, b),
  dismissGithubOffer: (id: string, account: string) =>
    request<G.WorkspaceGithub>("DELETE", `/workspaces/${enc(id)}/github/offers/${enc(account)}`),
  accept: (invitation: string) => request<G.Workspace>("POST", `/invitations/${enc(invitation)}/accept`, {}),
  decline: (invitation: string) => request<G.Ok>("POST", `/invitations/${enc(invitation)}/decline`, {}),
};

/** Your email addresses. */
export const emails = {
  update: (email: string, b: G.UpdateEmailRequest) => request<G.Me>("PATCH", `/me/emails/${enc(email)}`, b),
  remove: (email: string) => request<G.Me>("DELETE", `/me/emails/${enc(email)}`),
  /** Accepts an instance invitation by connecting its address to the signed-in account. */
  connectInvite: (token: string) => request<G.Me>("POST", `/invites/${enc(token)}/connect`, {}),
};

export function wsUrl(path: string): string {
  const proto = location.protocol === "https:" ? "wss:" : "ws:";
  return `${proto}//${location.host}/api/v1${path}`;
}
