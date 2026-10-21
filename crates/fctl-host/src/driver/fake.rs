//! A stand-in for the `docker` and `container` CLIs, for tests of the
//! drivers and of what the host does with its images.

use super::container::{ContainerDriver, Flavor};
use super::{Driver, SandboxSpec};
use anyhow::Result;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::{Mutex, PoisonError};

/// The CLI. Each image name on the host is a file in `have`, and each one
/// it can pull a file in `registry`, holding the build's ID, its revision
/// label and Claude Code version (`-` for a build without labels) and the
/// name. `containers` holds the ID of the build each container was made
/// from. Like Docker, `image rm` refuses to remove an image's last name
/// while a container uses it; Apple's `image delete` doesn't check. A
/// `build` lists its context in `context`, copies its Dockerfile to
/// `dockerfile` and makes a build without labels, and with `fail_build` it
/// fails; `size` holds the size images have (1000 bytes without it).
/// `network create` makes `network`, which holds whether its containers
/// reach each other, unless `no_networks` is there.
const SCRIPT: &str = r##"#!/bin/sh
d=$(dirname "$0")
echo "$*" >> "$d/calls"
key() { printf '%s' "$1" | tr '/:' '__'; }
for last in "$@"; do :; done
file() { printf '%s/have/%s' "$d" "$(key "$1")"; }
here() { [ -e "$(file "$1")" ] || { echo "Error: No such image: $1" >&2; exit 1; }; }
id_of() { cut -d' ' -f1 "$(file "$1")"; }
labels() {
  if [ "$1" = - ]; then echo null; else
    printf '{"org.opencontainers.image.revision": "%s", "sh.fctl.claude-code.version": "%s"}' "$1" "$2"
  fi
}
size() { cat "$d/size" 2>/dev/null || echo 1000; }
inspect() {
  set -- $(cat "$(file "$1")")
  printf '[{"Id": "%s", "RepoTags": ["%s"], "Config": {"Labels": %s},' "$1" "$4" "$(labels "$2" "$3")"
  printf ' "variants": [{"platform": {"os": "linux", "architecture": "amd64"}, "size": %s}]}]\n' "$(size)"
}
pull() {
  [ -e "$d/registry/$(key "$last")" ] || { echo "$last: manifest unknown" >&2; exit 1; }
  cp "$d/registry/$(key "$last")" "$(file "$last")"
}
names() { for f in "$d"/have/*; do [ -e "$f" ] && cut -d' ' -f4 "$f"; done; }
count() {
  n=0
  for f in "$d"/have/*; do [ -e "$f" ] && [ "$(cut -d' ' -f1 "$f")" = "$1" ] && n=$((n + 1)); done
  echo "$n"
}
used() {
  for c in "$d"/containers/*; do [ -e "$c" ] && [ "$(cat "$c")" = "$1" ] && return 0; done
  return 1
}
case "$1 $2" in
  "image inspect")
    here "$last"
    case "$3 $4" in
      "--format {{.Size}}") size ;;
      --format*) id_of "$last" ;;
      *) inspect "$last" ;;
    esac ;;
  "image pull" | "pull "*) pull ;;
  "image tag")
    here "$3"
    set -- $(cat "$(file "$3")") "$4"
    echo "$1 $2 $3 $5" > "$(file "$5")" ;;
  "image rm" | "image delete")
    here "$3"
    id=$(id_of "$3")
    if [ "$2" = rm ] && [ "$(count "$id")" = 1 ] && used "$id"; then
      echo "Error response from daemon: conflict: unable to remove repository reference \"$3\" (must force) - container is using its referenced image $id" >&2
      exit 1
    fi
    rm "$(file "$3")"
    echo "Untagged: $3" ;;
  "image ls") names | while read -r n; do case "$n" in "$last":*) echo "$n" ;; esac; done ;;
  "image list") names ;;
  "build "*)
    [ ! -e "$d/fail_build" ] || { echo "#3 ERROR: process did not complete successfully: exit code: 1" >&2; exit 1; }
    echo "#1 [internal] load build definition from Dockerfile"
    echo "#5 [1/2] FROM debian" >&2
    echo "#5 0.1 noise"
    (cd "$last" && find . | sort) > "$d/context"
    cat "$5" > "$d/dockerfile"
    echo "x" >> "$d/builds"
    echo "sha256:build$(wc -l < "$d/builds" | tr -d ' ') - - $3" > "$(file "$3")" ;;
  "network inspect")
    [ -e "$d/network" ] || { echo "Error response from daemon: network $last not found" >&2; exit 1; }
    cat "$d/network" ;;
  "network create")
    [ ! -e "$d/no_networks" ] || { echo "Error response from daemon: not supported" >&2; exit 1; }
    case " $* " in *" com.docker.network.bridge.enable_icc=false "*) echo false ;; *) echo true ;; esac > "$d/network"
    echo "f00d" ;;
  "inspect "*) exit 1 ;;
  "run "*)
    [ ! -e "$d/fail_run" ] || { echo "no room for it" >&2; exit 1; }
    while [ $# -gt 0 ] && [ "$1" != tail ]; do
      [ "$1" != --name ] || name=$2
      shift
    done
    [ -e "$(file "$2")" ] || { echo "Unable to find image '$2' locally" >&2; exit 125; }
    id_of "$2" > "$d/containers/$name" ;;
  *) echo "unexpected: $*" >&2; exit 2 ;;
esac
"##;

/// The revision label of the stand-in's published builds.
const REVISION: &str = "688d0dd";

/// A stand-in for the `docker` or `container` CLI, with no sandboxes yet.
pub struct FakeCli {
    pub dir: tempfile::TempDir,
}

impl FakeCli {
    /// A host that has the published images `have` and can pull those in
    /// `registry`, each a build of its own.
    pub fn new(have: &[&str], registry: &[&str]) -> Result<Self> {
        let cli = Self::empty()?;
        for name in have {
            cli.have(name, &id(&format!("have {name}")), Some("2.1.302"))?;
        }
        for name in registry {
            cli.publish(name, &id(&format!("registry {name}")), "2.1.302")?;
        }
        Ok(cli)
    }

    /// A host with no images, and nothing it can pull.
    pub fn empty() -> Result<Self> {
        let dir = tempfile::tempdir()?;
        for sub in ["have", "registry", "containers"] {
            std::fs::create_dir(dir.path().join(sub))?;
        }
        write_script(&dir.path().join("cli"), SCRIPT)?;
        Ok(Self { dir })
    }

    /// Puts `name` on the host as the build `id`: published, with that
    /// Claude Code, or built here (`None`).
    pub fn have(&self, name: &str, id: &str, claude_code: Option<&str>) -> Result<()> {
        let labels = match claude_code {
            Some(version) => format!("{REVISION} {version}"),
            None => "- -".into(),
        };
        let file = self.dir.path().join("have").join(key(name));
        std::fs::write(file, format!("{id} {labels} {name}\n"))?;
        Ok(())
    }

    /// Publishes the build `id`, with that Claude Code, as `name`.
    pub fn publish(&self, name: &str, id: &str, claude_code: &str) -> Result<()> {
        let file = self.dir.path().join("registry").join(key(name));
        std::fs::write(file, format!("{id} {REVISION} {claude_code} {name}\n"))?;
        Ok(())
    }

    /// Makes a container, `name`, from the build `id`.
    pub fn uses(&self, name: &str, id: &str) -> Result<()> {
        std::fs::write(self.dir.path().join("containers").join(name), id)?;
        Ok(())
    }

    /// Removes the container `name`.
    pub fn drop_container(&self, name: &str) -> Result<()> {
        std::fs::remove_file(self.dir.path().join("containers").join(name))?;
        Ok(())
    }

    /// Makes `network create` fail, as a runtime that can't make networks.
    pub fn no_networks(&self) -> Result<()> {
        std::fs::write(self.dir.path().join("no_networks"), "")?;
        Ok(())
    }

    /// Makes every `run` fail as if for some other reason than the image.
    pub fn fail_runs(&self) -> Result<()> {
        std::fs::write(self.dir.path().join("fail_run"), "")?;
        Ok(())
    }

    /// Takes away the CLI's execute permission, so that it can't be started.
    pub fn wont_start(&self) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let bin = self.dir.path().join("cli");
        std::fs::set_permissions(bin, std::fs::Permissions::from_mode(0o644))?;
        Ok(())
    }

    /// The images on the host: each name with its build's ID, by name.
    pub fn images(&self) -> Result<Vec<(String, String)>> {
        let mut images = Vec::new();
        for entry in std::fs::read_dir(self.dir.path().join("have"))? {
            let text = std::fs::read_to_string(entry?.path())?;
            let fields: Vec<&str> = text.split_whitespace().collect();
            if let [id, _, _, name] = fields[..] {
                images.push((name.to_owned(), id.to_owned()));
            }
        }
        images.sort();
        Ok(images)
    }

    pub fn driver(&self, flavor: Flavor) -> ContainerDriver {
        let bin = self.dir.path().join("cli").display().to_string();
        ContainerDriver::with_bin(flavor, bin)
    }

    pub fn host_driver(&self, flavor: Flavor) -> Driver {
        Driver::Container(self.driver(flavor))
    }

    /// The commands run so far, one per line.
    pub fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.path().join("calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// Creates the sandbox: the image it started from, and the log.
    pub async fn ensure(
        &self,
        flavor: Flavor,
        spec: &SandboxSpec,
    ) -> Result<(String, Vec<String>)> {
        let lines = Mutex::new(Vec::new());
        let log = |line: String| {
            lines
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(line)
        };
        self.driver(flavor).ensure(spec, &log).await?;
        let run = self
            .calls()
            .into_iter()
            .rev()
            .find(|c| c.starts_with("run "));
        let image = run
            .as_deref()
            .and_then(|c| c.split(" tail ").nth(1)?.split(' ').next())
            .unwrap_or_default()
            .to_owned();
        Ok((
            image,
            lines.into_inner().unwrap_or_else(PoisonError::into_inner),
        ))
    }
}

/// Writes `script` to `path`, executable, for a test to run right away.
/// A process that another test starts holds copies of this process's open
/// files until it execs, and running a script while such a copy is open for
/// writing fails with "Text file busy". So `sh` writes the script, and it is
/// never open here.
pub fn write_script(path: &Path, script: &str) -> Result<()> {
    let status = std::process::Command::new("sh")
        .args(["-c", r#"printf %s "$2" > "$1" && chmod 755 "$1""#, "sh"])
        .arg(path)
        .arg(script)
        .status()?;
    anyhow::ensure!(status.success(), "writing {}: {status}", path.display());
    Ok(())
}

/// A made-up image ID.
pub fn id(seed: &str) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(seed.as_bytes())))
}

fn key(name: &str) -> String {
    name.replace(['/', ':'], "_")
}
