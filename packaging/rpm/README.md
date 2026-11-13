# Oxide360 on Fedora COPR

Fedora users install it with:

```sh
sudo dnf copr enable <copr-user>/oxide360
sudo dnf install oxide360
```

After that, Discover shows updates for it like any other Fedora package.

## One-time setup (Gavin)

1. A Fedora account: https://accounts.fedoraproject.org (free).
2. Log in at https://copr.fedorainfracloud.org, then open **API** (top right)
   and copy the shown config into `~/.config/copr`.
3. `sudo dnf install copr-cli rpm-build`

## Each release

After the GitHub release (step 5 in docs/PROJECT.md):

1. Set `Version:` in `oxide360.spec` to the new version and add a
   `%changelog` entry at the top of the list.
2. `packaging/rpm/copr.sh` — makes the source RPM from the tag and builds it
   on COPR for Fedora 44 and 45 (they have OpenCASCADE 7.9) (about 15 minutes there). The first run
   also creates the COPR project (networking on, so cargo can fetch crates).
