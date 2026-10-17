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
    /// macOS starts login shells, which read ~/.bash_profile.
    pub fn rc_path(&self, shell: Shell) -> Option<PathBuf> {
        match shell {
            Shell::Zsh => Some(self.zdotdir.join(".zshrc")),
            Shell::Bash if self.macos => Some(self.home.join(".bash_profile")),
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
            Shell::Bash => Some(format!(
                "\n{MARKER}\n[ -f \"{script}\" ] && . \"{script}\"\n"
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
    let mut rc_changed = None;
    if let (Some(rc), Some(block)) = (places.rc_path(shell), places.rc_block(shell)) {
        let current = std::fs::read_to_string(&rc).unwrap_or_default();
        if !current.contains(MARKER) {
            let mut text = current;
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&block);
            std::fs::write(&rc, text).with_context(|| format!("writing {}", rc.display()))?;
            rc_changed = Some(rc);
        }
    }
    Ok(Installed {
        shell,
        script,
        rc_changed,
    })
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
    fn bash_uses_the_login_profile_on_macos() -> Result<()> {
        let home = tempfile::tempdir()?;
        let mac = install(&places(home.path(), true), Shell::Bash, b"complete")?;
        assert_eq!(mac.rc_changed, Some(home.path().join(".bash_profile")));
        let linux = install(&places(home.path(), false), Shell::Bash, b"complete")?;
        assert_eq!(linux.rc_changed, Some(home.path().join(".bashrc")));
        let rc = std::fs::read_to_string(home.path().join(".bashrc"))?;
        assert!(rc.contains(
            "[ -f \"$HOME/.local/share/fctl/completions/fctl.bash\" ] && . \"$HOME/.local/share/fctl/completions/fctl.bash\""
        ));
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
