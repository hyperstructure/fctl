import { describe, expect, it } from "vitest";
import { googleError } from "./google-sign-in";

describe("what the sign-in page says when Google didn't sign someone in", () => {
  it("names the way in of an account that signs in another way", () => {
    expect(googleError("google_link_required", true)).toContain("Sign in with your password");
    expect(googleError("google_link_passkey", true)).toContain("Sign in with your passkey");
    expect(googleError("google_link_other", true)).toContain("Another Google account signs in");
    // None tells someone without a password to sign in with one.
    expect(googleError("google_link_passkey", true)).not.toContain("password");
    expect(googleError("google_link_other", true)).not.toContain("password");
  });

  it("tells someone an admin made an account for, at an address Google doesn't host, how to choose a password", () => {
    expect(googleError("google_not_authoritative", true)).toContain(
      "If an admin made your account, choose a password with Forgot? below.",
    );
    // Without email, Forgot? sends nothing: an admin makes the link.
    expect(googleError("google_not_authoritative", false)).toContain(
      "If an admin made your account, ask them for a link to choose a password.",
    );
  });

  it("says the sign-in did not work for an error it doesn't know", () => {
    expect(googleError("google_new_thing", true)).toBe("The sign-in did not work. Try again.");
    expect(googleError("constructor", true)).toBe("The sign-in did not work. Try again.");
  });
});
