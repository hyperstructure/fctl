# Identity tokens

An agent proves to your own services and clouds that a request comes from
one of your fctl sessions with an identity token: a short-lived OpenID
Connect (OIDC) ID token that your fctl instance signs, which says whom the
session runs for and its workspace, project, runner and what it may do on
GitHub.
Your service checks it with your workspace's public keys, as clouds check
GitHub Actions' tokens, so agents need no long-lived keys of yours: an
internal API takes the token as a bearer token, AWS trades it for a role's
credentials, Google Cloud and Azure for theirs, Vault for its own.

```sh
# in a session's sandbox
curl -H "Authorization: Bearer $(fctl id-token --audience https://api.acme.dev)" https://api.acme.dev/deploys
```

A project can also connect its agents to AWS, Google Cloud and Azure
([Clouds](#clouds)): their sessions then have each cloud's credentials
without anyone handing them a secret, and the `aws`, `gcloud` and `az`
tools and every SDK sign in on their own.

## How it works

- **Each workspace is an OIDC issuer of its own**:
  `https://app.fctl.sh/oidc/wsp_…` on app.fctl.sh, your instance's address
  and `/oidc/` and the workspace's id on yours. It serves its discovery
  document and keys to anyone at
  `{issuer}/.well-known/openid-configuration` and
  `{issuer}/.well-known/jwks.json`, and every token says it in `iss`. The
  issuer's own address names it rather than holding a document: opening it
  leads to the discovery document. A cloud set up to trust one workspace's
  agents trusts no other workspace's, on the same instance or any other,
  whatever its conditions leave out.
- **Agents mint them in their sandbox** with `fctl id-token`, which asks
  fctl-agent, which asks the runner's host, which asks the control plane
  with its runner token. The control plane signs a token only for a
  session that runs on that runner now, and fills its claims from what it
  knows of the session: the agent chooses the audience, and which of
  those facts make up the subject, never their values.
- **A token lasts 10 minutes**, for one audience. Agents mint one when
  they need it; fctl-agent mints the clouds' every five minutes.
- **The instance's keys sign them** (RS256, 2048-bit RSA), made by the
  control plane and kept sealed with its master key: a new one every 30
  days, published two days before it signs and kept two days after, so
  verifiers that keep the keys for a while always have the one they meet.

A project's agents mint identity tokens unless its maintainers turn them
off (**Settings → Identity OIDC**, or `fctl projects set -p web --id-tokens
off`). Review agents never do: the pull requests they review could steer
what runs in their sandbox.

## Minting a token

In a session's sandbox, the agent runs `fctl id-token` with the audience
your service expects, and sends the token as the service asks, usually as
a bearer token:

```sh
fctl id-token --audience https://api.acme.dev            # the token, alone
fctl id-token --audience https://api.acme.dev --claims   # what it says, without it
fctl id-token --audience https://api.acme.dev --json     # {"token", "expires_at", "claims"}
fctl id-token --audience sts.amazonaws.com --aws-session-tags
fctl id-token --audience api://AzureADTokenExchange --subject project
fctl id-token --audience https://api.acme.dev --nonce 4f1c2e…
```

| Option | Does |
|---|---|
| `--audience`, `-a` | Whom the token is for, its `aud`: printable ASCII without spaces, up to 512 characters. Required. |
| `--subject PARTS` | Which facts make up its subject (`sub`), comma-separated: `workspace`, `project`, `user`, `session`, `runner`, `github_access`. Always in that order. Without it, `project` and `user`. |
| `--nonce` | A value the token repeats in `nonce`, for a service that takes each token once. |
| `--aws-session-tags` | Session tags for AWS ([AWS](#aws)). |
| `--claims` | Prints the claims rather than the token. |

Every agent is told how in its `~/AGENTS.md`, with its issuer, and to send
a token only to the service it is for. A session mints at most 30 tokens
a minute. It fails, and says why, where the project turned them off, for
a review agent, and for a session whose sandbox no longer runs. The
sandbox needs the `fctl` of a release that has `fctl id-token` (the
default image's), and its runner a host of that release.

## Claims

```json
{
  "iss": "https://app.fctl.sh/oidc/wsp_01JC3F5W5Q0Q7A2N9X4D1V6ZBK",
  "sub": "project:prj_01JC3F6B3M7Y8T1R5H2E9W4Q0N:user:usr_01JC3F2Z8A4K6P0S3D9F1G7H5J",
  "aud": "https://api.acme.dev",
  "iat": 1795161600,
  "nbf": 1795161595,
  "exp": 1795162200,
  "jti": "idt_01KAD8V4N2P6R0T3W5Y7Z9B1C3",
  "workspace_id": "wsp_01JC3F5W5Q0Q7A2N9X4D1V6ZBK",
  "workspace_slug": "acme",
  "project_id": "prj_01JC3F6B3M7Y8T1R5H2E9W4Q0N",
  "project_slug": "web",
  "session_id": "ses_01KAD8RZ3X5V7N9Q1S4U6W8Y0A",
  "session_number": 212,
  "session_url": "https://app.fctl.sh/s/ses_01KAD8RZ3X5V7N9Q1S4U6W8Y0A",
  "user_id": "usr_01JC3F2Z8A4K6P0S3D9F1G7H5J",
  "user_email": "ada@acme.dev",
  "runner_id": "rnr_01JC4A9C1E3G5J7L9N1Q3S5U7W",
  "runner_name": "build-box",
  "runner_personal": false,
  "repository": "acme/web",
  "branch": "fctl/212-fix-the-cart",
  "base_branch": "main",
  "github_access": "push",
  "started_by": "person"
}
```

| Claim | Says | In |
|---|---|---|
| `iss` | The workspace's issuer. | every token |
| `sub` | `project:<project id>:user:<person's id>`, or the parts the agent chose ([Subject](#subject)). | every token |
| `aud` | The audience the agent asked for. | every token |
| `iat`, `nbf`, `exp` | When it was signed, in seconds; it counts from 5 seconds before and expires 10 minutes after. | every token |
| `jti` | Its own id, `idt_…`, never the same twice. | every token |
| `nonce` | The nonce the agent passed. | tokens that asked |
| `workspace_id`, `workspace_slug` | The workspace. Its slug can change; its id can't. | every token |
| `project_id`, `project_slug` | The project. | every token |
| `session_id`, `session_number`, `session_url` | The session: its id, its number in the project and its page. | every token |
| `user_id` | The person it runs for: who started it, or for whom a schedule, an autopilot, a ticket delegated to fctl or a service account started it. | every token |
| `user_email` | The address they are a member of the workspace as. Addresses change: trust `user_id`. | tokens of members |
| `runner_id`, `runner_name` | The runner it runs on. | every token |
| `runner_personal` | `true` for a runner that a member added for their own sessions, which runs no one else's in the workspace. | every token |
| `repository` | The project's primary repository, `owner/name`. | projects with one |
| `branch`, `base_branch` | The branch its agent works on, and the one it started from. | every token |
| `github_access` | What its agent may do on GitHub: `push`, `comment`, `triage` or `read`. | every token |
| `started_by` | `person` (themselves, or a tool with their access token), `schedule`, `ticket`, `autopilot` or `service_account`, as webhooks say it. | every token |
| `schedule_id` | The schedule that started it. | sessions of schedules |
| `ticket` | The ticket it started from: `ENG-123`, `acme/web#12`. | sessions of tickets |
| `autopilot_id`, `autopilot_role` | The autopilot's project, and `pilot` for its own session or `crew` for one it started. | sessions of autopilots |
| `service_account_id` | The service account that started it. | sessions of service accounts |
| `https://aws.amazon.com/tags` | Session tags ([AWS](#aws)). | tokens that asked |

The discovery document lists them (`claims_supported`). Later versions may
add claims: take the ones you know, and leave the others.

### Subject

`sub` names the project and the person, `project:prj_…:user:usr_…`, so
that a pattern picks the agents a service trusts: `project:prj_…:user:*`
for a project's, `project:*:user:usr_…` for one person's in every project.
An agent may ask for other parts (`--subject`), always in the order
`workspace`, `project`, `user`, `session`, `runner`, `github_access`: a
cloud that matches the subject exactly, as Azure does, takes
`project:prj_…` (`--subject project`), and one that should trust only
agents that push takes `project:prj_…:github_access:push`. The parts are
ids and words without a `:`, so no value passes for another part.

## Checking a token

A service that takes tokens checks, in this order:

1. its signature, RS256 with the key its header names (`kid`), from the
   issuer's keys (`{issuer}/.well-known/jwks.json`), which it may keep for
   an hour;
2. `iss`: exactly your workspace's issuer, the only one it trusts;
3. `aud`: exactly its own audience. fctl lets agents ask for any audience,
   so a token for another service must not pass;
4. `exp` and `nbf`, with a few seconds of leeway;
5. the claims its rules are about: `project_id`, `user_id`,
   `github_access`, `started_by`, `runner_id`.

Any OpenID Connect or JWT library does the first four. In Node, with
[jose](https://github.com/panva/jose):

```js
import { createRemoteJWKSet, jwtVerify } from "jose";

const issuer = "https://app.fctl.sh/oidc/wsp_01JC3F5W5Q0Q7A2N9X4D1V6ZBK";
const keys = createRemoteJWKSet(new URL(`${issuer}/.well-known/jwks.json`));

export async function agentOf(request) {
  const token = request.headers.get("authorization")?.replace(/^Bearer /, "") ?? "";
  const { payload } = await jwtVerify(token, keys, {
    issuer,
    audience: "https://api.acme.dev",
    algorithms: ["RS256"],
    clockTolerance: 30,
  });
  if (payload.project_id !== "prj_01JC3F6B3M7Y8T1R5H2E9W4Q0N") throw new Error("Not an agent of web");
  return payload; // payload.user_id, payload.session_url, …
}
```

In Python, with [PyJWT](https://pyjwt.readthedocs.io):

```python
import jwt

ISSUER = "https://app.fctl.sh/oidc/wsp_01JC3F5W5Q0Q7A2N9X4D1V6ZBK"
keys = jwt.PyJWKClient(f"{ISSUER}/.well-known/jwks.json")

def agent_of(token: str) -> dict:
    key = keys.get_signing_key_from_jwt(token)
    claims = jwt.decode(token, key, algorithms=["RS256"], issuer=ISSUER,
                        audience="https://api.acme.dev", leeway=30)
    if claims["project_id"] != "prj_01JC3F6B3M7Y8T1R5H2E9W4Q0N":
        raise PermissionError("not an agent of web")
    return claims
```

A project's issuer, keys and subject are on its **Settings → Identity OIDC**
page, and `fctl projects identity -p web` prints them.

## Clouds

A project's maintainers connect its agents to AWS, Google Cloud and Azure
on its **Settings → Identity OIDC** page or with `fctl projects set`. Each
cloud trusts the project's agents once someone sets it up to: the same
page, and `fctl projects identity -p web`, give the commands and policies
for it, filled in. fctl-agent then keeps a token for each cloud in a file
in the sandbox, minted every five minutes, and sets the variables the
cloud's CLI and SDKs read: they trade the token for the cloud's own
short-lived credentials themselves, and do again as those run out. fctl
never talks to the cloud, and holds none of its credentials.

The project's own variables come after the clouds' and win
(`AWS_ROLE_SESSION_NAME`, say), and the setup script gets them too, so it
can install packages from a private registry: it waits up to 15 seconds
for the first tokens. A session started before a change of the clouds
takes it when it next starts or resumes.

### AWS

```sh
fctl projects set -p web --aws-role arn:aws:iam::123456789012:role/fctl-web --aws-region eu-north-1
fctl projects identity -p web
```

AWS needs an IAM OIDC identity provider for your workspace's issuer, once
in each account, with the audience `sts.amazonaws.com`, and a trust policy
on the role that lets the project's agents assume it:

```sh
aws iam create-open-id-connect-provider \
  --url https://app.fctl.sh/oidc/wsp_01JC3F5W5Q0Q7A2N9X4D1V6ZBK \
  --client-id-list sts.amazonaws.com
aws iam update-assume-role-policy --role-name fctl-web --policy-document file://trust-policy.json
```

```json
{
  "Version": "2012-10-17",
  "Statement": [{
    "Effect": "Allow",
    "Principal": { "Federated": "arn:aws:iam::123456789012:oidc-provider/app.fctl.sh/oidc/wsp_01JC3F5W5Q0Q7A2N9X4D1V6ZBK" },
    "Action": ["sts:AssumeRoleWithWebIdentity"],
    "Condition": {
      "StringEquals": { "app.fctl.sh/oidc/wsp_01JC3F5W5Q0Q7A2N9X4D1V6ZBK:aud": "sts.amazonaws.com" },
      "StringLike": { "app.fctl.sh/oidc/wsp_01JC3F5W5Q0Q7A2N9X4D1V6ZBK:sub": "project:prj_01JC3F6B3M7Y8T1R5H2E9W4Q0N:user:*" }
    }
  }]
}
```

The project's sessions get `AWS_ROLE_ARN`, `AWS_WEB_IDENTITY_TOKEN_FILE`,
`AWS_ROLE_SESSION_NAME` (`fctl-web-212`, which CloudTrail shows) and, with
a region, `AWS_REGION` and `AWS_DEFAULT_REGION`: `aws sts
get-caller-identity` shows the role.

AWS's conditions see only `aud` and `sub` of a token. For the rest, turn
on session tags (`--aws-session-tags on`): its tokens then carry
`fctl:workspace`, `fctl:project`, `fctl:user`, `fctl:session`,
`fctl:runner`, `fctl:github_access` and `fctl:started_by` as session tags,
all transitive. The trust policy must then allow `sts:TagSession` too, or
AWS refuses the role, and it can check them with `aws:RequestTag`, while
the role's permissions use them with `aws:PrincipalTag`:

```json
"Action": ["sts:AssumeRoleWithWebIdentity", "sts:TagSession"],
"Condition": {
  "StringEquals": {
    "app.fctl.sh/oidc/wsp_01JC3F5W5Q0Q7A2N9X4D1V6ZBK:aud": "sts.amazonaws.com",
    "aws:RequestTag/fctl:project": "prj_01JC3F6B3M7Y8T1R5H2E9W4Q0N",
    "aws:RequestTag/fctl:github_access": "push",
    "aws:RequestTag/fctl:started_by": "person"
  }
}
```

```json
{
  "Effect": "Allow",
  "Action": ["s3:GetObject", "s3:PutObject"],
  "Resource": "arn:aws:s3:::acme-agent-scratch/${aws:PrincipalTag/fctl:session}/*"
}
```

### Google Cloud

```sh
fctl projects set -p web \
  --gcp-provider projects/123456789/locations/global/workloadIdentityPools/fctl/providers/acme \
  --gcp-service-account agents@acme-prod.iam.gserviceaccount.com
fctl projects identity -p web
```

Google Cloud needs a workload identity pool with an OIDC provider for your
workspace's issuer, which maps the claims rules are about to attributes,
and either lets the project's agents act as a service account or gives
them roles itself, as the principal set
`principalSet://iam.googleapis.com/projects/123456789/locations/global/workloadIdentityPools/fctl/attribute.project_id/prj_…`:

```sh
gcloud iam workload-identity-pools create projects/123456789/locations/global/workloadIdentityPools/fctl --location=global
gcloud iam workload-identity-pools providers create-oidc \
  projects/123456789/locations/global/workloadIdentityPools/fctl/providers/acme \
  --location=global \
  --issuer-uri="https://app.fctl.sh/oidc/wsp_01JC3F5W5Q0Q7A2N9X4D1V6ZBK" \
  --attribute-mapping="google.subject=assertion.sub,attribute.workspace_id=assertion.workspace_id,attribute.project_id=assertion.project_id,attribute.user_id=assertion.user_id,attribute.github_access=assertion.github_access,attribute.started_by=assertion.started_by" \
  --attribute-condition="assertion.workspace_id == 'wsp_01JC3F5W5Q0Q7A2N9X4D1V6ZBK'"
gcloud iam service-accounts add-iam-policy-binding agents@acme-prod.iam.gserviceaccount.com \
  --role=roles/iam.workloadIdentityUser \
  --member="principalSet://iam.googleapis.com/projects/123456789/locations/global/workloadIdentityPools/fctl/attribute.project_id/prj_01JC3F6B3M7Y8T1R5H2E9W4Q0N"
```

The tokens' audience is the provider's full resource URL, which providers
take unless you choose others. The sessions get
`GOOGLE_APPLICATION_CREDENTIALS` and
`CLOUDSDK_AUTH_CREDENTIAL_FILE_OVERRIDE`, naming a credential
configuration that reads the token from its file, as `gcloud iam
workload-identity-pools create-cred-config` writes one: the client
libraries and `gcloud` sign in on their own, with a project from
`--project`. A provider's attribute condition narrows it further, in CEL:
`assertion.github_access == 'push' && assertion.started_by == 'person'`.

### Azure

```sh
fctl projects set -p web --azure-tenant 72f988bf-86f1-41af-91ab-2d7cd011db47 --azure-client 6d1c1d3b-2f9b-4a2e-9a52-0b8f5e1e3c7a
fctl projects identity -p web
```

Azure needs a federated credential on the application (an app
registration, or a user-assigned managed identity) that trusts your
workspace's issuer, with the subject `project:prj_…` and the audience
`api://AzureADTokenExchange`. Azure matches the subject exactly, so the
project's Azure tokens name the project alone:

```sh
az ad app federated-credential create --id 6d1c1d3b-2f9b-4a2e-9a52-0b8f5e1e3c7a --parameters '{
  "name": "fctl-web",
  "issuer": "https://app.fctl.sh/oidc/wsp_01JC3F5W5Q0Q7A2N9X4D1V6ZBK",
  "subject": "project:prj_01JC3F6B3M7Y8T1R5H2E9W4Q0N",
  "audiences": ["api://AzureADTokenExchange"]
}'
# or, for a managed identity
az identity federated-credential create --name fctl-web --identity-name agents --resource-group acme \
  --issuer https://app.fctl.sh/oidc/wsp_01JC3F5W5Q0Q7A2N9X4D1V6ZBK \
  --subject project:prj_01JC3F6B3M7Y8T1R5H2E9W4Q0N --audiences api://AzureADTokenExchange
```

The sessions get `AZURE_CLIENT_ID`, `AZURE_TENANT_ID`,
`AZURE_FEDERATED_TOKEN_FILE` and `AZURE_AUTHORITY_HOST`, which the Azure
SDKs' `DefaultAzureCredential` signs in with. The Azure CLI signs in with
`az login --service-principal -u "$AZURE_CLIENT_ID" -t "$AZURE_TENANT_ID"
--federated-token "$(cat "$AZURE_FEDERATED_TOKEN_FILE")"`.

### HashiCorp Vault and others

Anything that takes OpenID Connect ID tokens takes an agent's, with the
audience it expects. Vault's JWT auth method, for one, finds the keys from
the issuer and binds a role to claims:

```sh
vault auth enable -path=fctl jwt
vault write auth/fctl/config \
  oidc_discovery_url=https://app.fctl.sh/oidc/wsp_01JC3F5W5Q0Q7A2N9X4D1V6ZBK \
  bound_issuer=https://app.fctl.sh/oidc/wsp_01JC3F5W5Q0Q7A2N9X4D1V6ZBK
vault write auth/fctl/role/web-agents - <<'EOF'
{
  "role_type": "jwt",
  "user_claim": "sub",
  "bound_audiences": ["https://vault.acme.dev"],
  "bound_claims": { "project_id": "prj_01JC3F6B3M7Y8T1R5H2E9W4Q0N", "github_access": "push" },
  "token_policies": ["web-agents"],
  "token_ttl": "15m"
}
EOF

# in a session
vault write auth/fctl/login role=web-agents jwt="$(fctl id-token -a https://vault.acme.dev)"
```

## What a token proves

A token proves that whoever holds it got it from the control plane for
that session, through the runner that runs it, in the last 10 minutes. It
doesn't prove what the agent meant to do. Keep in mind:

- **Whatever runs in the sandbox can mint one**: the agent, the tools and
  scripts it runs, code in the repository. Give a role or a service what
  the project's agents should do there, and no more.
- **So can the runner's owner**, who controls the machine and its host,
  for the sessions the runner runs. Trust workspace runners alone with
  `runner_personal` false, or certain runners with `runner_id`.
- **Text from outside can steer an agent.** Triage agents
  (`github_access: triage`) read tickets anyone may write, and schedules
  start sessions on events on GitHub (`started_by: schedule`). Leave
  them out where it matters, with `github_access` and `started_by`.
  Review agents get no tokens at all.
- **Check the audience.** fctl signs a token for whichever audience an
  agent asks for, so that it can reach any of your services; a service
  that takes another's audience takes tokens meant elsewhere.
- **Tokens leave the sandbox only as the agent sends them.** Agents are
  told to keep them out of what they print, but one may show in a
  transcript: after 10 minutes it is worth nothing.
- **Your workspace's issuer is yours.** Trust its address alone: a token
  from another workspace has another `iss`, and its signature covers it.

## Your own instance

An instance makes its first key when an issuer is first asked for its
keys or an agent first mints a token, sealed with `FCTL_MASTER_KEY`, which
every instance has. For clouds to trust its agents, they must reach its
issuers: its public address (`FCTL_PUBLIC_URL`) must be `https://`, with
a certificate they trust, and `/oidc/` must reach the control plane from
the internet, as it does through Cloudflare and the standalone server. A
service on your own network can check tokens against an instance on it.
