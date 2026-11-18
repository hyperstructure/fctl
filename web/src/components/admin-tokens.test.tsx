import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys } from "@/lib/queries";
import { AdminTokens, provisionExample } from "./admin-tokens";

// No request is answered: the section shows what the cache holds.
vi.mock("@/lib/api", () => {
  const pending = new Proxy({}, { get: () => () => new Promise(() => {}) });
  return { api: pending, ApiFailure: class extends Error {} };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

function section(list: G.AdminTokenList) {
  const client = new QueryClient();
  client.setQueryData<G.AdminTokenList>(keys.adminTokens, list);
  const container = document.createElement("div");
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={client}>
        <AdminTokens />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => act(() => root.unmount()));
  return container;
}

const okta: G.AdminToken = {
  id: "adt_1",
  name: "Okta provisioning",
  created_by: { id: "usr_kofi", name: "Kofi Mensah" },
  created_at: Date.now() - 86_400_000,
};

const buttons = (page: HTMLElement) => [...page.querySelectorAll("button")].map((b) => b.textContent);

describe("admin tokens in the instance's settings", () => {
  it("lets admins make one where the instance takes them", () => {
    const page = section({ enabled: true, tokens: [] });
    expect(page.textContent).toContain("Admin tokens");
    expect(page.textContent).toContain("None yet");
    expect(buttons(page)).toContain("New admin token");
  });

  it("says who made each one, whether it was used and when it expires", () => {
    const page = section({ enabled: true, tokens: [okta] });
    expect(page.textContent).toContain("Okta provisioning");
    expect(page.textContent).toContain("Made by Kofi Mensah");
    expect(page.textContent).toContain("not used yet");
    expect(page.textContent).toContain("never expires");
    expect(page.querySelector('[aria-label="Revoke Okta provisioning"]')).not.toBeNull();
  });

  it("shows nothing where the instance doesn't take them, as on app.fctl.sh", () => {
    const page = section({ enabled: false, tokens: [] });
    expect(page.textContent).toBe("");
  });

  it("leaves the ones made before to revoke where the instance stopped taking them", () => {
    const page = section({ enabled: false, tokens: [okta] });
    expect(page.textContent).toContain("doesn't take admin tokens any more");
    expect(page.textContent).toContain("Okta provisioning");
    expect(buttons(page)).not.toContain("New admin token");
  });

  it("shows a request that creates an account on this instance", () => {
    const example = provisionExample("fctl_adm_secret", "https://fctl.acme.dev");
    expect(example).toContain("curl -X POST https://fctl.acme.dev/api/v1/admin/users");
    expect(example).toContain("Authorization: Bearer fctl_adm_secret");
  });
});
