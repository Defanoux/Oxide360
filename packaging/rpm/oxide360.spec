# Oxide360 as an RPM, for the Fedora COPR (dnf copr enable defanoux/oxide360).
# Built from the release tag's source. The Rust crates are fetched while
# building, so the COPR project has networking on for its builds.
#
# A new release: set Version, add a %changelog entry, then see
# packaging/rpm/README.md.

# (Rust's debug symbols would make a huge debuginfo package nobody uses.)
%global debug_package %{nil}

Name:           oxide360
Version:        0.3.2
Release:        1%{?dist}
Summary:        Parametric 3D CAD modeller in the spirit of Fusion 360
License:        GPL-3.0-or-later
URL:            https://github.com/Defanoux/Oxide360
Source0:        %{url}/archive/v%{version}/Oxide360-%{version}.tar.gz
ExclusiveArch:  x86_64

BuildRequires:  rust >= 1.88
BuildRequires:  cargo
BuildRequires:  gcc-c++
BuildRequires:  cmake
BuildRequires:  pkgconfig
BuildRequires:  opencascade-devel
BuildRequires:  desktop-file-utils

# Drawn with Vulkan; windows and keyboards through Wayland / X11 (opened at run time).
Requires:       vulkan-loader
Requires:       mesa-vulkan-drivers
Requires:       libxkbcommon
Requires:       libwayland-client
Requires:       hicolor-icon-theme
# File choosers come from the desktop's portal.
Recommends:     xdg-desktop-portal

%description
Oxide360 is a parametric 3D CAD modeller for Linux: sketches with
constraints and dimensions; extrude, revolve, sweep, loft, coil, holes,
threads, fillets, shells and patterns; a timeline of every step; components
and joints; measure and print checks; STEP / STL / 3MF / OBJ export, STEP /
STL / OpenSCAD import; designs on your own TrueNAS or Nextcloud. The
geometry is exact (OpenCASCADE B-rep).

%prep
%autosetup -n Oxide360-%{version}

%build
export CARGO_HOME=%{_builddir}/cargo-home
cargo build --release --locked

%install
install -Dm755 target/release/oxide360 %{buildroot}%{_bindir}/oxide360
install -Dm644 assets/oxide360.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/io.github.Defanoux.Oxide360.svg
install -Dm644 assets/oxide360-256.png %{buildroot}%{_datadir}/icons/hicolor/256x256/apps/io.github.Defanoux.Oxide360.png
install -Dm644 packaging/flatpak/io.github.Defanoux.Oxide360.desktop %{buildroot}%{_datadir}/applications/io.github.Defanoux.Oxide360.desktop
install -Dm644 packaging/flatpak/io.github.Defanoux.Oxide360.metainfo.xml %{buildroot}%{_metainfodir}/io.github.Defanoux.Oxide360.metainfo.xml
install -Dm644 packaging/flatpak/io.github.Defanoux.Oxide360.mime.xml %{buildroot}%{_datadir}/mime/packages/io.github.Defanoux.Oxide360.xml

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/io.github.Defanoux.Oxide360.desktop

%files
%license LICENSE
%doc README.md
%{_bindir}/oxide360
%{_datadir}/applications/io.github.Defanoux.Oxide360.desktop
%{_datadir}/icons/hicolor/scalable/apps/io.github.Defanoux.Oxide360.svg
%{_datadir}/icons/hicolor/256x256/apps/io.github.Defanoux.Oxide360.png
%{_metainfodir}/io.github.Defanoux.Oxide360.metainfo.xml
%{_datadir}/mime/packages/io.github.Defanoux.Oxide360.xml

%changelog
* Fri Nov 13 2026 Defanoux <128421570+Defanoux@users.noreply.github.com> - 0.3.2-1
- First RPM: fix for big coils on smaller graphics cards, crash logs,
  OpenSCAD import, own-cloud saving
