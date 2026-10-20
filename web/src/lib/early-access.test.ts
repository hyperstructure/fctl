import { afterEach, describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { invitationMail, loadSent, ordered, requestState, saveSent } from "./early-access";

const NOW = Date.UTC(2026, 9, 20, 12);
const DAY = 86_400_000;

function request(email: string, fields: Partial<G.AccessRequest> = {}): G.AccessRequest {
  return { id: `acr_${email}`, email, created_at: NOW - DAY, asked_at: NOW - DAY, ...fields };
}

afterEach(() => localStorage.clear());

describe("the request this browser sent", () => {
  it("is remembered, and forgotten", () => {
    expect(loadSent()).toBeUndefined();
    saveSent({ email: "ada@example.com", at: NOW, token: "fctl_acr_abc" });
    expect(loadSent(NOW)).toEqual({ email: "ada@example.com", at: NOW, token: "fctl_acr_abc" });
    saveSent(undefined);
    expect(loadSent(NOW)).toBeUndefined();
  });

  it("is forgotten a year on, as the control plane forgets it", () => {
    saveSent({ email: "ada@example.com", at: NOW });
    expect(loadSent(NOW + 364 * DAY)).toEqual({ email: "ada@example.com", at: NOW, token: undefined });
    expect(loadSent(NOW + 366 * DAY)).toBeUndefined();
  });

  it("is forgotten when what's stored isn't one", () => {
    localStorage.setItem("fctl:early-access", "{nope");
    expect(loadSent()).toBeUndefined();
    localStorage.setItem("fctl:early-access", JSON.stringify({ email: "ada@example.com" }));
    expect(loadSent()).toBeUndefined();
  });
});

describe("a request's state", () => {
  it("waits until the address is invited", () => {
    expect(requestState(request("ada@example.com"), NOW)).toBe("waiting");
  });

  it("is invited while the link works, and expired after", () => {
    const invited = request("ada@example.com", { invited_at: NOW - DAY, invite_expires_at: NOW + 13 * DAY });
    expect(requestState(invited, NOW)).toBe("invited");
    expect(requestState(invited, NOW + 14 * DAY)).toBe("expired");
    // The control plane leaves the expiry out once no link works.
    expect(requestState(request("ada@example.com", { invited_at: NOW - 20 * DAY }), NOW)).toBe("expired");
  });
});

describe("the list of requests", () => {
  it("puts those waiting longest first, then the latest invited", () => {
    const { waiting, invited } = ordered([
      request("new@example.com", { created_at: NOW - DAY }),
      request("old@example.com", { created_at: NOW - 9 * DAY }),
      request("first@example.com", { invited_at: NOW - 5 * DAY, invite_expires_at: NOW + 9 * DAY }),
      request("latest@example.com", { invited_at: NOW - DAY, invite_expires_at: NOW + 13 * DAY }),
      request("expired@example.com", { invited_at: NOW - 30 * DAY }),
    ]);
    expect(waiting.map((r) => r.email)).toEqual(["old@example.com", "new@example.com"]);
    expect(invited.map((r) => r.email)).toEqual([
      "latest@example.com",
      "first@example.com",
      "expired@example.com",
    ]);
  });
});

describe("the invitation email", () => {
  it("is addressed to them, with the link", () => {
    const mail = new URL(
      invitationMail({
        email: "ada+fctl@example.com",
        url: "https://app.fctl.sh/invite#token=fctl_inv_abc",
        instance: "fctl",
        expiresAt: NOW + 14 * DAY,
      }),
    );
    expect(mail.protocol).toBe("mailto:");
    expect(mail.pathname).toBe("ada%2Bfctl@example.com");
    expect(mail.searchParams.get("subject")).toBe("Your early access to fctl");
    const body = mail.searchParams.get("body") ?? "";
    expect(body).toContain("\r\nhttps://app.fctl.sh/invite#token=fctl_inv_abc\r\n");
    expect(body).toMatch(/It works once, until .+\.$/);
  });
});
