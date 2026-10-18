#!/usr/bin/env bash
# Build the Oxide360 Flatpak and a single-file installer, Oxide360.flatpak
# (double-click it, or: flatpak install --user Oxide360.flatpak).
# Needs: sudo dnf install flatpak-builder
# The first build compiles OpenCASCADE too (half an hour or so); later ones reuse it.
set -euo pipefail
cd "$(dirname "$0")"
flatpak-builder --user --install-deps-from=flathub --force-clean --ccache \
    --repo=repo build io.github.Defanoux.Oxide360.yml
flatpak build-bundle repo ../../dist/Oxide360.flatpak io.github.Defanoux.Oxide360 \
    --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo
du -h ../../dist/Oxide360.flatpak
echo "Built dist/Oxide360.flatpak — double-click it to install."
