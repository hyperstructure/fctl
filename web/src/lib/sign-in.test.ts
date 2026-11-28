import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { DEFAULT_POLICY } from "./session-policy";
import {
  consequences,
  domainOf,
  domainsWords,
  parseDomains,
  providers,
  ruleFor,
  says,
  startHref,
  today,
  without,
  withRequired,
} from "./sign-in";

const google: G.SignInProvider = { id: "google", name: "Google", start: "/api/v1/auth/google/start" };

function instance(rules: G.SignInRule[] = [], features: Partial<G.Features> = {}): G.InstanceInfo {
  return {
    name: "Acme factory",
    version: "1.0.0",
    setup_required: false,
    features: {
      google: true,
      email: true,
      github: true,
      publish_screenshots: false,
      early_access: false,
      ...features,
    },
    sign_in: { providers: [google], rules, self_sign_up: true },
  };
}

const rule = (domains: string[], passkeys = false): G.SignInRule => ({
  domains,
  provider: "google",
  passkeys,
});

const person = (name: string): G.SignInPerson => ({
  id: name.toLowerCase(),
  name,
  email: `${name.toLowerCase()}@acme.com`,
});

describe("the sign-in page and its rules", () => {
  it("turns an address at a covered domain into its provider's button", () => {
    const acme = instance([rule(["acme.com", "acme.dev"])]);
    expect(ruleFor(acme, "ada@ACME.dev")?.provider?.name).toBe("Google");
    expect(ruleFor(acme, "ada@acme.com ")?.rule.domains).toEqual(["acme.com", "acme.dev"]);
    // Not yet an address, another domain, a subdomain.
    expect(ruleFor(acme, "ada@")).toBeUndefined();
    expect(ruleFor(acme, "ada")).toBeUndefined();
    expect(ruleFor(acme, "cy@contractor.io")).toBeUndefined();
    expect(ruleFor(acme, "dee@eu.acme.com")).toBeUndefined();
    expect(domainOf("ada@acme.com")).toBe("acme.com");
  });

  it("shows Google from control planes before sign-in rules, and nothing where it isn't set up", () => {
    const old = { ...instance(), sign_in: undefined };
    expect(providers(old).map((p) => p.id)).toEqual(["google"]);
    expect(providers({ ...old, features: { ...old.features, google: false } })).toEqual([]);
    expect(providers(instance())).toEqual([google]);
  });

  it("starts a provider's sign-in with where to go next and the address typed", () => {
    expect(startHref(google)).toBe("/api/v1/auth/google/start?intent=login");
    expect(startHref(google, { next: "/p/web", hint: " ada@acme.com" })).toBe(
      "/api/v1/auth/google/start?intent=login&return_to=%2Fp%2Fweb&login_hint=ada%40acme.com",
    );
    expect(startHref(google, { intent: "link", next: "/settings" })).toBe(
      "/api/v1/auth/google/start?intent=link&return_to=%2Fsettings",
    );
  });

  it("says what a rule requires as the server does", () => {
    expect(says(["acme.com"], "Google")).toBe("acme.com signs in with Google");
    expect(says(["acme.com", "acme.dev"], "Google")).toBe("acme.com and acme.dev sign in with Google");
    expect(domainsWords(["a.io", "b.io", "c.io"], "or")).toBe("a.io, b.io or c.io");
  });
});

describe("changing the rules", () => {
  it("reads the domains an admin types, and those that aren't any", () => {
    expect(parseDomains("ACME.com, *@acme.dev\n@acme.io acme.com")).toEqual({
      domains: ["acme.com", "acme.dev", "acme.io"],
      invalid: [],
    });
    expect(parseDomains("acme, ada@acme.com acme.com").invalid).toEqual(["acme", "ada@acme.com"]);
  });

  it("moves domains between rules as the CLI does", () => {
    const rules = [rule(["acme.com"])];
    expect(withRequired(rules, ["ACME.dev"], "google", false)).toEqual([rule(["acme.com", "acme.dev"])]);
    expect(withRequired(rules, ["acme.com", "acme.io"], "google", true)).toEqual([
      rule(["acme.com", "acme.io"], true),
    ]);
    expect(without([rule(["acme.com", "acme.dev"])], ["acme.dev"])).toEqual([rule(["acme.com"])]);
    expect(without(rules, ["acme.com"])).toEqual([]);
    // The rules it was given stay as they were.
    expect(rules).toEqual([rule(["acme.com"])]);
  });

  it("says whom a change concerns before it saves", () => {
    const preview: G.SignInPreview = {
      covered: 42,
      passwords: [person("Bob"), person("Cy")],
      password_count: 38,
      passkeys: [person("Bob")],
      passkey_count: 1,
      access_tokens: 5,
      admin_tokens: 1,
      linked: 3,
      first: 0,
      released: 0,
      others: 4,
      owner: person("Ada"),
      you: "meets",
      email: true,
    };
    const lines = consequences(preview, "Google", "bob");
    expect(lines[0]).toBe(
      "38 people sign in with a password today: their next sign-in goes through Google, and their passwords stop working.",
    );
    expect(lines).toContain("4 people at other domains sign in as before.");
    expect(lines).toContain(
      "Their 5 access tokens and CLI sign-ins made with another way in go: they make new ones after signing in with Google.",
    );
    expect(lines).toContain(
      "1 admin token made with another way in stops working: a program that provisions people with one needs a new one.",
    );
    expect(lines.at(-1)).toBe(
      "Ada, the primary instance owner, keeps their password and passkeys: the way in if Google is down.",
    );
    expect(consequences({ ...preview, you: "owner" }, "Google", "ada").at(-1)).toContain(
      "You keep your password",
    );
    const quiet = {
      ...preview,
      covered: 0,
      password_count: 0,
      passkey_count: 0,
      access_tokens: 0,
      admin_tokens: 0,
      linked: 0,
      owner: undefined,
    };
    expect(consequences(quiet, "Google")).toEqual(["Nobody's way in changes."]);
  });

  it("says how people sign in today", () => {
    const overview: G.SignInOverview = {
      providers: [
        {
          id: "google",
          name: "Google",
          configured: true,
          setup: "GOOGLE_CLIENT_ID and GOOGLE_CLIENT_SECRET",
          redirect_url: "https://fctl.acme.dev/api/v1/auth/google/callback",
          people: 12,
        },
      ],
      rules: [],
      coverage: [],
      self_sign_up: true,
      active: 52,
      passwords: 38,
      passkeys: 9,
      no_way_in: 3,
      email: true,
      sessions: DEFAULT_POLICY,
    };
    expect(today(overview)).toBe(
      "52 people have an account: 38 with a password, 12 with Google and 9 with a passkey. 3 people haven't signed in yet.",
    );
    expect(today({ ...overview, active: 0 })).toBe("Nobody has an account yet.");
  });
});
