//! `fctl completions`: print shell completions, or set them up.
//!
//! `--install` writes the script for your shell under your home directory
//! and, for zsh and bash, adds a marked block to the shell's startup file
//! once (fish loads `~/.config/fish/completions` by itself). The installer
//! runs it, and `fctl self update` refreshes the scripts with `--refresh`.

use anyhow::{Context, Result, bail};
use clap_complete::Shell;
use std::path::{Path, PathBuf};

/// Marks the block `--install` adds to a startup file.
const MARKER: &str = "# fctl tab completion (added by `fctl completions --install`)";

/// Writes the completion script for `shell` into `out`.
pub fn generate(shell: Shell, out: &mut dyn std::io::Write) {
    use clap::CommandFactory;
    let mut cmd = crate::Cli::command();
    clap_complete::generate(shell, &mut cmd, "fctl", out);
}

fn script(shell: Shell) -> Vec<u8> {
    let mut buf = Vec::new();
    generate(shell, &mut buf);
    buf
}

/// Where things go for a user, from their environment.
pub struct Places {
    pub home: PathBuf,
    /// `$XDG_DATA_HOME` or `~/.local/share`.
    pub data: PathBuf,
    /// `$XDG_CONFIG_HOME` or `~/.config`.
    pub config: PathBuf,
    /// `$ZDOTDIR` or the home directory.
    pub zdotdir: PathBuf,
    pub macos: bool,
}

impl Places {
    pub fn from_env() -> Result<Self> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .context("HOME is not set")?;
        let var = |k: &str| {
            std::env::var_os(k)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
        };
        Ok(Self {
            data: var("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local/share")),
            config: var("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config")),
            zdotdir: var("ZDOTDIR").unwrap_or_else(|| home.clone()),
            macos: cfg!(target_os = "macos"),
            home,
        })
    }

    fn scripts_dir(&self) -> PathBuf {
        self.data.join("fctl/completions")
    }

    /// The completion script for `shell`.
    pub fn script_path(&self, shell: Shell) -> Option<PathBuf> {
        match shell {
            Shell::Zsh => Some(self.scripts_dir().join("_fctl")),
            Shell::Bash => Some(self.scripts_dir().join("fctl.bash")),
            Shell::Fish => Some(self.config.join("fish/completions/fctl.fish")),
            _ => None,
        }
    }

    /// The startup file that loads the script (fish needs none). Bash on
    /// macOS starts login shells, which read only the first of
    /// ~/.bash_profile, ~/.bash_login and ~/.profile that exists: the block
    /// goes in that one, or in a new ~/.profile, which hides nothing.
    pub fn rc_path(&self, shell: Shell) -> Option<PathBuf> {
        match shell {
            Shell::Zsh => Some(self.zdotdir.join(".zshrc")),
            Shell::Bash if self.macos => Some(
                [".bash_profile", ".bash_login", ".profile"]
                    .into_iter()
                    .map(|f| self.home.join(f))
                    .find(|p| p.exists())
                    .unwrap_or_else(|| self.home.join(".profile")),
            ),
            Shell::Bash => Some(self.home.join(".bashrc")),
            _ => None,
        }
    }

    /// A path for a startup file: under the home directory as `$HOME/…`.
    fn shell_path(&self, p: &Path) -> String {
        match p.strip_prefix(&self.home) {
            Ok(rest) => format!("$HOME/{}", rest.display()),
            Err(_) => p.display().to_string(),
        }
    }

    /// The lines `--install` adds to the startup file.
    pub fn rc_block(&self, shell: Shell) -> Option<String> {
        let script = self.shell_path(&self.script_path(shell)?);
        match shell {
            Shell::Zsh => {
                let dir = self.shell_path(&self.scripts_dir());
                Some(format!(
                    "\n{MARKER}\nfpath=(\"{dir}\" $fpath)\nautoload -Uz compinit && compinit\n"
                ))
            }
            // sh, dash and ksh read ~/.profile too, and stop at bash syntax.
            Shell::Bash => Some(format!(
                "\n{MARKER}\nif [ -n \"${{BASH_VERSION:-}}\" ] && [ -f \"{script}\" ]; then\n  . \"{script}\"\nfi\n"
            )),
            _ => None,
        }
    }
}

/// The shell from `$SHELL` (`/bin/zsh` → zsh).
pub fn login_shell() -> Option<Shell> {
    let path = std::env::var("SHELL").ok()?;
    let name = Path::new(&path).file_name()?.to_str()?;
    name.parse().ok()
}

