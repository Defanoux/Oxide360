#!/usr/bin/env bash
# Build Oxide360's RPM on Fedora COPR from the release tag's source.
# Run after the GitHub release (the tag must be pushed):  packaging/rpm/copr.sh
# One-time setup: packaging/rpm/README.md.
set -euo pipefail
cd "$(dirname "$0")"
VERSION=$(grep -m1 '^Version:' oxide360.spec | awk '{print $2}')
[ "$VERSION" = "$(grep -m1 '^version' ../../Cargo.toml | cut -d'"' -f2)" ] || { echo "oxide360.spec's Version ($VERSION) isn't Cargo.toml's: update it (and its %changelog)"; exit 1; }
USER_NAME=$(copr-cli whoami)
PROJECT="$USER_NAME/oxide360"
TOP=$(mktemp -d)
trap 'rm -rf "$TOP"' EXIT
mkdir -p "$TOP/SOURCES"
curl -sfL -o "$TOP/SOURCES/Oxide360-$VERSION.tar.gz" "https://github.com/Defanoux/Oxide360/archive/v$VERSION/Oxide360-$VERSION.tar.gz" \
    || { echo "No v$VERSION tag on GitHub yet: make the release first"; exit 1; }
rpmbuild --define "_topdir $TOP" -bs oxide360.spec >/dev/null
SRPM=$(ls "$TOP"/SRPMS/*.src.rpm)
# The project (once): Fedora releases to build for; networking on so cargo can fetch crates.
copr-cli get "$PROJECT" >/dev/null 2>&1 || copr-cli create oxide360 \
    --chroot fedora-44-x86_64 --chroot fedora-45-x86_64 \
    --enable-net on --description "Parametric 3D CAD for Linux, in the spirit of Fusion 360" \
    --instructions "sudo dnf copr enable $PROJECT && sudo dnf install oxide360"
copr-cli build "$PROJECT" "$SRPM"
echo "Built on COPR: https://copr.fedorainfracloud.org/coprs/$PROJECT/"
