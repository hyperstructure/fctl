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

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
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
  if (!res.ok) throw new ApiFailure(res.status, data ?? {});
  return data as T;
}

const enc = encodeURIComponent;

export const api = {
  instance: () => request<G.InstanceInfo>("GET", "/instance"),
  setup: (b: G.SetupRequest) => request<G.AuthResponse>("POST", "/setup", b),
  invitePreview: (token: string) => request<G.InvitePreview>("GET", `/invites/${enc(token)}`),
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
  github: () => request<G.GithubInfo>("GET", "/github"),
  githubRepos: () => request<G.GithubRepoList>("GET", "/github/repos"),

  sessions: (
    q: { project?: string; status?: string; limit?: number; before?: number; mine?: boolean } = {},
  ) => {
    const p = new URLSearchParams();
    if (q.project) p.set("project", q.project);
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
  interrupt: (id: string) => request<G.SendMessageResponse>("POST", `/sessions/${enc(id)}/interrupt`, {}),
  stop: (id: string) => request<G.Session>("POST", `/sessions/${enc(id)}/stop`, {}),
  resume: (id: string, prompt?: string) =>
    request<G.Session>("POST", `/sessions/${enc(id)}/resume`, { prompt }),
  events: (id: string, q: { before?: number; after?: number; limit?: number }) => {
    const p = new URLSearchParams();
    if (q.before) p.set("before", String(q.before));
    if (q.after) p.set("after", String(q.after));
    p.set("limit", String(q.limit ?? 300));
    return request<G.EventPage>("GET", `/sessions/${enc(id)}/events?${p}`);
  },
  shares: (id: string) => request<G.ShareList>("GET", `/sessions/${enc(id)}/shares`),
  share: (id: string, b: G.CreateShareRequest) =>
    request<G.ShareList>("POST", `/sessions/${enc(id)}/shares`, b),
  unshare: (id: string, user: string) =>
    request<G.ShareList>("DELETE", `/sessions/${enc(id)}/shares/${enc(user)}`),

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
  resetLink: (id: string) =>
    request<{ url: string; expires_in: number }>("POST", `/admin/users/${enc(id)}/reset-link`, {}),
  allowlist: () => request<G.AllowList>("GET", "/admin/allowlist"),
  allow: (b: G.CreateAllowRequest) => request<G.CreatedAllow>("POST", "/admin/allowlist", b),
  disallow: (id: string) =>
    request<{ ok: boolean; disabled: string[] }>("DELETE", `/admin/allowlist/${enc(id)}`),
  createInvite: (b: G.CreateInviteRequest) => request<G.Invite>("POST", "/admin/invites", b),
  audit: (before?: number) => request<G.AuditList>("GET", `/admin/audit${before ? `?before=${before}` : ""}`),
};

export function wsUrl(path: string): string {
  const proto = location.protocol === "https:" ? "wss:" : "ws:";
  return `${proto}//${location.host}/api/v1${path}`;
}
