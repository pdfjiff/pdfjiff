# Minimal pdfjiff image: one static binary, nothing else (no shell, no package manager).
#
#   docker run --rm --user "$(id -u):$(id -g)" -v "$PWD:/work" ghcr.io/pdfjiff/pdfjiff compress report.pdf
#
# The release workflow builds this from the static musl release binaries, arranged as
# dist/docker/<os>/<arch>/pdfjiff, so no compiler runs inside the image build. To build
# locally, place a static Linux binary at dist/docker/linux/amd64/pdfjiff (or arm64) and
# generate the notices: cargo about generate --locked -o THIRD-PARTY-LICENSES.md about.hbs
FROM scratch
ARG TARGETPLATFORM
LABEL org.opencontainers.image.title="pdfjiff" \
      org.opencontainers.image.description="Fast, private PDF compression, merging and inspection" \
      org.opencontainers.image.licenses="MIT OR Apache-2.0"
COPY --chmod=755 dist/docker/${TARGETPLATFORM}/pdfjiff /usr/local/bin/pdfjiff
COPY LICENSE-MIT LICENSE-APACHE THIRD-PARTY-LICENSES.md /usr/share/licenses/pdfjiff/
WORKDIR /work
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/pdfjiff"]
CMD ["--help"]
