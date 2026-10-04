#!/bin/sh
# Install the built .deb (mounted at /dist) in a clean Debian/Ubuntu container and check it.
set -eux
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq /dist/herdr-vibepad_*.deb >/dev/null

herdr-vibepad --help | grep -q "herdr"
side-keyboard-keys 2>&1 | grep -q "Set what each key"
side-keyboard-led 2>&1 | grep -q "Control the LEDs"

udevadm verify /usr/lib/udev/rules.d/70-side-keyboard.rules
systemd-analyze verify /usr/lib/systemd/user/herdr-vibepad.service
test -L /etc/systemd/user/default.target.wants/herdr-vibepad.service

apt-get remove -y -qq herdr-vibepad >/dev/null
test ! -e /etc/systemd/user/default.target.wants/herdr-vibepad.service
test ! -e /usr/bin/herdr-vibepad
echo "deb smoke test passed"
