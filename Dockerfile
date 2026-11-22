# syntax=docker/dockerfile:1

# Intentionally use the target platform: each package is built natively (under
# QEMU when needed), including GTK tools and dpkg-shlibdeps.
FROM rust:1-trixie AS build
ARG TARGETARCH
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential libgtk-4-dev libadwaita-1-dev pkg-config \
    blueprint-compiler gettext dpkg-dev python3 ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
ARG DEB_MAINTAINER="Debby contributors <debby@localhost>"
ARG DEB_REVISION=1
RUN --mount=type=cache,id=debby-registry-${TARGETARCH},target=/usr/local/cargo/registry \
    --mount=type=cache,id=debby-git-${TARGETARCH},target=/usr/local/cargo/git \
    --mount=type=cache,id=debby-target-${TARGETARCH},target=/src/target \
    DEB_MAINTAINER="$DEB_MAINTAINER" DEB_REVISION="$DEB_REVISION" \
    bash scripts/build-deb.sh /artifacts

FROM scratch AS artifacts
COPY --from=build /artifacts/ /
