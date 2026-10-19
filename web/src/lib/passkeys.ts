// Passkeys in the browser: WebAuthn between the server's options and answers
// (WebAuthn's JSON forms, in src/gen) and the browser's binary API, which
// every browser takes, also those without parseCreationOptionsFromJSON.

import type * as G from "@/gen";

/** sessionStorage: this browser just signed in without a passkey, so the app offers to make one. */
export const OFFER_PASSKEY = "fctl:offer-passkey";

/** base64url (without padding) as bytes. */
export function fromB64url(text: string): Uint8Array<ArrayBuffer> {
  const base64 = text.replace(/-/g, "+").replace(/_/g, "/");
  const padded = base64 + "=".repeat((4 - (base64.length % 4)) % 4);
  const binary = atob(padded);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

/** Bytes as base64url without padding. */
export function toB64url(bytes: ArrayBuffer | Uint8Array): string {
  const view = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  let binary = "";
  for (const b of view) binary += String.fromCharCode(b);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

type PKC = typeof PublicKeyCredential & {
  isConditionalMediationAvailable?: () => Promise<boolean>;
  signalUnknownCredential?: (o: { rpId: string; credentialId: string }) => Promise<void>;
  signalAllAcceptedCredentials?: (o: {
    rpId: string;
    userId: string;
    allAcceptedCredentialIds: string[];
  }) => Promise<void>;
  signalCurrentUserDetails?: (o: {
    rpId: string;
    userId: string;
    name: string;
    displayName: string;
  }) => Promise<void>;
};

function pkc(): PKC | undefined {
  return typeof PublicKeyCredential === "function" ? (PublicKeyCredential as PKC) : undefined;
}

/** What this browser does with passkeys. */
export interface PasskeySupport {
  /** It makes and uses passkeys. */
  supported: boolean;
  /** It offers them in the email field's suggestions (conditional mediation). */
  autofill: boolean;
  /** This device has its own: a fingerprint, face or screen lock. */
  platform: boolean;
}

export async function passkeySupport(): Promise<PasskeySupport> {
  const p = pkc();
  if (!p || !navigator.credentials?.create || !window.isSecureContext)
    return { supported: false, autofill: false, platform: false };
  const [autofill, platform] = await Promise.all([
    p.isConditionalMediationAvailable?.().catch(() => false) ?? false,
    p.isUserVerifyingPlatformAuthenticatorAvailable?.().catch(() => false) ?? false,
  ]);
  return { supported: true, autofill, platform };
}

const descriptor = (d: G.PasskeyDescriptor): PublicKeyCredentialDescriptor => ({
  type: "public-key",
  id: fromB64url(d.id),
  transports: d.transports as AuthenticatorTransport[] | undefined,
});

/** The server's options for making a passkey, as `navigator.credentials.create` takes them. */
export function creationOptions(o: G.PasskeyCreationOptions): PublicKeyCredentialCreationOptions {
  return {
    rp: o.rp,
    user: { id: fromB64url(o.user.id), name: o.user.name, displayName: o.user.displayName },
    challenge: fromB64url(o.challenge),
    pubKeyCredParams: o.pubKeyCredParams.map((p) => ({ type: "public-key", alg: p.alg })),
    timeout: o.timeout,
    excludeCredentials: o.excludeCredentials.map(descriptor),
    authenticatorSelection: {
      residentKey: o.authenticatorSelection.residentKey as ResidentKeyRequirement,
      requireResidentKey: o.authenticatorSelection.requireResidentKey,
      userVerification: o.authenticatorSelection.userVerification as UserVerificationRequirement,
    },
    attestation: o.attestation as AttestationConveyancePreference,
  };
}

/** The server's options for signing in, as `navigator.credentials.get` takes them. */
export function requestOptions(o: G.PasskeyRequestOptions): PublicKeyCredentialRequestOptions {
  return {
    challenge: fromB64url(o.challenge),
    rpId: o.rpId,
    timeout: o.timeout,
    allowCredentials: o.allowCredentials.map(descriptor),
    userVerification: o.userVerification as UserVerificationRequirement,
  };
}

/** Makes a passkey with the server's options: the browser asks for a fingerprint, face or screen lock. */
export async function createPasskey(o: G.PasskeyCreationOptions): Promise<G.PasskeyRegistration> {
  const credential = (await navigator.credentials.create({
    publicKey: creationOptions(o),
  })) as PublicKeyCredential | null;
  if (!credential) throw new DOMException("No passkey was made", "NotAllowedError");
  const response = credential.response as AuthenticatorAttestationResponse;
  return {
    id: credential.id,
    rawId: toB64url(credential.rawId),
    type: credential.type,
    authenticatorAttachment: credential.authenticatorAttachment ?? undefined,
    response: {
      clientDataJSON: toB64url(response.clientDataJSON),
      attestationObject: toB64url(response.attestationObject),
      transports: response.getTransports?.(),
    },
  };
}

/**
 * Signs in with a passkey. `conditional` offers the passkeys in the email
 * field's suggestions instead of a sheet, until `signal` aborts.
 */
export async function getPasskey(
  o: G.PasskeyRequestOptions,
  { conditional, signal }: { conditional?: boolean; signal?: AbortSignal } = {},
): Promise<G.PasskeyAssertion> {
  const credential = (await navigator.credentials.get({
    publicKey: requestOptions(o),
    mediation: conditional ? "conditional" : undefined,
    signal,
  })) as PublicKeyCredential | null;
  if (!credential) throw new DOMException("No passkey answered", "NotAllowedError");
  const response = credential.response as AuthenticatorAssertionResponse;
  return {
    id: credential.id,
    rawId: toB64url(credential.rawId),
    type: credential.type,
    authenticatorAttachment: credential.authenticatorAttachment ?? undefined,
    response: {
      clientDataJSON: toB64url(response.clientDataJSON),
      authenticatorData: toB64url(response.authenticatorData),
      signature: toB64url(response.signature),
      userHandle: response.userHandle ? toB64url(response.userHandle) : undefined,
    },
  };
}

/** The person closed the browser's passkey sheet, or its time ran out: nothing to say about it. */
export function cancelled(e: unknown): boolean {
  return e instanceof DOMException && (e.name === "NotAllowedError" || e.name === "AbortError");
}

/** What to say when making or using a passkey failed, in words. */
export function passkeyError(e: unknown): string {
  if (e instanceof DOMException) {
    if (e.name === "InvalidStateError") return "This device already has a passkey for your account.";
    if (e.name === "SecurityError")
      return "Passkeys don't work on this address. Open fctl at its usual address.";
    if (e.name === "NotSupportedError") return "This browser can't make passkeys of a kind fctl takes.";
  }
  return e instanceof Error ? e.message : "The passkey didn't work. Try again.";
}

// Signals (WebAuthn Level 3) keep the person's passkey provider in step with
// fctl, in browsers that have them; elsewhere they do nothing.

/** A passkey fctl doesn't know (removed, or for a deleted account): its provider can hide it. */
export function signalUnknown(rpId: string, credentialId: string) {
  pkc()
    ?.signalUnknownCredential?.({ rpId, credentialId })
    .catch(() => {});
}

/** The passkeys an account has: its provider can hide the others it keeps for this site. */
export function signalAccepted(rpId: string, userId: string, credentialIds: string[]) {
  pkc()
    ?.signalAllAcceptedCredentials?.({ rpId, userId, allAcceptedCredentialIds: credentialIds })
    .catch(() => {});
}

/** The account's name and email, which providers list its passkeys by. */
export function signalDetails(rpId: string, userId: string, name: string, displayName: string) {
  pkc()
    ?.signalCurrentUserDetails?.({ rpId, userId, name, displayName })
    .catch(() => {});
}

/** The user handle the server gave this account's passkeys: its id, base64url. */
export function userHandle(userId: string): string {
  return toB64url(new TextEncoder().encode(userId));
}

/** The relying party id passkeys here belong to: this host. */
export function rpId(): string {
  return location.hostname;
}
