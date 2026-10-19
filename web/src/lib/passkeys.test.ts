import { describe, expect, it } from "vitest";
import { cancelled, creationOptions, fromB64url, requestOptions, toB64url, userHandle } from "./passkeys";

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
    const create = creationOptions({
      rp: { id: "app.fctl.sh", name: "fctl" },
      user: { id: userHandle("usr_1"), name: "ada@example.com", displayName: "Ada" },
      challenge: toB64url(new Uint8Array([9, 9])),
      pubKeyCredParams: [{ type: "public-key", alg: -7 }],
      timeout: 300000,
      excludeCredentials: [
        { type: "public-key", id: toB64url(new Uint8Array([1])), transports: ["internal"] },
      ],
      authenticatorSelection: {
        residentKey: "required",
        requireResidentKey: true,
        userVerification: "required",
      },
      attestation: "none",
    });
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
