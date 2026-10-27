# Generated from src/settings.rs by cargo xtask generate
ARG NAME=rusticlone
ARG VERSION
ARG REPOSITORY=AlphaJack/rusticlone
ARG RUSTIC=v0.11.4

FROM --platform=$BUILDPLATFORM alpine:latest AS fetch
ARG NAME
ARG VERSION
ARG REPOSITORY
ARG RUSTIC
ARG TARGETARCH
WORKDIR /work
RUN set -eu; \
    test -n "${VERSION}" || { echo "pass --build-arg VERSION=x.y.z" >&2; exit 1; }; \
    case "${TARGETARCH}" in \
      amd64) triple=x86_64-unknown-linux-musl ;; \
      arm64) triple=aarch64-unknown-linux-musl ;; \
      arm) triple=armv7-unknown-linux-musleabihf ;; \
      *) echo "unsupported platform: ${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    fetch() { wget -q "$1/$2" "$1/$2.sha256"; echo "$(cut -d ' ' -f 1 "$2.sha256")  $2" | sha256sum -c -; }; \
    fetch "https://github.com/${REPOSITORY}/releases/download/v${VERSION#v}" "${NAME}-${triple}.tar.gz"; \
    fetch "https://github.com/rustic-rs/rustic/releases/download/${RUSTIC}" "rustic-${RUSTIC}-${triple}.tar.gz"; \
    mkdir out; tar -xzf "${NAME}-${triple}.tar.gz" -C out; tar -xzf "rustic-${RUSTIC}-${triple}.tar.gz" -C out; \
    install -m 0755 "$(find out -type f -name "${NAME}" | head -n 1)" "/${NAME}"; \
    install -m 0755 "$(find out -type f -name rustic | head -n 1)" /rustic

FROM alpine:latest
ARG NAME
ARG VERSION
LABEL org.opencontainers.image.title="${NAME}" \
      org.opencontainers.image.description="3-2-1-1-0 backups using rustic profiles" \
      org.opencontainers.image.licenses="AGPL-3.0-or-later" \
      org.opencontainers.image.version="${VERSION}"
RUN apk add --no-cache ca-certificates tzdata rclone openssh-client
COPY --from=fetch /${NAME} /rustic /usr/local/bin/
ENV RUSTICLONE_LISTEN=0.0.0.0:32110 \
    RUSTICLONE_STATE_DIR=/var/lib/${NAME}
VOLUME ["/etc/rustic", "/var/lib/${NAME}"]
EXPOSE 32110
HEALTHCHECK --interval=30s --timeout=10s --start-period=10s --retries=3 CMD ["rusticlone", "healthcheck"]
ENTRYPOINT ["rusticlone"]
CMD ["serve"]
