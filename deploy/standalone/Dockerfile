# syntax=docker/dockerfile:1
#
# The fctl control plane without Cloudflare, ghcr.io/hyperstructure/fctl-control:
# the Worker (crates/fctl-control) and the web app, run by workerd, Cloudflare's
# open-source Workers runtime, through Miniflare (server.mjs), with its state
# in /data. docs/self-hosting.md says how to run it. Build it from the root of
# the repository, which is its context:
#
#   docker build -f deploy/standalone/Dockerfile -t fctl-control .
#
# The Worker and the web app are the same on every platform, so they build on
# the build machine's (BUILDPLATFORM); only the last stage is the target's.

FROM --platform=$BUILDPLATFORM docker.io/library/rust:1-slim-bookworm AS worker
RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates pkg-config libssl-dev \
  && rm -rf /var/lib/apt/lists/*
WORKDIR /src
# The toolchain rust-toolchain.toml pins, with the wasm32 target.
COPY rust-toolchain.toml ./
RUN rustup toolchain install
# The version of the worker crate (xtask/src/util.rs installs the same).
RUN cargo install worker-build --version 0.8.7 --locked
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY xtask xtask
RUN cd crates/fctl-control && worker-build --release

FROM --platform=$BUILDPLATFORM docker.io/library/node:22-bookworm-slim AS web
WORKDIR /src
COPY package.json package-lock.json ./
COPY web/package.json web/
COPY deploy/standalone/package.json deploy/standalone/
RUN npm ci --no-audit --no-fund
COPY web web
RUN npm run build --workspace web

FROM docker.io/library/node:22-bookworm-slim
RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates tini \
  && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY package.json package-lock.json ./
COPY web/package.json web/
COPY deploy/standalone/package.json deploy/standalone/
# wrangler (the migrations), and the Miniflare and workerd it runs. Miniflare
# loads sharp only for Cloudflare Images, which fctl doesn't use.
RUN npm ci --omit=dev --workspace @fctl/standalone --include-workspace-root=false --no-audit --no-fund \
  && rm -rf node_modules/sharp node_modules/@img \
  && npm cache clean --force
COPY crates/fctl-control/wrangler.toml crates/fctl-control/
COPY crates/fctl-control/migrations crates/fctl-control/migrations
COPY --from=worker /src/crates/fctl-control/build/index.js /src/crates/fctl-control/build/index_bg.wasm crates/fctl-control/build/
COPY --from=web /src/web/dist web/dist
COPY deploy/standalone deploy/standalone
RUN mkdir /data && chown node:node /data

USER node
ENV FCTL_DATA_DIR=/data
VOLUME /data
EXPOSE 8787
HEALTHCHECK --interval=30s --timeout=10s --start-period=2m \
  CMD ["node", "-e", "fetch(`http://127.0.0.1:${process.env.FCTL_PORT || process.env.PORT || 8787}/api/v1/health`).then((r) => process.exit(r.ok ? 0 : 1), () => process.exit(1))"]
# -s: it reaps orphans even where it isn't PID 1 (a pod that shares its process namespace).
ENTRYPOINT ["tini", "-s", "--", "node", "/app/deploy/standalone/server.mjs"]

LABEL org.opencontainers.image.title="fctl-control" \
  org.opencontainers.image.description="The fctl control plane without Cloudflare: the Worker and the web app on workerd" \
  org.opencontainers.image.url="https://fctl.sh/docs/self-hosting/" \
  org.opencontainers.image.source="https://github.com/hyperstructure/fctl" \
  org.opencontainers.image.licenses="MIT"
