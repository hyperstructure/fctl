# The CLI

`fctl` does everything the web app does, for people and for scripts and
agents. Every command takes `--json` for machine-readable output and
`--server <url>` (or `FCTL_SERVER`) to pick an instance; `fctl <command> --help`
has the details.

## Signing in

```sh
fctl login app.fctl.sh       # opens the browser to approve this device
fctl login --no-browser      # prints the link and code instead (SSH, servers)
fctl whoami
fctl logout
```

For scripts and CI, create a personal access token and pass it as
`FCTL_TOKEN`:

```sh
fctl token "deploy bot" --days 90
FCTL_TOKEN=fctl_pat_… fctl sessions --json
```

Configuration lives in `~/.config/fctl/` (`config.toml`, and
`credentials.toml` with mode 0600). `FCTL_CONFIG_DIR` moves it.

## Sessions

Sessions are named by their id (`ses_…`), their URL, or `project#number`.

```sh
fctl run -p my-app "Add a dark mode toggle"   # start one and follow it
fctl run -p my-app -d -f task.md              # start from a file, don't follow
echo "Fix the flaky test" | fctl run -p my-app
fctl run --model opus --effort high "…"       # override the project's agent settings
fctl run --branch fix/login --base develop "…"
fctl run -p my-app -a screenshot.png "Make the header match this"   # attach files (repeat -a)

fctl sessions                    # recent sessions
fctl sessions --active -p my-app
fctl show my-app#12              # status, branch, pull request, usage
fctl edit my-app#12 --title "Fix the flaky login test"
fctl edit my-app#12 --description "Retries the redirect and tests it."   # "" clears it
fctl logs my-app#12              # the transcript
fctl logs -f my-app#12           # …and keep following

fctl attach my-app#12            # stream it and type messages (Ctrl-C detaches)
fctl attach --read-only my-app#12
fctl send my-app#12 "Use the existing Button component"
fctl send my-app#12 -a ci.log "This is what CI printed"
fctl interrupt my-app#12         # stop the current turn
fctl stop my-app#12              # end the session (its sandbox is kept for resuming)
fctl resume my-app#12 "Now add tests"
fctl open my-app#12              # in the browser
fctl open my-app#12 --claude     # in the Claude app (Remote Control)
```

`fctl run` uses the project from `fctl projects use` when `-p` is missing.

A session's title starts as the first line of its task. Agents rename it to
what they work on, and say what the session is about, with the same
command: in a sandbox, `fctl edit --title … --description …` without a
session edits the one the sandbox runs, through fctl-agent and without a
sign-in. The sandbox's `~/AGENTS.md` asks them to do it once they
understand the task, and again when the work turns to something else.

`--attach` (`-a`) sends files with the task or message (`run`, `send`,
`resume`): up to 10, of at most 20 MB each. The agent finds them in
`~/attachments/` in its sandbox, and sees images that Claude accepts as
they are (PNG, JPEG, GIF, WebP up to 2000 px a side) in the message itself.

## Projects

```sh
fctl projects list
fctl projects show my-app
fctl projects create "My app" --repo my-org/my-app
fctl projects create "New thing" --new-repo new-thing --public
fctl projects create "Site" --repo my-org/site --no-connectors   # no claude.ai connectors in its sessions
fctl projects use my-app
fctl projects delete my-app            # with its sessions; asks you to type the slug
```

## Claude

```sh
fctl claude connect --login      # full sign-in through a host (Remote Control)
fctl claude connect              # paste a token from `claude setup-token`
fctl claude connect --token sk-ant-oat01-…
fctl claude status
```

## Hosts

```sh
fctl host install                # register this machine and start the service
fctl host install --driver lima --capacity 2 --build-image
fctl host install --cpus 4 --memory 8g --max-memory 12g   # sandbox sizes
fctl host install --token fctl_rt_…   # with a runner token from an admin
fctl host install --no-service   # write the config only (run it yourself)
fctl host run                    # run in the foreground (what the service runs)
fctl host status
fctl host doctor                 # check the driver, image, agent binary, connection
fctl host logs -n 200
fctl host image build            # (re)build fctl-agent:latest for this driver
fctl host restart                # sessions keep running
fctl host uninstall              # sandboxes are kept
```

