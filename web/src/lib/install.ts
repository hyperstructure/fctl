// The commands that install fctl on a machine, as the app shows them, and
// the site they come from.

/**
 * The site beside the app: an app at app.<domain> has its site beside it (fctl.sh beside
 * app.fctl.sh); other instances, like self-hosted ones, have fctl.sh's, whose installer
 * installs the same releases and whose docs are the same.
 */
export function siteUrl(hostname: string = location.hostname): string {
  return hostname.startsWith("app.") ? `https://${hostname.slice(4)}` : "https://fctl.sh";
}

/** The installer's one-liner, from the site beside the app. */
export function installCommand(hostname: string = location.hostname): string {
  return `curl -fsSL ${siteUrl(hostname)}/install | sh`;
}

/** The same release through Homebrew, on macOS and Linux. */
export const BREW_INSTALL = "brew install hyperstructure/tap/fctl";
