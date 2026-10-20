// The commands that install fctl on a machine, as the app shows them.

/**
 * The installer's one-liner. An app at app.<domain> has its site beside it (fctl.sh beside
 * app.fctl.sh); other instances, like self-hosted ones, serve no installer, and fctl.sh's
 * installs the same releases.
 */
export function installCommand(hostname: string = location.hostname): string {
  const site = hostname.startsWith("app.") ? `https://${hostname.slice(4)}` : "https://fctl.sh";
  return `curl -fsSL ${site}/install | sh`;
}

/** The same release through Homebrew, on macOS and Linux. */
export const BREW_INSTALL = "brew install hyperstructure/tap/fctl";