/// What `--install` did, for the message.
pub struct Installed {
    pub shell: Shell,
    pub script: PathBuf,
    /// The startup file it added the block to (not when it was there).
    pub rc_changed: Option<PathBuf>,
}

/// Writes the script and adds the startup block once.
pub fn install(places: &Places, shell: Shell, script_bytes: &[u8]) -> Result<Installed> {
    let Some(script) = places.script_path(shell) else {
        bail!(
            "--install supports zsh, bash and fish; for {shell}, load the output of `fctl completions {shell}` the way your shell does"
        );
    };
    if let Some(dir) = script.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(&script, script_bytes)
        .with_context(|| format!("writing {}", script.display()))?;
    let rc_changed = match (places.rc_path(shell), places.rc_block(shell)) {
        (Some(rc), Some(block)) => add_block(&rc, &block)?.then_some(rc),
        _ => None,
    };
    Ok(Installed {
        shell,
        script,
        rc_changed,
    })
}

/// Appends `block` to the end of the startup file `rc` unless it is there.
fn add_block(rc: &Path, block: &str) -> Result<bool> {
    use std::io::Write;
    // Bytes: a startup file need not be UTF-8, and one we cannot read is
    // an error, never an empty file to write over.
    let current = match std::fs::read(rc) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", rc.display())),
    };
    if String::from_utf8_lossy(&current).contains(MARKER) {
        return Ok(false);
    }
    let mut text = Vec::new();
    if !current.is_empty() && !current.ends_with(b"\n") {
        text.push(b'\n');
    }
    text.extend_from_slice(block.as_bytes());
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(rc)
        .and_then(|mut f| f.write_all(&text))
        .with_context(|| format!("writing {}", rc.display()))?;
    Ok(true)
}

/// Rewrites the scripts that are installed (no startup files change).
pub fn refresh(places: &Places, scripts: impl Fn(Shell) -> Vec<u8>) -> Result<Vec<Shell>> {
    let mut done = Vec::new();
    for shell in [Shell::Zsh, Shell::Bash, Shell::Fish] {
        if let Some(p) = places.script_path(shell)
            && p.is_file()
        {
            std::fs::write(&p, scripts(shell))
                .with_context(|| format!("writing {}", p.display()))?;
            done.push(shell);
        }
    }
    Ok(done)
}

