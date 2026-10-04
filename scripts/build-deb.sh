#!/bin/sh
# Build dist/agentpad_<version>_all.deb from this checkout (version from pyproject.toml).
set -eu
umask 022
cd "$(dirname "$0")/.."

version=$(python3 -c 'import tomllib; print(tomllib.load(open("pyproject.toml", "rb"))["project"]["version"])')
root=build/deb
rm -rf "$root"

pkg=$root/usr/lib/python3/dist-packages/agentpad
install -d "$pkg" "$root/usr/bin" "$root/DEBIAN"
install -m 644 src/agentpad/*.py "$pkg/"
for entry in agentpad:daemon side-keyboard-keys:keys side-keyboard-led:led; do
    printf '#!/usr/bin/python3\nfrom agentpad.%s import main\n\nmain()\n' "${entry#*:}" > "$root/usr/bin/${entry%%:*}"
    chmod 755 "$root/usr/bin/${entry%%:*}"
done
install -D -m 644 packaging/70-side-keyboard.rules "$root/usr/lib/udev/rules.d/70-side-keyboard.rules"
install -D -m 644 packaging/agentpad.service "$root/usr/lib/systemd/user/agentpad.service"
install -D -m 644 README.md "$root/usr/share/doc/agentpad/README.md"

size=$(du -sk --exclude=DEBIAN "$root" | cut -f1)
sed -e "s/@VERSION@/$version/" -e "s/@SIZE@/$size/" packaging/deb/control > "$root/DEBIAN/control"
install -m 755 packaging/deb/postinst packaging/deb/prerm "$root/DEBIAN/"

mkdir -p dist
dpkg-deb --root-owner-group --build "$root" "dist/agentpad_${version}_all.deb"
