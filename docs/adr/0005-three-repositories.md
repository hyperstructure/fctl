# 0005: fctl, fctl-site and design are separate repositories

2026-10-17 · accepted

## Context

The project site (fctl.sh) and the design system (tokens, fonts, WebGL
scenes, marks) started inside this repository. The site has its own release
cadence, and the design system serves more than fctl: hyperstructure's
symbol lives there too.

## Decision

- hyperstructure/fctl keeps the product: the Rust crates, the web app, the
  docs, xtask, images and instance descriptions.
- hyperstructure/fctl-site holds fctl.sh. It renders this repository's
  `docs/` at a commit pinned in its `fctl-docs.ref`; this repository's CI
  updates that pin when `docs/` changes on `main`, which deploys the site.
- hyperstructure/design publishes `@hyperstructure/design`; the web app and
  the site depend on a tag.
- Releases connect through infrastructure, not code: `cargo xtask publish`
  writes to a KV namespace (`deploy/releases.toml`) that the site's Worker
  serves at `/releases`.

## Consequences

- Docs are written here, next to the code they describe, and still deploy
  on their own.
- Design changes ship as tagged releases and arrive as dependency bumps.
- While the repositories are private, CI reaches the others with read-only
  deploy keys and npm installs the design package over git; once public,
  the package moves to npm and the keys go away.
