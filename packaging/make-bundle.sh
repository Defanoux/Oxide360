#!/usr/bin/env bash
# Build a folder anyone on Fedora (x86_64) can run oxide360 from, without
# installing OpenCASCADE: the release binary plus the OCCT and TBB libraries
# it loads, a launcher, and an installer for the app menu.
#
#   packaging/make-bundle.sh        → dist/oxide360-<version>-fedora-x86_64.tar.gz
#
# On the other machine:
#   tar xf oxide360-*.tar.gz && cd oxide360 && ./install.sh   (adds it to the app menu)
#   or just: ./oxide360
set -euo pipefail
cd "$(dirname "$0")/.."

version=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
name="oxide360-${version}-fedora-x86_64"
out="dist/oxide360"

cargo build --release
rm -rf "$out" && mkdir -p "$out/lib"
cp target/release/oxide360 "$out/oxide360.bin"
cp assets/oxide360.svg "$out/oxide360.svg"
strip "$out/oxide360.bin" 2>/dev/null || true

# The libraries a stock Fedora desktop doesn't have: OpenCASCADE's (libTK*)
# and Intel TBB, which it uses. Everything else (glibc, the graphics stack,
# fonts) comes from the machine it runs on.
ldd target/release/oxide360 | awk '/=> \// {print $3}' | grep -E '/lib(TK[^/]*|tbb[^/]*)\.so' | while read -r lib; do
    cp -L "$lib" "$out/lib/"
done
# And what those pull in that isn't stock either.
for lib in "$out"/lib/*.so*; do
    ldd "$lib" | awk '/=> \// {print $3}' | grep -E '/lib(TK[^/]*|tbb[^/]*)\.so' || true
done | sort -u | while read -r lib; do
    [ -e "$out/lib/$(basename "$lib")" ] || cp -L "$lib" "$out/lib/"
done

cat > "$out/oxide360" <<'EOF'
#!/usr/bin/env bash
# Run oxide360 with the libraries bundled next to it.
here="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
export LD_LIBRARY_PATH="$here/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
exec "$here/oxide360.bin" "$@"
EOF
chmod +x "$out/oxide360"

cat > "$out/install.sh" <<'EOF'
#!/usr/bin/env bash
# Put oxide360 in the app menu (for this user; nothing outside your home).
set -euo pipefail
here="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
apps="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
mkdir -p "$apps" "$HOME/.local/bin"
ln -sf "$here/oxide360" "$HOME/.local/bin/oxide360"
cat > "$apps/oxide360.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Oxide360
Comment=Parametric 3D CAD
Exec=$here/oxide360 %f
Icon=$here/oxide360.svg
Terminal=false
Categories=Graphics;Engineering;3DGraphics;
StartupWMClass=oxide360
DESKTOP
update-desktop-database "$apps" 2>/dev/null || true
echo "Oxide360 is in your app menu (and 'oxide360' runs it from a terminal)."
echo "Keep this folder where it is: the menu entry points here."
EOF
chmod +x "$out/install.sh"

cat > "$out/README.txt" <<'EOF'
Oxide360 — a parametric 3D CAD program (Fedora, x86_64)

Run it:            ./oxide360
Add to app menu:   ./install.sh

Needs a 64-bit Fedora with a desktop (tested on Fedora 44, KDE). The 3D view
uses Vulkan, which Fedora's Mesa drivers provide out of the box. Nothing needs
installing: the CAD kernel's libraries are in lib/.
EOF

tar -C dist -czf "dist/${name}.tar.gz" oxide360
du -h "dist/${name}.tar.gz"
echo "Built dist/${name}.tar.gz"
