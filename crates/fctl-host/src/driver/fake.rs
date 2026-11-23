//! A stand-in for the `docker` and `container` CLIs, for tests of the
//! drivers and of what the host does with its images.

use super::container::{ContainerDriver, Flavor};
use super::{Driver, SandboxSpec};
use anyhow::Result;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

/// The CLI. Each image name on the host is a file in `have`, and each one
/// it can pull a file in `registry`, holding the build's ID, its revision
/// label and Claude Code version (`-` for a build without labels) and the
/// name. `containers` holds the ID of the build each container was made
/// from. Like Docker, `image rm` refuses to remove an image's last name
/// while a container uses it; Apple's `image delete` doesn't check. A
/// `build` lists its context in `context`, copies its Dockerfile to
/// `dockerfile` and makes a build without labels, and with `fail_build` it
/// fails; `size` holds the size images have (1000 bytes without it). With
/// `fail_run`, `run` fails, and with `full_disk` as for a full disk. With
/// `slow_pull`, pulls print their progress as Apple's `container` does and
/// take most of a second; `breaks` holds how many more pulls break off, and
/// `break_error` what they say then.
/// `network create` makes `network`, which holds whether its containers
/// reach each other, unless `no_networks` is there. Each volume is a file in
/// `volumes` holding the KiB it takes, which `run` makes when a container
/// mounts one that isn't there, and `mounts` holds the volume each
/// container mounts. Like Docker, `volume rm` refuses while a container
/// mounts the volume. Short-lived containers (`run --rm`) measure a volume
/// with `du` and empty it with `find`. `exec` prints `uid` (`1000:1000`
/// without it), as `id` would, or its user's part for `id -u`, unless it
/// runs as root or in the background; asked whether fctl-agent ran as root,
/// it says so when `agent_dir` is there; `cp` copies nothing. `version`
/// answers with the daemon's version, as a daemon that runs does.
const SCRIPT: &str = r##"#!/bin/sh
d=$(dirname "$0")
echo "$*" >> "$d/calls"
[ ! -e "$d/refuse" ] || { cat "$d/refuse" >&2; exit 1; }
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
  if [ -e "$d/slow_pull" ]; then
    echo "[1/2] Fetching image 40% (6 of 7 blobs, 400 MB/1 GB, 120 KB/s) [1s]" >&2
    sleep 0.4
    echo "[1/2] Fetching image 70% (6 of 7 blobs, 700 MB/1 GB, 150 KB/s) [2s]" >&2
    sleep 0.4
  fi
  breaks=$(cat "$d/breaks" 2>/dev/null || echo 0)
  if [ "$breaks" -gt 0 ]; then
    echo $((breaks - 1)) > "$d/breaks"
    echo "[1/2] Fetching image 76% (6 of 7 blobs, 981,6 MB/1,25 GB, 233 KB/s) [9m 47s]" >&2
    echo "[1/2] Fetching image 100% (7 blobs, 1,25 GB) [10m 0s]" >&2
    if [ -e "$d/break_error" ]; then cat "$d/break_error" >&2; else
      echo 'Error: StreamClosed(streamID: HTTP2StreamID(5), errorCode: HTTP2ErrorCode<0x1 ProtocolError>, location: "NIOHTTP2/HTTP2StreamChannel.swift:892")' >&2
    fi
    exit 1
  fi
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
  "version --format") echo 28.5.1 ;;
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
  "inspect --format")
    case "$3" in
      "{{.Image}}")
        [ -e "$d/containers/$4" ] || { echo "Error: No such object: $4" >&2; exit 1; }
        cat "$d/containers/$4" ;;
      "{{.State.Status}}") [ -e "$d/state/$4" ] || exit 1; cat "$d/state/$4" ;;
      "{{json .HostConfig.CapDrop}}")
        [ -e "$d/containers/$4" ] || { echo "Error: No such object: $4" >&2; exit 1; }
        if [ -e "$d/capdrop/$4" ]; then cat "$d/capdrop/$4"; else echo null; fi ;;
      *) exit 1 ;;
    esac ;;
  "inspect "*) exit 1 ;;
  "create --name")
    [ ! -e "$d/fail_guard" ] || { echo "Error response from daemon: cannot join network namespace of a non running container" >&2; exit 1; }
    echo "guard" ;;
  "start -a")
    [ ! -e "$d/fail_guard" ] || exit 1
    echo "Cloud metadata services are unreachable from this network: 169.254.169.254" ;;
  "rm -f") rm -f "$d/containers/$3" "$d/state/$3" "$d/capdrop/$3" ;;
  "start "*) [ ! -e "$d/state/$2" ] || echo running > "$d/state/$2" ;;
  "run "*)
    [ ! -e "$d/fail_run" ] || { echo "no room for it" >&2; exit 1; }
    [ ! -e "$d/full_disk" ] || { echo "docker: Error response from daemon: mkdir /var/lib/docker/overlay2/4f1c/merged: no space left on device." >&2; exit 125; }
    name=; rm=; vol=; capdrop=
    while [ $# -gt 0 ] && [ "$1" != --entrypoint ]; do
      case "$1" in
        --name) name=$2 ;;
        --cap-drop) capdrop=$2 ;;
        --rm) rm=1 ;;
        --mount) case "$2" in type=volume,source=*) vol=${2#type=volume,source=}; vol=${vol%%,*} ;; esac ;;
      esac
      shift
    done
    entry=$2; image=$3
    [ -e "$(file "$image")" ] || { echo "Unable to find image '$image' locally" >&2; exit 125; }
    [ -z "$vol" ] || [ -e "$d/volumes/$vol" ] || echo 0 > "$d/volumes/$vol"
    if [ -z "$rm" ]; then
      id_of "$image" > "$d/containers/$name"
      [ -z "$capdrop" ] || printf '["%s"]\n' "$capdrop" > "$d/capdrop/$name"
      [ -z "$vol" ] || echo "$vol" > "$d/mounts/$name"
    else
      case "$entry" in
        du) printf '%s\t/cache\n' "$(cat "$d/volumes/$vol")" ;;
        find) echo 0 > "$d/volumes/$vol" ;;
      esac
    fi ;;
  "volume ls") for v in "$d"/volumes/*; do [ -e "$v" ] && basename "$v"; done ;;
  "volume inspect")
    [ -e "$d/volumes/$3" ] || { echo "Error: no such volume: $3" >&2; exit 1; }
    echo "[{\"Name\": \"$3\"}]" ;;
  "volume rm")
    [ -e "$d/volumes/$3" ] || { echo "Error: No such volume: $3" >&2; exit 1; }
    for m in "$d"/mounts/*; do
      if [ -e "$m" ] && [ "$(cat "$m")" = "$3" ]; then
        echo "Error response from daemon: remove $3: volume is in use - [$(basename "$m")]" >&2
        exit 1
      fi
    done
    rm "$d/volumes/$3"
    echo "$3" ;;
  "exec -u" | "exec -d" | "cp "*) ;;
  "exec "*)
    ids=$(cat "$d/uid" 2>/dev/null || echo 1000:1000)
    case "$3 $4 $5" in
      "id -u "*|*"; id -u") echo "${ids%%:*}" ;;
      *"/var/lib/fctl-agent ]"*) if [ -e "$d/agent_dir" ]; then echo root; else echo user; fi ;;
      *) echo "$ids" ;;
    esac ;;
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
        for sub in [
            "have",
            "registry",
            "containers",
            "volumes",
            "mounts",
            "state",
            "capdrop",
        ] {
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

    /// Makes the container `name` a stopped one, which `docker inspect`
    /// says is there, and `docker start` starts.
    pub fn stopped(&self, name: &str) -> Result<()> {
        std::fs::write(self.dir.path().join("state").join(name), "exited\n")?;
        Ok(())
    }

    /// Removes the container `name`.
    pub fn drop_container(&self, name: &str) -> Result<()> {
        std::fs::remove_file(self.dir.path().join("containers").join(name))?;
        let _ = std::fs::remove_file(self.dir.path().join("mounts").join(name));
        Ok(())
    }

    /// Makes the volume `name`, taking `kib`.
    pub fn volume(&self, name: &str, kib: u64) -> Result<()> {
        std::fs::write(
            self.dir.path().join("volumes").join(name),
            format!("{kib}\n"),
        )?;
        Ok(())
    }

    /// The volumes here, with the KiB each takes.
    pub fn volumes(&self) -> Result<Vec<(String, u64)>> {
        let mut volumes = Vec::new();
        for entry in std::fs::read_dir(self.dir.path().join("volumes"))? {
            let entry = entry?;
            let kib = std::fs::read_to_string(entry.path())?.trim().parse()?;
            volumes.push((entry.file_name().to_string_lossy().into_owned(), kib));
        }
        volumes.sort();
        Ok(volumes)
    }

    /// Makes the sandbox's user `uid:gid`, as `id` prints them.
    pub fn user(&self, ids: &str) -> Result<()> {
        std::fs::write(self.dir.path().join("uid"), format!("{ids}\n"))?;
        Ok(())
    }

    /// Makes the sandbox one where fctl-agent ran as root.
    pub fn ran_agent_as_root(&self) -> Result<()> {
        std::fs::write(self.dir.path().join("agent_dir"), "")?;
        Ok(())
    }

    /// Makes the helper that keeps a sandbox off cloud metadata services
    /// fail to start.
    pub fn fail_guards(&self) -> Result<()> {
        std::fs::write(self.dir.path().join("fail_guard"), "")?;
        Ok(())
    }

    /// Makes `network create` fail, as a runtime that can't make networks.
    pub fn no_networks(&self) -> Result<()> {
        std::fs::write(self.dir.path().join("no_networks"), "")?;
        Ok(())
    }

    /// Makes pulls print how far along they are and take most of a second.
    pub fn slow_pulls(&self) -> Result<()> {
        std::fs::write(self.dir.path().join("slow_pull"), "")?;
        Ok(())
    }

    /// Makes the next `times` pulls break off partway, as ghcr.io's HTTP/2
    /// streams did for Apple's `container`.
    pub fn break_pulls(&self, times: u32) -> Result<()> {
        std::fs::write(self.dir.path().join("breaks"), format!("{times}\n"))?;
        Ok(())
    }

    /// Makes the pulls that break off fail with `error` instead.
    pub fn break_with(&self, error: &str) -> Result<()> {
        std::fs::write(self.dir.path().join("break_error"), format!("{error}\n"))?;
        Ok(())
    }

    /// Makes every `run` fail as if for some other reason than the image.
    pub fn fail_runs(&self) -> Result<()> {
        std::fs::write(self.dir.path().join("fail_run"), "")?;
        Ok(())
    }

    /// Makes every `run` fail as Docker's does when its disk is full.
    pub fn full_disk(&self) -> Result<()> {
        std::fs::write(self.dir.path().join("full_disk"), "")?;
        Ok(())
    }

    /// Makes every command fail with `error`, as the docker CLI fails when
    /// the daemon's socket refuses it or nothing answers there.
    pub fn refuse(&self, error: &str) -> Result<()> {
        std::fs::write(self.dir.path().join("refuse"), format!("{error}\n"))?;
        Ok(())
    }

    /// Lets every command through again, after [`refuse`](Self::refuse).
    pub fn answer(&self) -> Result<()> {
        std::fs::remove_file(self.dir.path().join("refuse"))?;
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

/// A stand-in for Apple's `container` whose sandboxes' VMs run from
/// helper processes, as Apple's do (`container-runtime-linux start
/// --root … --uuid NAME`), each a file `vm-NAME` holding its pid: a
/// sandbox runs while its helper does. The VM of one with a file
/// `frozen-NAME` doesn't answer: `stats`, `stop`, `delete`, `cp` and
/// `exec` in it wait for ever (30 seconds), and an `exec` in it holds the
/// API server's lock (`lock` holds its helper's pid) until that helper
/// goes, which every `exec`, `cp`, `start`, `run`, `stop` and `delete`
/// waits for meanwhile. `list`, `inspect` and the others' `stats` answer
/// as ever. `exec` says `1000` to `id -u`. With `refuse-start` there,
/// `start` fails.
const STUCK_APPLE: &str = r##"#!/bin/sh
d=$(dirname "$0")
echo "$*" >> "$d/calls"
for last in "$@"; do :; done
alive() { s=$(ps -o stat= -p "$1" 2>/dev/null); [ -n "$s" ] && [ "${s#Z}" = "$s" ]; }
runs() { [ -e "$d/vm-$1" ] && alive "$(cat "$d/vm-$1")"; }
frozen() { [ -e "$d/frozen-$1" ] && runs "$1"; }
held() { [ -e "$d/lock" ] && alive "$(cat "$d/lock")"; }
hang() { exec sleep 30; }
wait_lock() { while held; do sleep 0.1; done; }
vm() {
  rm -f "$d/frozen-$1"
  "$d/container-runtime-linux" start --root "$d/$1" --uuid "$1" </dev/null >/dev/null 2>&1 &
  echo $! > "$d/vm-$1"
}
state() { if runs "$1"; then echo running; else echo stopped; fi; }
case "$1" in
  list)
    printf '['; sep=
    for f in "$d"/vm-*; do
      [ -e "$f" ] || continue
      n=${f#"$d"/vm-}
      [ "$(state "$n")" = running ] || continue
      printf '%s{"configuration": {"id": "%s"}, "status": {"state": "running"}}' "$sep" "$n"
      sep=,
    done
    echo ']' ;;
  inspect)
    [ -e "$d/vm-$last" ] || { echo "Error: container $last not found" >&2; exit 1; }
    printf '[{"configuration": {"id": "%s"}, "status": {"state": "%s"}}]\n' "$last" "$(state "$last")" ;;
  stats)
    frozen "$last" && hang
    printf '[{"id": "%s", "memoryUsageBytes": 1}]\n' "$last" ;;
  exec)
    shift
    while [ "${1#-}" != "$1" ]; do case "$1" in -u) shift 2 ;; *) shift ;; esac; done
    runs "$1" || { echo "Error: container $1 is not running" >&2; exit 1; }
    if frozen "$1"; then cp "$d/vm-$1" "$d/lock"; hang; fi
    wait_lock
    case "$*" in *"id -u"*) echo 1000 ;; esac ;;
  cp)
    n=${last%%:*}
    runs "$n" || { echo "Error: container $n is not running" >&2; exit 1; }
    frozen "$n" && hang
    wait_lock ;;
  run)
    n=; prev=
    for a in "$@"; do [ "$prev" = --name ] && n=$a; prev=$a; done
    wait_lock
    vm "$n"
    echo "$n" ;;
  start)
    wait_lock
    [ ! -e "$d/refuse-start" ] || { echo "Error: failed to bootstrap container" >&2; exit 1; }
    runs "$last" || vm "$last"
    echo "$last" ;;
  stop)
    frozen "$last" && hang
    wait_lock
    [ ! -e "$d/vm-$last" ] || kill -9 "$(cat "$d/vm-$last")" 2>/dev/null
    echo "$last" ;;
  delete)
    frozen "$last" && hang
    wait_lock
    [ ! -e "$d/vm-$last" ] || kill -9 "$(cat "$d/vm-$last")" 2>/dev/null
    rm -f "$d/vm-$last" ;;
  clean) wait_lock ;;
  image) echo '[{"name": "image"}]' ;;
  system) echo '{"version": "1.5.0"}' ;;
  *) echo "unexpected: $*" >&2; exit 2 ;;
esac
"##;

/// Apple's `container`, played by [`STUCK_APPLE`] in a directory of
/// its own, with its helpers.
pub struct StuckApple {
    dir: tempfile::TempDir,
}

impl StuckApple {
    /// The stand-in, with the sandboxes `running` and the frozen VMs of
    /// `frozen`.
    pub fn new(running: &[&str], frozen: &[&str]) -> Result<Self> {
        let dir = tempfile::tempdir()?;
        write_script(&dir.path().join("cli"), STUCK_APPLE)?;
        write_script(
            &dir.path().join("container-runtime-linux"),
            "#!/bin/sh\ni=0\nwhile [ $i -lt 600 ]; do sleep 0.1; i=$((i + 1)); done\n",
        )?;
        let apple = Self { dir };
        for name in running {
            apple.cli(&["start", name])?;
        }
        for name in frozen {
            apple.freeze(name)?;
        }
        Ok(apple)
    }

    /// A driver that plays it: its commands take a second at most, and a
    /// VM that doesn't answer within a second is stuck.
    pub fn driver(&self) -> ContainerDriver {
        let bin = self.dir.path().join("cli");
        ContainerDriver::with_bin(Flavor::Apple, bin.display().to_string())
            .timed(Duration::from_secs(1), Duration::from_secs(1))
    }

    /// The VM of the sandbox `name` stops answering.
    pub fn freeze(&self, name: &str) -> Result<()> {
        Ok(std::fs::write(
            self.dir.path().join(format!("frozen-{name}")),
            "",
        )?)
    }

    /// Runs the stand-in, as the host would.
    pub fn cli(&self, args: &[&str]) -> Result<String> {
        let out = std::process::Command::new(self.dir.path().join("cli"))
            .args(args)
            .output()?;
        anyhow::ensure!(out.status.success(), "{args:?}: {out:?}");
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Its `start`s fail from now on, as for a reason of their own.
    pub fn refuse_starts(&self) -> Result<()> {
        Ok(std::fs::write(self.dir.path().join("refuse-start"), "")?)
    }

    /// As if an `exec` had waited on the frozen VM of `name`, and held
    /// the API server's lock since.
    pub fn held_by(&self, name: &str) -> Result<()> {
        Ok(std::fs::write(
            self.dir.path().join("lock"),
            self.helper(name)?.to_string(),
        )?)
    }

    /// The helper of the sandbox `name`, as it was started last.
    pub fn helper(&self, name: &str) -> Result<i32> {
        let pid = std::fs::read_to_string(self.dir.path().join(format!("vm-{name}")))?;
        Ok(pid.trim().parse()?)
    }

    /// Whether the sandbox `name` runs.
    pub fn runs(&self, name: &str) -> Result<bool> {
        let inspect = self.cli(&["inspect", name])?;
        Ok(inspect.contains("\"running\""))
    }

    /// The commands it ran, in order.
    pub fn calls(&self) -> Result<Vec<String>> {
        let calls = std::fs::read_to_string(self.dir.path().join("calls"))?;
        Ok(calls.lines().map(str::to_owned).collect())
    }
}

impl Drop for StuckApple {
    /// Stops the helpers that still run.
    fn drop(&mut self) {
        let Ok(files) = std::fs::read_dir(self.dir.path()) else {
            return;
        };
        for f in files.flatten() {
            let name = f.file_name().to_string_lossy().into_owned();
            let Some(name) = name.strip_prefix("vm-") else {
                continue;
            };
            if let Ok(pid) = self.helper(name) {
                let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
                if String::from_utf8_lossy(&cmdline).contains("container-runtime-linux") {
                    let _ = nix::sys::signal::kill(
                        nix::unistd::Pid::from_raw(pid),
                        nix::sys::signal::Signal::SIGKILL,
                    );
                }
            }
        }
    }
}
