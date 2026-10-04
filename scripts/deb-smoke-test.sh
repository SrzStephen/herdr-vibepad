#!/bin/sh
# Install the built .deb (mounted at /dist) in a clean Debian/Ubuntu container and check it.
set -eux
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq /dist/agentpad_*_all.deb >/dev/null

agentpad --help | grep -q "herdr"
side-keyboard-keys 2>&1 | grep -q "Set what each key"
side-keyboard-led 2>&1 | grep -q "Control the LEDs"
python3 -c "import agentpad.daemon"

udevadm verify /usr/lib/udev/rules.d/70-side-keyboard.rules
systemd-analyze verify /usr/lib/systemd/user/agentpad.service
test -L /etc/systemd/user/default.target.wants/agentpad.service

apt-get remove -y -qq agentpad >/dev/null
test ! -e /etc/systemd/user/default.target.wants/agentpad.service
test ! -e /usr/bin/agentpad
echo "deb smoke test passed"
