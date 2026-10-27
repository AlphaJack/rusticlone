# syntax=docker/dockerfile:1
# The computer the screenshots show: rusticlone built from this checkout, rustic, vhs to record its terminal, dunst to show its desktop
# notifications, and an Xfce panel to show its tray icon.

FROM rust:1-alpine AS build
ARG RUSTIC
RUN apk add --no-cache musl-dev && \
    wget -qO- "https://github.com/rustic-rs/rustic/releases/download/${RUSTIC}/rustic-${RUSTIC}-$(uname -m)-unknown-linux-musl.tar.gz" | tar -xz -C / rustic
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry --mount=type=cache,target=/src/target \
    cargo build --release --locked --no-default-features --features desktop,self-update && cp target/release/rusticlone /

FROM ghcr.io/charmbracelet/vhs
RUN apt-get update && apt-get install -y --no-install-recommends xvfb x11-xserver-utils dbus-x11 dunst imagemagick pngquant fonts-inter xfce4-panel xdotool python3-dbus python3-gi && \
    rm -rf /var/lib/apt/lists/* && useradd -m -s /bin/bash jack && install -d -o jack /mnt/hdd/backups /mnt/gdrive /mnt/b2
COPY --chown=jack <<'BASHRC' /home/jack/.bashrc
PS1='\[\e[1;32m\]\u@\h\[\e[0m\]:\[\e[1;34m\]\w\[\e[0m\]\$ '
BASHRC
COPY --from=build /rusticlone /rustic /usr/local/bin/
ENV RUSTICLONE_LISTEN=0.0.0.0:32110
