# Changelog

What changed in each fctl release, from its Conventional Commits (`cargo xtask release` writes this file).

## 0.10.1 (2026-10-20)

### Features

- **host:** Give back the space Apple sandboxes freed as they stop ([#120](https://github.com/hyperstructure/fctl/pull/120))

## 0.10.0 (2026-10-20)

### Features

- **cli:** Show runner ids and let add-runner default to this machine ([#113](https://github.com/hyperstructure/fctl/pull/113))
- **cli:** Show Claude connection ids and choose connections by name ([#114](https://github.com/hyperstructure/fctl/pull/114))
- Keep the floor's draft and count a project's other repositories ([#117](https://github.com/hyperstructure/fctl/pull/117))

### Fixes

- **web:** Load GitHub account avatars from the host the CSP allows ([#110](https://github.com/hyperstructure/fctl/pull/110))
- **web:** Line up the workspace dropdown in the admin runner form ([#111](https://github.com/hyperstructure/fctl/pull/111))
- **web:** List runners from control planes before workspaces ([#112](https://github.com/hyperstructure/fctl/pull/112))
- **web:** Give list rows the hover arrow's room on phones ([#115](https://github.com/hyperstructure/fctl/pull/115))
- **web:** Stop browsers filling saved logins into secret fields ([#116](https://github.com/hyperstructure/fctl/pull/116))
- **web:** Make the floor's hero shorter on phones ([#118](https://github.com/hyperstructure/fctl/pull/118))
- **web:** Fit the Claude plan meters on one line on phones ([#119](https://github.com/hyperstructure/fctl/pull/119))

## 0.9.0 (2026-10-20)

### Features

- Add workspaces with their own people, runners and GitHub accounts ([#109](https://github.com/hyperstructure/fctl/pull/109))

### Fixes

- **host:** Treat a container CLI that can't start as an error ([#107](https://github.com/hyperstructure/fctl/pull/107))
- **web:** Open the account menu above your name so you can sign out ([#108](https://github.com/hyperstructure/fctl/pull/108))

### Other changes

- Fix flaky tests that run a script right after writing it ([#106](https://github.com/hyperstructure/fctl/pull/106))

## 0.8.0 (2026-10-19)

### Features

- Add two-factor sign-in with authenticator apps (#103) and sign in with passkeys ([#104](https://github.com/hyperstructure/fctl/pull/104))

### Documentation

- Tell agents that only people with write access instruct them ([#98](https://github.com/hyperstructure/fctl/pull/98))
- List hyperstructure/sandbox among the other repositories ([#99](https://github.com/hyperstructure/fctl/pull/99))
- List Homebrew as an alternative way to install the CLI ([#101](https://github.com/hyperstructure/fctl/pull/101))

### Other changes

- Check agent skills against the Agent Skills specification ([#102](https://github.com/hyperstructure/fctl/pull/102))

## 0.7.0 (2026-10-18)

### Fixes

- **control:** Let sessions carry on when their runner comes back ([#94](https://github.com/hyperstructure/fctl/pull/94))

### Other changes

- Keep production secrets in the production environment ([#95](https://github.com/hyperstructure/fctl/pull/95))
- Drop the design deploy key now that the repositories are public ([#96](https://github.com/hyperstructure/fctl/pull/96))
- Protect main with rulesets and push releases with a deploy key ([#97](https://github.com/hyperstructure/fctl/pull/97))

## 0.6.0 (2026-10-18)

### Features

- Let hosts allow and build projects' sandbox images ([#88](https://github.com/hyperstructure/fctl/pull/88))
- **host:** Refresh the default sandbox image and remove old ones ([#89](https://github.com/hyperstructure/fctl/pull/89))
- **host:** Make room when the disk runs low ([#93](https://github.com/hyperstructure/fctl/pull/93))

### Fixes

- **agent:** Keep the host's link when a second fctl-agent starts ([#82](https://github.com/hyperstructure/fctl/pull/82))
- Keep fake-claude sessions running while they answer queued messages ([#85](https://github.com/hyperstructure/fctl/pull/85))
- **agent:** Keep sessions up while agents wait for background tasks ([#91](https://github.com/hyperstructure/fctl/pull/91))

### Documentation

- Explain how Apple container sandboxes use CPUs, the clock and disk ([#77](https://github.com/hyperstructure/fctl/pull/77))
- Say how agents in a session show recordings ([#78](https://github.com/hyperstructure/fctl/pull/78))
- Tick the first real-Claude runs in the roadmap ([#81](https://github.com/hyperstructure/fctl/pull/81))

### Other changes

- **web:** Replace Biome's deprecated recommended field with preset ([#83](https://github.com/hyperstructure/fctl/pull/83))
- **agent:** Fix a flaky test of the host socket's lock ([#84](https://github.com/hyperstructure/fctl/pull/84))
- Run one fake-claude session through cargo xtask dev ([#86](https://github.com/hyperstructure/fctl/pull/86))

## 0.5.0 (2026-10-18)

### Features

- Stop idle sessions after ten minutes and resume them for a message ([#67](https://github.com/hyperstructure/fctl/pull/67))
- Show when a session's runner is away or can't reach its agent ([#69](https://github.com/hyperstructure/fctl/pull/69))
- Let agents publish screenshots for pull requests on fctl ([#71](https://github.com/hyperstructure/fctl/pull/71))
- **web:** Turn the 404 and error pages into a stream you can play with ([#72](https://github.com/hyperstructure/fctl/pull/72))
- Drain runners so they take no new sessions ([#74](https://github.com/hyperstructure/fctl/pull/74))
- Ask before fctl host uninstall stops the sessions on a host ([#75](https://github.com/hyperstructure/fctl/pull/75))

### Fixes

- Let fake-claude sessions continue after a usage limit resets ([#64](https://github.com/hyperstructure/fctl/pull/64))
- **host:** Fall back to latest when the sandbox image isn't published ([#62](https://github.com/hyperstructure/fctl/pull/62))
- **web:** Make hover fills visible in the light theme ([#65](https://github.com/hyperstructure/fctl/pull/65))
- **control:** Keep events that newer ones overtook after a reconnect ([#70](https://github.com/hyperstructure/fctl/pull/70))
- Resume a session for messages that come as it stops ([#73](https://github.com/hyperstructure/fctl/pull/73))

### Other changes

- **deps:** Use @hyperstructure/design v0.4.0 ([#68](https://github.com/hyperstructure/fctl/pull/68))

## 0.4.0 (2026-10-18)

### Features

- Notify people when a session waits for them, pauses or fails ([#48](https://github.com/hyperstructure/fctl/pull/48))
- Show Claude plan usage in Settings, on the floor and in the CLI ([#58](https://github.com/hyperstructure/fctl/pull/58))
- **host:** Pull the sandbox image from GHCR instead of building it ([#39](https://github.com/hyperstructure/fctl/pull/39))
- Credit the person who started a session in its commits ([#55](https://github.com/hyperstructure/fctl/pull/55))
- **web:** Replace native selects with an animated Select ([#61](https://github.com/hyperstructure/fctl/pull/61))

### Fixes

- Stop removed browsers coming back and digests replacing each other ([#52](https://github.com/hyperstructure/fctl/pull/52))
- Keep runners online and agents reachable when Docker hangs ([#57](https://github.com/hyperstructure/fctl/pull/57))
- **xtask:** Install worker-build when cargo xtask dev needs it ([#54](https://github.com/hyperstructure/fctl/pull/54))
- **web:** Make destructive actions and errors red instead of pink ([#60](https://github.com/hyperstructure/fctl/pull/60))

### Other changes

- **release:** Build the binaries in parallel and without warnings ([#50](https://github.com/hyperstructure/fctl/pull/50))

## 0.3.0 (2026-10-18)

### Features

- Open Remote Control sessions in the Claude desktop and mobile apps ([#42](https://github.com/hyperstructure/fctl/pull/42))
- **web:** Use hyperstructure's logo and make the app installable ([#44](https://github.com/hyperstructure/fctl/pull/44))
- Add Python 3.15, Rust, Bun and TypeScript to the sandbox image ([#45](https://github.com/hyperstructure/fctl/pull/45))

### Fixes

- **web:** Pad session rows so the hover highlight has room ([#38](https://github.com/hyperstructure/fctl/pull/38))

### Documentation

- Add agent rules for screenshots in pull requests ([#43](https://github.com/hyperstructure/fctl/pull/43))

## 0.2.0 (2026-10-18)

### Features

- Give hosts sandbox limits and schedule sessions by size ([#31](https://github.com/hyperstructure/fctl/pull/31))
- Install fctl in sandboxes and let agents name their sessions ([#33](https://github.com/hyperstructure/fctl/pull/33))
- Turn off claude.ai connectors for a project ([#32](https://github.com/hyperstructure/fctl/pull/32))
- Attach files and screenshots to tasks and messages ([#35](https://github.com/hyperstructure/fctl/pull/35))
- Let projects span several repositories or a whole organization ([#36](https://github.com/hyperstructure/fctl/pull/36))

### Fixes

- **agent:** Leave Claude Code's footer out of pull requests ([#23](https://github.com/hyperstructure/fctl/pull/23))
- **host:** Check the driver before registering a runner ([#24](https://github.com/hyperstructure/fctl/pull/24))
- **cli:** Install bash completions without hiding ~/.profile ([#27](https://github.com/hyperstructure/fctl/pull/27))
- **web:** Show that a session is starting and never start it twice ([#30](https://github.com/hyperstructure/fctl/pull/30))

### Documentation

- Add agent rules for issues and pull requests ([#29](https://github.com/hyperstructure/fctl/pull/29))

### Other changes

- Start session branches untracked so a plain `git push` publishes them ([#14](https://github.com/hyperstructure/fctl/pull/14))
- Release from a workflow, with notes from Conventional Commits ([#21](https://github.com/hyperstructure/fctl/pull/21))
- Check pull request titles in their own workflow ([#25](https://github.com/hyperstructure/fctl/pull/25))

## 0.1.2 (2026-10-17)

### Other changes

- Turn on Google sign-in for app.fctl.sh ([d4a3152](https://github.com/hyperstructure/fctl/commit/d4a3152d65e3ed518470790f0221bbd54f990e55))
- Purge deleted sessions' transcripts; delete expired records hourly ([e7571fc](https://github.com/hyperstructure/fctl/commit/e7571fc100551c48b8330ead03fa2f9eac60662c))
- AGENTS.md: keep fctl.sh's privacy policy in step with the app ([5c7bc51](https://github.com/hyperstructure/fctl/commit/5c7bc5147d752f666ef488cfe94fa6ae8bb9ace7))
- Export your data and delete your account from Settings ([5309ebd](https://github.com/hyperstructure/fctl/commit/5309ebd6cec2a407658c360f0a5893ce382a027a))
- First and last names, and greetings that use all your first names ([9da1b84](https://github.com/hyperstructure/fctl/commit/9da1b842e579a62e4ce15876b664102dcd1871a6))
- Delete a project from its settings and with `fctl projects delete` ([375d54c](https://github.com/hyperstructure/fctl/commit/375d54ca0abf01abcc952c92a2aad1ee95be0b53))
- Remove stopped sandboxes after keep_stopped_hours ([39c5de9](https://github.com/hyperstructure/fctl/commit/39c5de93c1c142cd7174a7260123cdfcfb14f6e7))
- fctl self update, and completions that set themselves up ([b58170a](https://github.com/hyperstructure/fctl/commit/b58170a3445433a84c3ef2a0a3b0c798c83ec89c))

## 0.1.1 (2026-10-17)

### Other changes

- Link the roadmap to its issues; a firmer sign-in card ([74ce9d3](https://github.com/hyperstructure/fctl/commit/74ce9d3c962a92cc831d263889333fa86397da22))
- Record the first five architecture decisions ([b76b749](https://github.com/hyperstructure/fctl/commit/b76b7494fbb1ab4f19dbdbd2a5a69634cdf8ca33))
- Dist: macOS binaries run on macOS 12 and later ([#7](https://github.com/hyperstructure/fctl/pull/7))

## 0.1.0 (2026-10-17)

### Other changes

- Bootstrap the protocol, domain logic and control plane ([f6a60d8](https://github.com/hyperstructure/fctl/commit/f6a60d83793e0bab1c3155c48830ae7de616d438))
- Add fctl-agent, the in-sandbox supervisor, and fake-claude ([df58db4](https://github.com/hyperstructure/fctl/commit/df58db4027b9ef5d88903b9ec107b23176b94d8e))
- Add the host daemon, the fctl CLI and the client library ([48d57b0](https://github.com/hyperstructure/fctl/commit/48d57b0dc946756ac5d588513ec8492adfcb9ca1))
- Web: the fctl web app ([4190e3f](https://github.com/hyperstructure/fctl/commit/4190e3f3751cd24c8b734934fbc0958b25e968bd))
- Web: the brand, and the foundry scene ([630260e](https://github.com/hyperstructure/fctl/commit/630260efb98f4240fa221d01547f358d44163748))
- Add fctl.sh and the docs ([3e0a9e9](https://github.com/hyperstructure/fctl/commit/3e0a9e9d64adcd4e7dbe778a7185f112f656ce36))
- Xtask: dev, check, deploy, secrets, dist and publish ([4235869](https://github.com/hyperstructure/fctl/commit/4235869fed78f0b14a3ae0102fee9454cb9c31eb))
- Roadmap: split the site and the design system into their own repositories ([e98df96](https://github.com/hyperstructure/fctl/commit/e98df960d911cf98a4153f23f258e230c7f0cb49))
- CI: checks, deploys from main, releases from tags ([02820aa](https://github.com/hyperstructure/fctl/commit/02820aa95016d59770d6419f5af14a4210d1a016))
- Split the site and the design system into their own repositories ([bf4a8b0](https://github.com/hyperstructure/fctl/commit/bf4a8b0a208c44f41b68736b304aaf5f34b2bf16))
