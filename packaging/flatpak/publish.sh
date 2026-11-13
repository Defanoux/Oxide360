#!/usr/bin/env bash
# Publish the Flatpak built by build.sh as Oxide360's own Flatpak
# repository on GitHub Pages (https://defanoux.github.io/Oxide360/), signed,
# so Discover / GNOME Software / `flatpak update` keep it up to date.
#
# Run after build.sh, from any folder:  packaging/flatpak/publish.sh
#
# The signing key lives OUTSIDE the project (never committed):
#   ~/.local/share/oxide360-signing/gnupg   (keep a backup of this folder —
#   without it, updates can't be signed and everyone has to re-install)
#
# What goes up (the gh-pages branch, replaced each time — one commit):
#   repo/                 the OSTree repository (app only, no debug symbols)
#   Oxide360.flatpakref   one click / `flatpak install <url>` installs it
#   oxide360.flatpakrepo  adds the repository by itself
#   index.html            how to install
set -euo pipefail
cd "$(dirname "$0")"

APP=io.github.Defanoux.Oxide360
BRANCH=master
URL=https://defanoux.github.io/Oxide360
GNUPG=${OXIDE360_GNUPG:-$HOME/.local/share/oxide360-signing/gnupg}
KEY=$(gpg --homedir "$GNUPG" --list-secret-keys --with-colons | awk -F: '/^fpr/ {print $10; exit}')
[ -n "$KEY" ] || { echo "No signing key in $GNUPG"; exit 1; }
[ -d repo/refs ] || { echo "No build: run build.sh first"; exit 1; }

# The published repository, kept between runs so updates are small deltas.
PUB=pages-repo
[ -d "$PUB/refs" ] || ostree --repo="$PUB" init --mode=archive-z2
flatpak build-commit-from --src-repo=repo --gpg-sign="$KEY" --gpg-homedir="$GNUPG" \
    --no-update-summary "$PUB" "app/$APP/x86_64/$BRANCH"
flatpak build-update-repo --gpg-sign="$KEY" --gpg-homedir="$GNUPG" \
    --title="Oxide360" --default-branch="$BRANCH" \
    --prune --prune-depth=3 --generate-static-deltas "$PUB"

GPGKEY=$(gpg --homedir "$GNUPG" --export "$KEY" | base64 -w0)

# The site, in a throwaway folder pushed as the only commit of gh-pages.
SITE=$(mktemp -d)
trap 'rm -rf "$SITE"' EXIT
cp -r "$PUB" "$SITE/repo"
rm -rf "$SITE/repo/tmp" "$SITE/repo/state"
mkdir -p "$SITE/repo/tmp"
touch "$SITE/.nojekyll"
cat > "$SITE/Oxide360.flatpakref" <<EOF
[Flatpak Ref]
Title=Oxide360
Name=$APP
Branch=$BRANCH
Url=$URL/repo/
SuggestRemoteName=oxide360
Homepage=https://github.com/Defanoux/Oxide360
Icon=$URL/icon.svg
RuntimeRepo=https://dl.flathub.org/repo/flathub.flatpakrepo
IsRuntime=false
GPGKey=$GPGKEY
EOF
cat > "$SITE/oxide360.flatpakrepo" <<EOF
[Flatpak Repo]
Title=Oxide360
Url=$URL/repo/
Homepage=https://github.com/Defanoux/Oxide360
Comment=Parametric 3D CAD for Linux
Icon=$URL/icon.svg
GPGKey=$GPGKEY
EOF
cp ../../assets/oxide360.svg "$SITE/icon.svg"
VERSION=$(grep -m1 '^version' ../../Cargo.toml | cut -d'"' -f2)
sed "s/@VERSION@/$VERSION/g" site-index.html > "$SITE/index.html"

# OXIDE360_PUBLISH_DRY=1: build the site into site-preview/ and stop (nothing pushed).
if [ -n "${OXIDE360_PUBLISH_DRY:-}" ]; then
    rm -rf site-preview && cp -r "$SITE" site-preview
    echo "Dry run: the site is in packaging/flatpak/site-preview (nothing pushed)"
    exit 0
fi

cd "$SITE"
git init -q -b gh-pages
git -c user.name=Defanoux -c user.email=128421570+Defanoux@users.noreply.github.com add -A
git -c user.name=Defanoux -c user.email=128421570+Defanoux@users.noreply.github.com commit -q -m "Flatpak repository: Oxide360 $VERSION"
git push -q -f "$(git -C "$OLDPWD" remote get-url origin)" gh-pages
echo "Published Oxide360 $VERSION to $URL/"
