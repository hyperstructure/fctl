// Two-factor sign-in: the pure parts of the pages that set it up and ask for
// codes. Recovery codes use the same characters as the server's
// (fctl_domain::two_factor): no vowels, no look-alikes.

const RECOVERY_ALPHABET = "BCDFGHJKMNPQRSTVWXZ2";
const RECOVERY_LEN = 10;
/** Digits in a code from an authenticator app. */
export const CODE_LENGTH = 6;
/** Unused recovery codes at or below which Settings suggests new ones. */
export const FEW_RECOVERY_CODES = 3;
/** sessionStorage: a recovery code signed this browser in, and this many are left (the app says so once). */
export const RECOVERY_LEFT = "fctl:recovery-codes-left";

/** An authenticator app's key in groups of four, for typing it in: `JBSW Y3DP EHPK 3PXP`. */
export function groupKey(secret: string): string {
  return secret.replace(/\s+/g, "").replace(/(.{4})(?=.)/g, "$1 ");
}

/** The digits of what someone typed or pasted into a code field, at most `n`. */
export function digits(typed: string, n = CODE_LENGTH): string {
  return typed.replace(/\D/g, "").slice(0, n);
}

/**
 * A recovery code as someone types it: capitals, only the characters codes
 * use, and the dash after the fifth (`kftwqx` → `KFTWQ-X`).
 */
export function formatRecoveryCode(typed: string): string {
  const chars = typed
    .toUpperCase()
    .split("")
    .filter((c) => RECOVERY_ALPHABET.includes(c))
    .slice(0, RECOVERY_LEN)
    .join("");
  return chars.length > 5 ? `${chars.slice(0, 5)}-${chars.slice(5)}` : chars;
}

/** Whether a typed recovery code is complete. */
export function isRecoveryCode(typed: string): boolean {
  return formatRecoveryCode(typed).replace("-", "").length === RECOVERY_LEN;
}

/** The recovery codes as a file to keep: who they are for, what they do, one a line. */
export function recoveryCodesText(codes: string[], instance: string, email: string, now: Date): string {
  return [
    `${instance} recovery codes for ${email}`,
    `Made on ${now.toISOString().slice(0, 10)}.`,
    "",
    "Each code signs you in once, in place of a code from your authenticator app.",
    "Keep them somewhere safe, such as your password manager. New ones replace them all.",
    "",
    ...codes,
    "",
  ].join("\n");
}

/** `fctl-recovery-codes.txt`, or with the instance's name when it has its own. */
export function recoveryCodesFile(instance: string): string {
  const slug = instance
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");
  return `${slug || "fctl"}-recovery-codes.txt`;
}

/** A wait as a clock: `0:47`, `2:05`, `1:00:00`. */
export function clock(ms: number): string {
  const total = Math.max(0, Math.ceil(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = String(total % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${s}` : `${m}:${s}`;
}

/**
 * A QR code's dark modules as SVG paths, one per ring around the center
 * (its distance in modules), so that it can appear from the middle out.
 * `margin` modules of quiet zone go around it.
 */
export function qrRings(rows: string[], margin = 4): { ring: number; d: string }[] {
  const center = (rows.length - 1) / 2;
  const rings = new Map<number, string[]>();
  rows.forEach((row, y) => {
    for (let x = 0; x < row.length; x++) {
      if (row[x] !== "1") continue;
      const ring = Math.round(Math.max(Math.abs(x - center), Math.abs(y - center)));
      const parts = rings.get(ring) ?? [];
      parts.push(`M${x + margin} ${y + margin}h1v1h-1z`);
      rings.set(ring, parts);
    }
  });
  return [...rings.entries()].sort(([a], [b]) => a - b).map(([ring, parts]) => ({ ring, d: parts.join("") }));
}

/** Whether this device is a phone or tablet, where the app opens from a link rather than a scan. */
export function touchDevice(): boolean {
  return typeof matchMedia === "function" && matchMedia("(pointer: coarse)").matches;
}
