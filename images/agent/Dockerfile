# The default fctl sandbox image: Debian with git, gh, Node.js, Python (uv),
# build tools, Claude Code and fctl. Sessions run as the `agent` user.
#
#   fctl host image build          # builds it with your driver (apple or docker)
#   docker build -t fctl-agent:latest images/agent
FROM debian:bookworm-slim

# The fctl release to install and where releases come from; `fctl host
# image build` passes its own version and download_base. Empty: the latest,
# from https://fctl.sh/releases.
ARG FCTL_VERSION=
ARG FCTL_DOWNLOAD=
ENV LANG=C.UTF-8 LC_ALL=C.UTF-8 TZ=UTC
COPY provision.sh /tmp/provision.sh
RUN bash /tmp/provision.sh && rm /tmp/provision.sh

USER agent
WORKDIR /home/agent
ENV PATH=/home/agent/.local/bin:/opt/fctl/bin:$PATH
CMD ["tail", "-f", "/dev/null"]
