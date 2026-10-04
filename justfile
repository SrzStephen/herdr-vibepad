# Development and install commands for agentpad. Run `just` to list them.

set shell := ["bash", "-euo", "pipefail", "-c"]

bin_dir := env("HOME") / ".local/bin"
udev_rule := "/etc/udev/rules.d/70-side-keyboard.rules"
user_unit := env("HOME") / ".config/systemd/user/agentpad.service"

# List recipes
default:
    @just --list

# Fetch Cargo dependencies
sync:
    cargo fetch

# Lint Rust (clippy, fmt) and shell scripts (shellcheck)
lint:
    cargo clippy --all-targets --locked -- -D warnings
    cargo fmt --check
    shellcheck scripts/*.sh packaging/deb/postinst packaging/deb/prerm

# Format Rust and apply safe lint fixes
fmt:
    cargo fmt
    cargo clippy --fix --allow-dirty --allow-staged

# Run the tests (extra arguments go to cargo test)
test *args:
    cargo test --locked {{ args }}

# Lint and test
check: lint test

# Regenerate the README diagrams in docs/ from the daemon's layout and colours
diagrams:
    cargo run --release --locked --bin diagrams

# Build dist/agentpad_<version>_amd64.deb
deb:
    scripts/build-deb.sh

# Build the .deb, install it in a clean container and smoke-test it
deb-test image="ubuntu:24.04": deb
    docker run --rm -v "$PWD/dist:/dist:ro" -v "$PWD/scripts:/scripts:ro" {{ image }} /scripts/deb-smoke-test.sh

# Build the .deb and install it on this machine (replaces `just install`)
install-deb: deb
    -just uninstall
    sudo apt-get install -y --reinstall ./dist/agentpad_*.deb
    systemctl --user daemon-reload
    systemctl --user enable agentpad
    systemctl --user restart agentpad

# Install compiled binaries, plus the udev rule and user service
install:
    cargo build --release --locked
    install -Dm755 target/release/agentpad {{ bin_dir }}/agentpad
    install -Dm755 target/release/side-keyboard-keys {{ bin_dir }}/side-keyboard-keys
    install -Dm755 target/release/side-keyboard-led {{ bin_dir }}/side-keyboard-led
    sudo install -m 644 packaging/70-side-keyboard.rules {{ udev_rule }}
    sudo udevadm control --reload-rules
    sudo udevadm trigger --action=change --subsystem-match=hidraw --subsystem-match=input --property-match=ID_VENDOR_ID=6d7d
    rm -f {{ user_unit }}
    mkdir -p "$(dirname {{ user_unit }})"
    sed "s|^ExecStart=.*|ExecStart={{ bin_dir }}/agentpad|" packaging/agentpad.service > {{ user_unit }}
    systemctl --user daemon-reload
    systemctl --user reenable agentpad
    systemctl --user restart agentpad

# Undo `just install`
uninstall:
    -systemctl --user disable --now agentpad
    rm -f {{ user_unit }}
    systemctl --user daemon-reload
    sudo rm -f {{ udev_rule }}
    rm -f {{ bin_dir }}/agentpad {{ bin_dir }}/side-keyboard-keys {{ bin_dir }}/side-keyboard-led

# Follow the service log
logs:
    journalctl --user -u agentpad -f

# Restart the service (run `just install` after editing Rust code, since there's no editable install)
restart:
    systemctl --user restart agentpad

# Show the service status
status:
    systemctl --user status agentpad

# Remove build output and caches
clean:
    rm -rf build dist .pytest_cache .ruff_cache
