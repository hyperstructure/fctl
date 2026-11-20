import { QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import { IdentitySettings } from "./identity";

// No request is answered but a project's update, which answers with what it was sent; the identity
// is whatever the test puts in the cache. Each update is noted.
const updates = vi.hoisted(() => [] as G.UpdateProjectRequest[]);
vi.mock("@/lib/api", async (real) => {
  const pending = new Proxy(
    {},
    {
      get: (_, name) =>
        name === "updateProject"
          ? (_id: string, body: G.UpdateProjectRequest) => {
              updates.push(body);
              const before = queryClient.getQueryData<G.Project>(keys.project("shop"));
              return Promise.resolve({ ...before, ...body });
            }
          : () => new Promise(() => {}),
    },
  );
  return { ...(await real<typeof import("@/lib/api")>()), api: pending };
});
vi.mock("@tanstack/react-router", () => ({
  useBlocker: () => ({ status: "idle" }),
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  queryClient.clear();
  updates.splice(0);
});

const ROLE = "arn:aws:iam::123456789012:role/fctl-shop";

const settings: G.ProjectSettings = {
  auto_pull_request: true,
  remote_control: false,
  claude_ai_connectors: false,
  credit_people: true,
  credit_claude: true,
  publish_screenshots: false,
  package_caches: false,
  project_memory: true,
  retrospectives: true,
  id_tokens: true,
  env: { NODE_ENV: "development" },
};

const project = (s: Partial<G.ProjectSettings> = {}): G.Project => ({
  id: "prj_1",
  slug: "shop",
  name: "Shop",
  default_branch: "main",
  settings: { ...settings, ...s },
  role: "maintainer",
  created_by: { id: "usr_ada", name: "Ada" },
  created_at: 0,
  updated_at: 0,
  active_sessions: 0,
  total_sessions: 3,
});

const identity = (aws?: G.AwsSetup): G.ProjectIdentity => ({
  enabled: true,
  issuer: "https://app.fctl.sh/oidc/wsp_acme",
  discovery_url: "https://app.fctl.sh/oidc/wsp_acme/.well-known/openid-configuration",
  jwks_url: "https://app.fctl.sh/oidc/wsp_acme/.well-known/jwks.json",
  subject: "project:prj_1:user:*",
  lifetime_secs: 600,
  ...(aws && { aws }),
});

const AWS_SETUP: G.AwsSetup = {
  provider_arn: "arn:aws:iam::123456789012:oidc-provider/app.fctl.sh/oidc/wsp_acme",
  audience: "sts.amazonaws.com",
  trust_policy: '{\n  "Version": "2012-10-17"\n}',
  commands: "aws iam create-open-id-connect-provider --url https://app.fctl.sh/oidc/wsp_acme\n",
};

/** The project's Identity page, with `id` as what the control plane says of it. */
function open(p: G.Project, id?: G.ProjectIdentity) {
  queryClient.setQueryData(keys.project(p.slug), p);
  if (id) queryClient.setQueryData(keys.projectIdentity(p.id), id);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const show = (q: G.Project) =>
    act(() =>
      root.render(
        <QueryClientProvider client={queryClient}>
          <IdentitySettings project={q} />
        </QueryClientProvider>,
      ),
    );
  show(p);
  /** What the page shows once the route has the project as the control plane answered it. */
  const saved = () => {
    const q = queryClient.getQueryData<G.Project>(keys.project(p.slug));
    if (q) show(q);
  };
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  const input = (label: string) => {
    const l = [...container.querySelectorAll("label")].find((x) => x.textContent === label);
    return container.querySelector(`#${CSS.escape(l?.htmlFor ?? "")}`) as HTMLInputElement;
  };
  const type = (label: string, value: string) =>
    act(() => {
      const el = input(label);
      const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      set?.call(el, value);
      el.dispatchEvent(new Event("input", { bubbles: true }));
    });
  const button = (text: string) =>
    [...container.querySelectorAll("button")].find((b) => b.textContent === text) as HTMLButtonElement;
  const toggle = (label: string) =>
    container.querySelector(`button[role="switch"][aria-label="${label}"]`) as HTMLButtonElement;
  return { container, input, type, button, toggle, saved };
}

describe("a project's Identity page", () => {
  it("shows the workspace's issuer and the subject of the project's tokens", () => {
    const s = open(project(), identity());
    expect(s.container.textContent).toContain("https://app.fctl.sh/oidc/wsp_acme");
    expect(s.container.textContent).toContain("project:prj_1:user:*");
    expect(s.container.querySelector('a[href$="/docs/identity-tokens"]')).not.toBeNull();
    // A cloud's region and session tags wait for its role.
    expect(s.input("Region").disabled).toBe(true);
    expect(s.toggle("Session tags").disabled).toBe(true);
  });

  it("saves a cloud with the rest of the project's settings as they are", async () => {
    const s = open(project(), identity());
    expect(s.button("Save")).toBeUndefined();
    s.type("Role ARN", ` ${ROLE} `);
    s.type("Region", "eu-north-1");
    act(() => s.toggle("Session tags").click());
    await act(async () => s.button("Save").click());
    expect(updates).toEqual([
      {
        settings: {
          ...settings,
          id_tokens: true,
          cloud_access: { aws: { role_arn: ROLE, region: "eu-north-1", session_tags: true } },
        },
      },
    ]);
    // Saved, nothing is left to save.
    s.saved();
    expect(s.button("Save")).toBeUndefined();
  });

  it("takes a cloud away when its first field is emptied, and turns tokens off", async () => {
    const s = open(project({ cloud_access: { aws: { role_arn: ROLE } } }), identity(AWS_SETUP));
    s.type("Role ARN", "");
    act(() => s.toggle("Identity tokens").click());
    expect(s.container.textContent).toContain("reach none of these clouds");
    await act(async () => s.button("Save").click());
    expect(updates.at(-1)?.settings).toMatchObject({ id_tokens: false, cloud_access: {} });
  });

  it("shows what a saved cloud needs, folded, and nothing for one being changed", () => {
    const s = open(project({ cloud_access: { aws: { role_arn: ROLE } } }), identity(AWS_SETUP));
    const fold = s.button("What AWS needs to trust them");
    expect(fold.getAttribute("aria-expanded")).toBe("false");
    expect(s.container.textContent).not.toContain("create-open-id-connect-provider");
    act(() => fold.click());
    expect(s.container.textContent).toContain("create-open-id-connect-provider");
    expect(s.container.textContent).toContain("trust-policy.json");
    expect(s.container.textContent).toContain('"Version": "2012-10-17"');
    // What it shows is the saved role's: changed, it waits for the save.
    s.type("Role ARN", "arn:aws:iam::123456789012:role/other");
    expect(s.button("What AWS needs to trust them")).toBeUndefined();
  });
});
