import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import {
  askedTab,
  claudeGist,
  emailsGist,
  linearGist,
  SETTINGS_PAGES,
  SETTINGS_TABS,
  securityGist,
  settingsPageFor,
  tokensGist,
  workspacesGist,
} from "./settings";

describe("Settings' pages", () => {
  it("lists every page once", () => {
    expect(SETTINGS_PAGES.map((p) => p.tab).sort()).toEqual([...SETTINGS_TABS].sort());
  });

  it("finds a page by its name, its label or what it holds", () => {
    expect(settingsPageFor("tokens")).toBe("tokens");
    expect(settingsPageFor("Email-addresses")).toBe("emails");
    expect(settingsPageFor("passkeys")).toBe("security");
    expect(settingsPageFor("password")).toBe("security");
    expect(settingsPageFor("theme")).toBe("appearance");
    expect(settingsPageFor("billing")).toBeUndefined();
    expect(settingsPageFor("")).toBeUndefined();
  });
});

describe("the page an address opens", () => {
  it("is its tab", () => {
    expect(askedTab({ tab: "notifications" }, "")).toBe("notifications");
    expect(askedTab({ tab: "claude", linked: "google" }, "#linear")).toBe("claude");
  });

  it("is Settings itself without one", () => {
    expect(askedTab({}, "")).toBeUndefined();
    expect(askedTab({}, "#nothing-here")).toBeUndefined();
  });

  it("is the page with the section an older link named", () => {
    expect(askedTab({}, "#claude")).toBe("claude");
    expect(askedTab({}, "#connect-claude")).toBe("claude");
    expect(askedTab({}, "#linear")).toBe("linear");
    expect(askedTab({}, "#security")).toBe("security");
  });

  it("is where Google and Linear send people back to", () => {
    expect(askedTab({ linked: "google" }, "")).toBe("security");
    expect(askedTab({ error: "google_cancelled" }, "")).toBe("security");
    expect(askedTab({ linear: "linked" }, "#linear")).toBe("linear");
    expect(askedTab({ linear: "linked" }, "")).toBe("linear");
  });
});

const claude = (fields: Partial<G.ClaudeCredentialInfo> = {}): G.ClaudeCredentialInfo => ({
  id: "crd_1",
  kind: "setup_token",
  remote_control: false,
  status: "active",
  is_default: true,
  created_at: 0,
  ...fields,
});

const twoFactor = (fields: Partial<G.TwoFactor> = {}): G.TwoFactor => ({
  recovery_codes: 0,
  google: false,
  google_required: false,
  required: false,
  setup_required: false,
  on: false,
  ...fields,
});

const passkey = { id: "pk_1" } as G.Passkey;

describe("what the list of pages says", () => {
  it("names the addresses", () => {
    const user = { email: "ada@example.com" } as G.User;
    const address = (email: string, primary = false) => ({ email, primary }) as G.EmailAddress;
    expect(emailsGist({ user, emails: [] }).text).toBe("ada@example.com");
    expect(emailsGist({ user, emails: [address("ada@example.com", true)] }).text).toBe("ada@example.com");
    expect(
      emailsGist({ user, emails: [address("ada@work.example", true), address("ada@example.com")] }).text,
    ).toBe("ada@work.example and 1 more");
  });

  it("asks for a Claude connection until there is one that works", () => {
    expect(claudeGist([])).toEqual({ text: "Not connected yet", attention: true });
    expect(claudeGist([claude({ status: "invalid" })])).toEqual({
      text: "Needs reconnecting",
      attention: true,
    });
    expect(
      claudeGist([
        claude({ account_email: "ada@example.com" }),
        claude({ label: "Spare", status: "invalid" }),
      ]),
    ).toEqual({ text: "Spare needs reconnecting", attention: true });
    expect(claudeGist([claude({ account_email: "ada@example.com" })])).toEqual({ text: "ada@example.com" });
    expect(claudeGist([claude(), claude({ id: "crd_2", is_default: false })])).toEqual({
      text: "2 connections",
    });
  });

  it("says first what Security needs", () => {
    expect(securityGist({ two_factor: twoFactor(), passkeys: [] })).toEqual({
      text: "Two-factor sign-in off",
    });
    expect(securityGist({ two_factor: twoFactor({ on: true }), passkeys: [passkey, passkey] })).toEqual({
      text: "Two-factor sign-in on · 2 passkeys",
    });
    expect(securityGist({ two_factor: twoFactor(), passkeys: [] }, 2)).toEqual({
      text: "Two-factor sign-in required",
      attention: true,
    });
    const app = { on: true, authenticator_since: 1 };
    expect(
      securityGist({ two_factor: twoFactor({ ...app, recovery_codes: 2 }), passkeys: [] }).attention,
    ).toBe(true);
    expect(securityGist({ two_factor: twoFactor({ ...app, recovery_codes: 0 }), passkeys: [] }).text).toBe(
      "No recovery codes left",
    );
    // Without an authenticator app there are no recovery codes to run out of.
    expect(
      securityGist({ two_factor: twoFactor({ on: true }), passkeys: [passkey] }).attention,
    ).toBeUndefined();
  });

  it("counts workspaces, Linear accounts and access tokens once they've loaded", () => {
    expect(workspacesGist(undefined)).toBeUndefined();
    expect(workspacesGist([])?.text).toBe("None yet");
    expect(workspacesGist([{}, {}])?.text).toBe("2 workspaces");
    expect(linearGist(undefined)).toBeUndefined();
    expect(linearGist([])?.text).toBe("Not linked");
    expect(linearGist([{ organization_name: "Acme" } as G.LinearAccount])?.text).toBe("Linked in Acme");
    expect(tokensGist(undefined)).toBeUndefined();
    // Browsers and the CLI are devices, not access tokens.
    expect(tokensGist([{ kind: "web" }, { kind: "cli" }])?.text).toBe("None yet");
    expect(tokensGist([{ kind: "pat" }, { kind: "web" }])?.text).toBe("1 token");
  });
});
