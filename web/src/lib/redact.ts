// Takes secrets out of text people may paste into a bug report. The error
// page's "What broke" shows and copies an error's message and stack, and
// those can quote whatever the code was handling: a JSON parse error quotes
// the response it choked on, a URL can carry an invitation's token.

const HIDDEN = "[hidden]";

/** Secrets that say what they are. */
const SECRETS = [
  // fctl's own: fctl_ws_…, fctl_cli_…, fctl_pat_…, fctl_rt_…, fctl_inv_…, fctl_rst_…, fctl_dc_…, fctl_n_….
  // Even a few characters go: a JSON parse error quotes the first ten of the text.
  /\bfctl_[a-z]{1,4}_[0-9A-Za-z]+/g,
  // Claude's tokens (sk-ant-oat01-…, sk-ant-ort01-…) and API keys (sk-ant-api03-…).
  /\bsk-ant-[0-9A-Za-z_-]+/g,
  // GitHub's tokens: ghp_, gho_, ghu_, ghs_, ghr_, github_pat_.
  /\b(?:gh[pousr]_|github_pat_)[0-9A-Za-z_]+/g,
  // JSON Web Tokens.
  /\beyJ[0-9A-Za-z_-]{6,}\.[0-9A-Za-z_-]{6,}\.[0-9A-Za-z_-]{6,}/g,
  // Email addresses.
  /[\w.+-]+@[\w-]+(?:\.[\w-]+)*\.[A-Za-z]{2,}/g,
];

/** A value after a name that says it is secret: token=…, "password": "…", Authorization: Bearer …. */
const NAMED =
  /\b(token|secret|password|passwd|code|state|key|auth|authorization|cookie|credential|bearer)(s?["']?\s*[:=]\s*["']?(?:bearer\s+)?)([^\s"'&,;)}\]]+)/gi;

/** URLs: their user and password, query and fragment go; a stack frame's :line:column stays. */
const URLS = /\b(https?:\/\/)(?:[^\s/?#@]+@)?([^\s/?#)]+)([^\s?#)]*)([?#][^\s)]*)?/g;

/** `text` without anything that looks like a secret or an email address. */
export function redact(text: string): string {
  let out = text.replace(URLS, (_m, scheme: string, host: string, path: string, rest?: string) => {
    const position = rest?.match(/(:\d+:\d+)$/)?.[1] ?? "";
    return `${scheme}${host}${path}${position}`;
  });
  out = out.replace(NAMED, (_m, name: string, between: string) => `${name}${between}${HIDDEN}`);
  for (const pattern of SECRETS) out = out.replace(pattern, HIDDEN);
  return out;
}

/** An address to quote: no query or fragment, where tokens travel (/invite#token=…). */
export function safeAddress(href: string): string {
  try {
    const url = new URL(href);
    return `${url.origin}${url.pathname}`;
  } catch {
    return "";
  }
}