`fctl host install` checks the driver before it registers anything: if the
driver can't run sessions (Apple's `container` or Lima isn't installed, a
service isn't running), it says what to fix and stops. `--driver auto` (the
default) picks the first ready one of apple, docker and lima, never the
process driver, and the driver it checked goes into the host's settings in
`~/.config/fctl/host.toml`:

```toml
server = "https://app.fctl.sh"
name = "studio-mac"
driver = "apple"         # apple | lima | docker | process | auto (picks at every start)
capacity = 4             # concurrent sessions
image = "fctl-agent:latest"
cpus = 4                 # per sandbox, when the project sets none
memory_mb = 8192         # per sandbox, when the project sets none
max_cpus = 8             # the most one sandbox may have (unset: all there is)
max_memory_mb = 12288
keep_stopped_hours = 72  # then remove a stopped session's sandbox (0 keeps them)
```

A project's Settings → Sandbox can ask for more CPUs or memory than the
host's default. Sessions only start on a host whose maximum covers that;
when none does, they stay queued and say so ("No runner can give this
session 32 GiB of memory (the most is 15.6 GiB)"). Without `max_cpus` and
`max_memory_mb`, the maximum is what the machine has, or with Docker the
Docker daemon's: on macOS that is the VM of OrbStack, Docker Desktop or
Colima, which is usually smaller than the Mac. A default above the maximum
is lowered to it. `fctl host status` shows the sizes the host gives. The
host measures them when it starts and when it reconnects, so run `fctl host
restart` after changing `host.toml` or resizing the Docker VM.

Agents are told their sandbox's size in their instructions and in
`FCTL_SANDBOX_CPUS` and `FCTL_SANDBOX_MEMORY_MB`: with Docker, `nproc` and
`free` show the whole VM rather than the sandbox's share.

## Admin

```sh
fctl admin users
fctl admin allowlist
fctl admin allow ada@example.com           # also creates an invite link
fctl admin allow '*@example.com'           # a whole domain
fctl admin allow grace@example.com --admin
fctl admin disallow '*@example.com'        # its users lose access
fctl admin runners
```

## Your account

```sh
fctl account export                        # your data as JSON: fctl-export-<date>.json
fctl account export -o - | jq .sessions    # or print it
fctl account delete                        # shows what goes, asks for your email address
```

The export holds your account, sign-in methods, devices and tokens, Claude
connections (without their secrets), projects, the sessions you started with
their transcripts, and your activity log. Deleting your account deletes all
of that and the projects only you are a member of; sessions you started in
shared projects stay there, credited to a deleted user. Settings in the web
app has both, the deletion in its danger zone.

## Updating

```sh
fctl self update             # to the latest release
fctl self update --check     # is there one?
fctl self update --version 0.1.2
```

It downloads the release for this machine, checks its SHA-256, replaces
`fctl` (and the Linux agent next to it), restarts a host service on the new
version (sessions keep running) and refreshes installed completions.
`FCTL_DOWNLOAD` points it at another release server.

## Shell completions

The installer sets up tab completion for your shell. To do it yourself:

```sh
fctl completions --install           # zsh, bash or fish, from $SHELL
fctl completions zsh > ~/.zfunc/_fctl  # or print them: bash, zsh, fish, elvish, powershell
```

`--install` writes the script under `~/.local/share/fctl/completions` (fish:
`~/.config/fish/completions`) and adds a marked block once to the end of
`~/.zshrc` or `~/.bashrc`. Bash on macOS starts as a login shell, which reads
only the first of `~/.bash_profile`, `~/.bash_login` and `~/.profile` that
exists, so the block goes in that file (a new `~/.profile` if there is none).
Delete the script and the block to undo it.

## Exit codes and output

Commands exit non-zero on failure and print errors to stderr. With `--json`,
results go to stdout as one JSON document (or one per line for streams such as
`fctl logs -f --json`), using the same shapes as the HTTP API.
