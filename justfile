# Development and install commands for agentpad. Run `just` to list them.

set shell := ["bash", "-euo", "pipefail", "-c"]

udev_rule := "/etc/udev/rules.d/70-side-keyboard.rules"
user_unit := env("HOME") / ".config/systemd/user/agentpad.service"

# List recipes
default:
    @just --list

# Create .venv with the dev tools (pytest, ruff)
sync:
    uv sync

# Lint Python (ruff) and shell scripts (shellcheck)
lint:
    uv run ruff check .
    uv run ruff format --check .
    shellcheck scripts/*.sh packaging/deb/postinst packaging/deb/prerm

# Format Python and apply safe lint fixes
fmt:
    uv run ruff format .
    uv run ruff check --fix .

# Run the tests (extra arguments go to pytest)
test *args:
    uv run pytest {{ args }}

# Lint and test
check: lint test

# Regenerate the README diagrams in docs/ from the daemon's layout and colours
diagrams:
    uv run python scripts/diagrams.py

# Build dist/agentpad_<version>_all.deb
deb:
    scripts/build-deb.sh

# Build the .deb, install it in a clean container and smoke-test it
deb-test image="ubuntu:24.04": deb
    docker run --rm -v "$PWD/dist:/dist:ro" -v "$PWD/scripts:/scripts:ro" {{ image }} /scripts/deb-smoke-test.sh

# Build the .deb and install it on this machine (replaces `just install`)
install-deb: deb
    -just uninstall
    sudo apt-get install -y --reinstall ./dist/agentpad_*_all.deb
    systemctl --user daemon-reload
    systemctl --user enable agentpad
    systemctl --user restart agentpad

# Install from this checkout with uv (editable), plus the udev rule and user service
install:
    uv tool install --force --editable .
    sudo install -m 644 packaging/70-side-keyboard.rules {{ udev_rule }}
    sudo udevadm control --reload-rules
    sudo udevadm trigger --action=change --subsystem-match=hidraw --subsystem-match=input --property-match=ID_VENDOR_ID=6d7d
    rm -f {{ user_unit }}
    mkdir -p "$(dirname {{ user_unit }})"
    sed "s|^ExecStart=.*|ExecStart=$(uv tool dir --bin --color never)/agentpad|" packaging/agentpad.service > {{ user_unit }}
    systemctl --user daemon-reload
    systemctl --user reenable agentpad
    systemctl --user restart agentpad

# Undo `just install`
uninstall:
    -systemctl --user disable --now agentpad
    rm -f {{ user_unit }}
    systemctl --user daemon-reload
    sudo rm -f {{ udev_rule }}
    -uv tool uninstall agentpad

# Follow the service log
logs:
    journalctl --user -u agentpad -f

# Restart the service (after editing the code with `just install`'s editable install)
restart:
    systemctl --user restart agentpad

# Show the service status
status:
    systemctl --user status agentpad

# Remove build output and caches
clean:
    rm -rf build dist .pytest_cache .ruff_cache
