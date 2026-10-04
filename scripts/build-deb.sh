#!/bin/sh
# Build dist/herdr-vibepad_<version>_amd64.deb from this checkout (version from Cargo.toml).
set -eu
umask 022
cd "$(dirname "$0")/.."

version=$(grep '^version' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
root=build/deb
rm -rf "$root"

install -d "$root/usr/bin" "$root/DEBIAN"
cargo build --release --locked
install -D -m 755 target/release/herdr-vibepad "$root/usr/bin/herdr-vibepad"
install -D -m 755 target/release/side-keyboard-keys "$root/usr/bin/side-keyboard-keys"
install -D -m 755 target/release/side-keyboard-led "$root/usr/bin/side-keyboard-led"
install -D -m 644 packaging/70-side-keyboard.rules "$root/usr/lib/udev/rules.d/70-side-keyboard.rules"
install -D -m 644 packaging/herdr-vibepad.service "$root/usr/lib/systemd/user/herdr-vibepad.service"
install -D -m 644 README.md "$root/usr/share/doc/herdr-vibepad/README.md"

size=$(du -sk --exclude=DEBIAN "$root" | cut -f1)
sed -e "s/@VERSION@/$version/" -e "s/@SIZE@/$size/" packaging/deb/control > "$root/DEBIAN/control"
install -m 755 packaging/deb/postinst packaging/deb/prerm "$root/DEBIAN/"

mkdir -p dist
dpkg-deb --root-owner-group --build "$root" "dist/herdr-vibepad_${version}_amd64.deb"