pub fn run(shell: Option<Shell>, install_it: bool, refresh_them: bool) -> Result<()> {
    if refresh_them {
        let places = Places::from_env()?;
        for shell in refresh(&places, script)? {
            println!("Refreshed the {shell} completions");
        }
        return Ok(());
    }
    if install_it {
        let shell = shell.or_else(login_shell).context(
            "could not tell your shell from $SHELL; name it: fctl completions zsh --install",
        )?;
        let places = Places::from_env()?;
        let done = install(&places, shell, &script(shell))?;
        println!(
            "Installed {} completions: {}",
            done.shell,
            places.shell_path(&done.script).replace("$HOME", "~")
        );
        if let Some(rc) = done.rc_changed {
            println!(
                "Added a few lines to {} to load them. Open a new terminal to use them.",
                places.shell_path(&rc).replace("$HOME", "~")
            );
        }
        return Ok(());
    }
    let Some(shell) = shell else {
        bail!(
            "name a shell (fctl completions zsh), or set them up with: fctl completions --install"
        );
    };
    generate(shell, &mut std::io::stdout());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn places(home: &Path, macos: bool) -> Places {
        Places {
            home: home.to_owned(),
            data: home.join(".local/share"),
            config: home.join(".config"),
            zdotdir: home.to_owned(),
            macos,
        }
    }

    #[test]
    fn zsh_install_adds_the_block_once() -> Result<()> {
        let home = tempfile::tempdir()?;
        let p = places(home.path(), true);
        std::fs::write(home.path().join(".zshrc"), "export EDITOR=vi")?;
        let first = install(&p, Shell::Zsh, b"#compdef fctl\n")?;
        assert_eq!(
            first.script,
            home.path().join(".local/share/fctl/completions/_fctl")
        );
        assert_eq!(first.rc_changed, Some(home.path().join(".zshrc")));
        let second = install(&p, Shell::Zsh, b"#compdef fctl\n")?;
        assert_eq!(second.rc_changed, None);
        let rc = std::fs::read_to_string(home.path().join(".zshrc"))?;
        assert!(rc.starts_with("export EDITOR=vi\n"));
        assert_eq!(rc.matches(MARKER).count(), 1);
        assert!(rc.contains("fpath=(\"$HOME/.local/share/fctl/completions\" $fpath)"));
        assert!(rc.contains("autoload -Uz compinit && compinit"));
        Ok(())
    }

    #[test]
    fn bash_on_macos_adds_to_the_file_login_shells_read() -> Result<()> {
        // A ~/.profile and no ~/.bash_profile: creating ~/.bash_profile
        // would stop bash from reading ~/.profile.
        let home = tempfile::tempdir()?;
        let p = places(home.path(), true);
        std::fs::write(
            home.path().join(".profile"),
            "export PATH=\"$HOME/bin:$PATH\"",
        )?;
        let done = install(&p, Shell::Bash, b"complete")?;
        assert_eq!(done.rc_changed, Some(home.path().join(".profile")));
        assert!(!home.path().join(".bash_profile").exists());
        let rc = std::fs::read_to_string(home.path().join(".profile"))?;
        assert!(rc.starts_with("export PATH=\"$HOME/bin:$PATH\"\n\n# fctl tab completion"));
        // Bash reads ~/.bash_profile first, then ~/.bash_login.
        std::fs::write(home.path().join(".bash_login"), "")?;
        assert_eq!(
            p.rc_path(Shell::Bash),
            Some(home.path().join(".bash_login"))
        );
        std::fs::write(home.path().join(".bash_profile"), "")?;
        assert_eq!(
            p.rc_path(Shell::Bash),
            Some(home.path().join(".bash_profile"))
        );
        // With none of them, a new ~/.profile hides nothing.
        let empty = tempfile::tempdir()?;
        let done = install(&places(empty.path(), true), Shell::Bash, b"complete")?;
        assert_eq!(done.rc_changed, Some(empty.path().join(".profile")));
        assert!(!empty.path().join(".bash_profile").exists());
        Ok(())
    }

    #[test]
    fn bash_on_linux_uses_bashrc_and_only_bash_loads_the_script() -> Result<()> {
        let home = tempfile::tempdir()?;
        let done = install(&places(home.path(), false), Shell::Bash, b"complete")?;
        assert_eq!(done.rc_changed, Some(home.path().join(".bashrc")));
        let rc = std::fs::read_to_string(home.path().join(".bashrc"))?;
        let script = "$HOME/.local/share/fctl/completions/fctl.bash";
        assert!(rc.contains(&format!(
            "if [ -n \"${{BASH_VERSION:-}}\" ] && [ -f \"{script}\" ]; then\n  . \"{script}\"\nfi\n"
        )));
        Ok(())
    }

    #[test]
    fn install_appends_to_files_that_are_not_utf8() -> Result<()> {
        let home = tempfile::tempdir()?;
        let rc = home.path().join(".bashrc");
        std::fs::write(&rc, b"# caf\xe9\nexport A=1")?;
        install(&places(home.path(), false), Shell::Bash, b"complete")?;
        let text = std::fs::read(&rc)?;
        assert!(text.starts_with(b"# caf\xe9\nexport A=1\n\n# fctl tab completion"));
        Ok(())
    }

    #[test]
    fn fish_needs_no_startup_file() -> Result<()> {
        let home = tempfile::tempdir()?;
        let done = install(
            &places(home.path(), false),
            Shell::Fish,
            b"complete -c fctl",
        )?;
        assert_eq!(
            done.script,
            home.path().join(".config/fish/completions/fctl.fish")
        );
        assert_eq!(done.rc_changed, None);
        assert!(install(&places(home.path(), false), Shell::Elvish, b"").is_err());
        Ok(())
    }

    #[test]
    fn refresh_rewrites_only_installed_scripts() -> Result<()> {
        let home = tempfile::tempdir()?;
        let p = places(home.path(), false);
        install(&p, Shell::Fish, b"old")?;
        let done = refresh(&p, |s| format!("new {s}").into_bytes())?;
        assert_eq!(done, vec![Shell::Fish]);
        assert_eq!(
            std::fs::read_to_string(home.path().join(".config/fish/completions/fctl.fish"))?,
            "new fish"
        );
        assert!(
            !home
                .path()
                .join(".local/share/fctl/completions/_fctl")
                .exists()
        );
        Ok(())
    }

    #[test]
    fn generated_scripts_name_fctl() {
        let zsh = String::from_utf8(script(Shell::Zsh)).unwrap_or_default();
        assert!(zsh.starts_with("#compdef fctl"));
        assert!(zsh.contains("self"));
    }
}
