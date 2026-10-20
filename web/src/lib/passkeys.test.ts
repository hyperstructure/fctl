import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import {
  cancelled,
  createPasskey,
  creationOptions,
  fromB64url,
  requestOptions,
  signalDetails,
  toB64url,
  userHandle,
} from "./passkeys";

const OPTIONS: G.PasskeyCreationOptions = {
  rp: { id: "app.fctl.sh", name: "fctl" },
  user: { id: userHandle("usr_1"), name: "ada@example.com", displayName: "Ada" },
  challenge: toB64url(new Uint8Array([9, 9])),
  pubKeyCredParams: [{ type: "public-key", alg: -7 }],
  timeout: 300000,
  excludeCredentials: [{ type: "public-key", id: toB64url(new Uint8Array([1])), transports: ["internal"] }],
  authenticatorSelection: {
    residentKey: "required",
    requireResidentKey: true,
    userVerification: "required",
  },
  attestation: "none",
};

describe("passkeys", () => {
  it("round-trips base64url without padding", () => {
    for (const bytes of [
      [],
      [0],
      [0xff, 0xfe],
      [1, 2, 3, 4, 5],
      Array.from({ length: 33 }, (_, i) => i * 7),
    ]) {
      const text = toB64url(new Uint8Array(bytes));
      expect(text).not.toMatch(/[+/=]/);
      expect([...fromB64url(text)]).toEqual(bytes);
    }
    expect(toB64url(new Uint8Array([0xfb, 0xff]))).toBe("-_8");
    expect([...fromB64url("-_8")]).toEqual([0xfb, 0xff]);
  });

  it("turns the server's options into the browser's", () => {
    const create = creationOptions(OPTIONS);
    expect(new TextDecoder().decode(create.user.id as Uint8Array)).toBe("usr_1");
    expect([...(create.challenge as Uint8Array)]).toEqual([9, 9]);
    expect(create.excludeCredentials?.[0]?.transports).toEqual(["internal"]);
    expect(create.authenticatorSelection?.userVerification).toBe("required");

    const get = requestOptions({
      challenge: toB64url(new Uint8Array([7])),
      rpId: "app.fctl.sh",
      timeout: 300000,
      allowCredentials: [],
      userVerification: "required",
    });
    expect(get.rpId).toBe("app.fctl.sh");
    expect(get.allowCredentials).toEqual([]);
  });

  it("says nothing when the person closed the sheet", () => {
    expect(cancelled(new DOMException("x", "NotAllowedError"))).toBe(true);
    expect(cancelled(new DOMException("x", "AbortError"))).toBe(true);
    expect(cancelled(new DOMException("x", "InvalidStateError"))).toBe(false);
    expect(cancelled(new Error("x"))).toBe(false);
  });
});

describe("the account's details", () => {
  const ada = userHandle("usr_1");
  const signal = vi.fn((_: unknown) => Promise.resolve());
  const details = (userId: string, name: string, displayName: string) =>
    ({ rpId: "app.fctl.sh", userId, name, displayName }) as const;

  beforeEach(() => {
    signal.mockReset().mockResolvedValue(undefined);
    vi.stubGlobal(
      "PublicKeyCredential",
      Object.assign(() => {}, { signalCurrentUserDetails: signal }),
    );
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    localStorage.clear();
  });

  it("go to the passkey provider once, and again when they change", () => {
    for (let i = 0; i < 3; i++) signalDetails("app.fctl.sh", ada, "ada@example.com", "Ada");
    expect(signal.mock.calls).toEqual([[details(ada, "ada@example.com", "Ada")]]);
    signalDetails("app.fctl.sh", ada, "ada@example.com", "Ada Lovelace");
    signalDetails("app.fctl.sh", ada, "ada@lovelace.dev", "Ada Lovelace");
    signalDetails("app.fctl.sh", ada, "ada@lovelace.dev", "Ada Lovelace");
    expect(signal).toHaveBeenCalledTimes(3);
    expect(signal).toHaveBeenLastCalledWith(details(ada, "ada@lovelace.dev", "Ada Lovelace"));
  });

  it("go once for each account", () => {
    const grace = userHandle("usr_2");
    signalDetails("app.fctl.sh", ada, "ada@example.com", "Ada");
    signalDetails("app.fctl.sh", grace, "grace@example.com", "Grace");
    signalDetails("app.fctl.sh", ada, "ada@example.com", "Ada");
    expect(signal.mock.calls).toEqual([
      [details(ada, "ada@example.com", "Ada")],
      [details(grace, "grace@example.com", "Grace")],
    ]);
  });

  it("don't go for a passkey this browser made with them", async () => {
    const credential = {
      id: "AQ",
      rawId: new Uint8Array([1]).buffer,
      type: "public-key",
      authenticatorAttachment: "platform",
      response: {
        clientDataJSON: new Uint8Array([2]).buffer,
        attestationObject: new Uint8Array([3]).buffer,
        getTransports: () => ["internal"],
      },
    };
    vi.spyOn(navigator, "credentials", "get").mockReturnValue({
      create: async () => credential,
    } as unknown as CredentialsContainer);
    expect((await createPasskey(OPTIONS)).response.transports).toEqual(["internal"]);
    signalDetails("app.fctl.sh", ada, "ada@example.com", "Ada");
    expect(signal).not.toHaveBeenCalled();
    signalDetails("app.fctl.sh", ada, "ada@lovelace.dev", "Ada");
    expect(signal).toHaveBeenCalledTimes(1);
  });

  it("go again when the browser refused them", async () => {
    signal.mockRejectedValueOnce(new DOMException("Not now", "NotAllowedError"));
    signalDetails("app.fctl.sh", ada, "ada@example.com", "Ada");
    await vi.waitFor(() => expect(localStorage.length).toBe(0));
    signalDetails("app.fctl.sh", ada, "ada@example.com", "Ada");
    signalDetails("app.fctl.sh", ada, "ada@example.com", "Ada");
    expect(signal).toHaveBeenCalledTimes(2);
  });

  it("stay out of the browser's storage", () => {
    signalDetails("app.fctl.sh", ada, "ada@example.com", "Ada Lovelace");
    const kept = Array.from({ length: localStorage.length }, (_, i) => {
      const key = localStorage.key(i) ?? "";
      return `${key}=${localStorage.getItem(key)}`;
    }).join();
    expect(kept).toMatch(/^fctl:passkey-details:/);
    expect(kept).not.toMatch(/ada|lovelace/i);
  });

  it("are neither given nor kept where the browser has no signals", () => {
    vi.stubGlobal("PublicKeyCredential", () => {});
    signalDetails("app.fctl.sh", ada, "ada@example.com", "Ada");
    expect(localStorage.length).toBe(0);
  });
});
