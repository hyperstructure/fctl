// What the sign-in page says when a Google sign-in didn't sign someone in: the control plane's callback
// sends them back to /login with an error (crates/fctl-control/src/api/google.rs and api/sign_in.rs), and
// each says what to do next.

import { domainsWords } from "./sign-in";

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
  // A domain's rule made Google the way in for this address, and that Google account signs in to another account.
  google_linked_elsewhere:
    "That Google account signs in to another account here. Choose the Google account of the address you typed.",
  not_allowed: "This email is not allowed on this instance. Ask an admin for access.",
  // Accounts only from the directory (InstanceSettings.self_sign_up off).
  sign_up_closed:
    "Your organization makes the accounts here, and there's none for this address yet. Ask an admin of this instance for one.",
  disabled: "Your account is disabled. Ask an admin for access.",
};

/**
 * The sign-in page's words for the error `code` a Google sign-in came back with. `email`: the instance
 * sends email, so Forgot? sends a link to choose a password. `domains`: for an account whose domain signs
 * in with Google, the domains of its addresses that do (`?domains=acme.com,acme.dev`).
 */
export function googleError(code: string, email: boolean, domains?: string, provider = "Google"): string {
  if (code === "google_wrong_account") {
    // Their own Gmail, say, where their organization's domain signs in with its Google Workspace.
    const list = (domains ?? "").split(",").filter(Boolean);
    const at = list.length > 0 ? domainsWords(list, "or") : "your organization's";
    if (provider !== "Google")
      return `Your account signs in with ${provider} as your ${at} address, not with Google. Continue with ${provider} instead.`;
    return `Your account signs in with Google as your ${at} address. Choose that Google account when Google asks: the one you chose isn't one.`;
  }
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
