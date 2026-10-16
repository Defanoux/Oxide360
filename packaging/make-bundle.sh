#!/usr/bin/env bash
# Build a folder anyone on Fedora (x86_64) can run opencad from, without
# installing OpenCASCADE: the release binary plus the OCCT and TBB libraries
# it loads, a launcher, and an installer for the app menu.
#
#   packaging/make-bundle.sh        → dist/opencad-<version>-fedora-x86_64.tar.gz
#
# On the other machine:
#   tar xf opencad-*.tar.gz && cd opencad && ./install.sh   (adds it to the app menu)
#   or just: ./opencad
set -euo pipefail
cd "$(dirname "$0")/.."

version=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
name="opencad-${version}-fedora-x86_64"
out="dist/opencad"

cargo build --release
rm -rf "$out" && mkdir -p "$out/lib"
cp target/release/opencad "$out/opencad.bin"
strip "$out/opencad.bin" 2>/dev/null || true

# The libraries a stock Fedora desktop doesn't have: OpenCASCADE's (libTK*)
# and Intel TBB, which it uses. Everything else (glibc, the graphics stack,
# fonts) comes from the machine it runs on.
ldd target/release/opencad | awk '/=> \// {print $3}' | grep -E '/lib(TK[^/]*|tbb[^/]*)\.so' | while read -r lib; do
    cp -L "$lib" "$out/lib/"
done
# And what those pull in that isn't stock either.
for lib in "$out"/lib/*.so*; do
    ldd "$lib" | awk '/=> \// {print $3}' | grep -E '/lib(TK[^/]*|tbb[^/]*)\.so' || true
done | sort -u | while read -r lib; do
    [ -e "$out/lib/$(basename "$lib")" ] || cp -L "$lib" "$out/lib/"
done

cat > "$out/opencad" <<'EOF'
#!/usr/bin/env bash
# Run opencad with the libraries bundled next to it.
here="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
export LD_LIBRARY_PATH="$here/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
exec "$here/opencad.bin" "$@"
EOF
chmod +x "$out/opencad"

cat > "$out/install.sh" <<'EOF'
#!/usr/bin/env bash
# Put opencad in the app menu (for this user; nothing outside your home).
set -euo pipefail
here="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
apps="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
mkdir -p "$apps" "$HOME/.local/bin"
ln -sf "$here/opencad" "$HOME/.local/bin/opencad"
cat > "$apps/opencad.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=opencad
Comment=Parametric 3D CAD
Exec=$here/opencad %f
Icon=applications-engineering
Terminal=false
Categories=Graphics;Engineering;3DGraphics;
StartupWMClass=opencad
DESKTOP
update-desktop-database "$apps" 2>/dev/null || true
echo "opencad is in your app menu (and 'opencad' runs it from a terminal)."
echo "Keep this folder where it is: the menu entry points here."
EOF
chmod +x "$out/install.sh"

cat > "$out/README.txt" <<'EOF'
opencad — a parametric 3D CAD program (Fedora, x86_64)

Run it:            ./opencad
Add to app menu:   ./install.sh

Needs a 64-bit Fedora with a desktop (tested on Fedora 44, KDE). The 3D view
uses Vulkan, which Fedora's Mesa drivers provide out of the box. Nothing needs
installing: the CAD kernel's libraries are in lib/.
EOF

tar -C dist -czf "dist/${name}.tar.gz" opencad
du -h "dist/${name}.tar.gz"
echo "Built dist/${name}.tar.gz"
