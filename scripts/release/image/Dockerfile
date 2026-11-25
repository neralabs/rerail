# A published Rerail, installed globally, for headless scans in CI: the image the reviewer's Rerail module runs in
# (.github/workflows/reviewer.yml). rerail-image.yml builds it for each release published to `next`, and once it passes
# a scan of this repository (smoke.sh) pushes it as ghcr.io/neralabs/rerail:<version> and moves `:next` to it.
#
#   docker build --build-arg RERAIL_VERSION=<version> scripts/release/image
#
# Node 24 is what Rerail needs (`engines`) and what the reviewer runs its own scripts with. git is for the scan and
# for actions/checkout inside the container, which falls back to the REST API without it and leaves no history.
FROM node:24-bookworm-slim@sha256:873f586f3794f52281bd039bf02c048a0be08fde619338f9941bad1eb6b34530

RUN apt-get update \
  && apt-get install --yes --no-install-recommends ca-certificates git \
  && rm -rf /var/lib/apt/lists/*

# An exact version, never a dist-tag: the image's tag names the Rerail it holds. npm installs all six engine packages
# core's shrinkwrap lists (docs/release.md, "Shrinkwrap"), about 700 MB; only this platform's is kept. smoke.sh's scan
# is what shows the kept engine loads.
ARG RERAIL_VERSION
RUN test -n "$RERAIL_VERSION" \
  && npm install --global "@neralabs/rerail@${RERAIL_VERSION}" \
  && npm cache clean --force \
  && engines="$(npm root --global)/@neralabs/rerail/node_modules/@neralabs" \
  && keep="rerail-engine-linux-$(node -p process.arch)-gnu" \
  && test -d "$engines/$keep" \
  && find "$engines" -mindepth 1 -maxdepth 1 -name 'rerail-engine-*' ! -name "$keep" -exec rm -rf {} + \
  && rerail --version

LABEL org.opencontainers.image.source="https://github.com/neralabs/rerail" \
  org.opencontainers.image.version="${RERAIL_VERSION}" \
  org.opencontainers.image.description="Rerail, as published to npm, for headless scans in CI" \
  org.opencontainers.image.licenses="Apache-2.0"
