// What the sign-in page says when a Google sign-in didn't sign someone in: the control plane's callback
// sends them back to /login with an error (crates/fctl-control/src/api/google.rs), and each says what to
// do next.

const GOOGLE_ERRORS: Record<string, string> = {
  google_expired: "The Google sign-in took too long. Try again.",
  google_state: "The Google sign-in could not be verified. Try again.",
  google_cancelled: "Google sign-in was cancelled.",
  google_exchange: "Google did not confirm the sign-in. Try again.",
  google_invalid: "Google returned an invalid sign-in.",
  google_unverified: "Your Google account's email is not verified.",
  // The account with this address signs in another way: that way first, then Settings links Google.
  google_link_required:
    "An account with this email exists. Sign in with your password, then link Google in Settings.",
  google_link_passkey:
    "An account with this email exists. Sign in with your passkey, then link Google in Settings.",
  google_link_other:
    "Another Google account signs in to the account with this email. Sign in with that one, then link this one in Settings.",
  not_allowed: "This email is not allowed on this instance. Ask an admin for access.",
  disabled: "Your account is disabled. Ask an admin for access.",
};

/**
 * The sign-in page's words for the error `code` a Google sign-in came back with. `email`: the instance
 * sends email, so Forgot? sends a link to choose a password.
 */
export function googleError(code: string, email: boolean): string {
  if (code === "google_not_authoritative") {
    // Someone whose account an admin made, with an address Google doesn't host, chooses a password.
    return `Google can't vouch for this address, as it doesn't host it: sign up with the invitation sent to it, or with a Gmail or Google Workspace account. If an admin made your account, ${
      email ? "choose a password with Forgot? below." : "ask them for a link to choose a password."
    }`;
  }
  // Only its own keys: ?error=constructor is no message.
  return (
    (Object.hasOwn(GOOGLE_ERRORS, code) && GOOGLE_ERRORS[code]) || "The sign-in did not work. Try again."
  );
}
