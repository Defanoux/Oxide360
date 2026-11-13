#!/usr/bin/env bash
# Build the Oxide360 Flatpak and a single-file installer, Oxide360.flatpak
# (double-click it, or: flatpak install --user Oxide360.flatpak).
# Needs: sudo dnf install flatpak-builder
# The first build compiles OpenCASCADE too (half an hour or so); later ones reuse it.
set -euo pipefail
cd "$(dirname "$0")"
flatpak-builder --user --install-deps-from=flathub --force-clean --ccache \
    --repo=repo build io.github.Defanoux.Oxide360.yml
# The bundle remembers Oxide360's own repository (publish.sh), so installing
# the downloaded file also gets updates from there.
GNUPG=${OXIDE360_GNUPG:-$HOME/.local/share/oxide360-signing/gnupg}
KEYFILE=$(mktemp)
trap 'rm -f "$KEYFILE"' EXIT
REPO_ARGS=()
if gpg --homedir "$GNUPG" --export > "$KEYFILE" 2>/dev/null && [ -s "$KEYFILE" ]; then
    REPO_ARGS=(--repo-url=https://defanoux.github.io/Oxide360/repo/ --gpg-keys="$KEYFILE")
fi
flatpak build-bundle repo ../../dist/Oxide360.flatpak io.github.Defanoux.Oxide360 \
    --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo "${REPO_ARGS[@]}"
du -h ../../dist/Oxide360.flatpak
echo "Built dist/Oxide360.flatpak — double-click it to install."
