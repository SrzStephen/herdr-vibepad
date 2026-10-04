# Development and install commands for herdr-vibepad. Run `just` to list them.

set shell := ["bash", "-euo", "pipefail", "-c"]

bin_dir := env("HOME") / ".local/bin"
udev_rule := "/etc/udev/rules.d/70-side-keyboard.rules"
user_unit := env("HOME") / ".config/systemd/user/herdr-vibepad.service"

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

# Build dist/herdr-vibepad_<version>_amd64.deb
deb:
    scripts/build-deb.sh

# Build the .deb, install it in a clean container and smoke-test it
deb-test image="ubuntu:24.04": deb
    docker run --rm -v "$PWD/dist:/dist:ro" -v "$PWD/scripts:/scripts:ro" {{ image }} /scripts/deb-smoke-test.sh

# Build the .deb and install it on this machine (replaces `just install`)
install-deb: deb
    -just uninstall
    sudo apt-get install -y --reinstall ./dist/herdr-vibepad_*.deb
    systemctl --user daemon-reload
    systemctl --user enable herdr-vibepad
    systemctl --user restart herdr-vibepad

# Install compiled binaries, plus the udev rule and user service
install:
    cargo build --release --locked
    install -Dm755 target/release/herdr-vibepad {{ bin_dir }}/herdr-vibepad
    install -Dm755 target/release/side-keyboard-keys {{ bin_dir }}/side-keyboard-keys
    install -Dm755 target/release/side-keyboard-led {{ bin_dir }}/side-keyboard-led
    sudo install -m 644 packaging/70-side-keyboard.rules {{ udev_rule }}
    sudo udevadm control --reload-rules
    sudo udevadm trigger --action=change --subsystem-match=hidraw --subsystem-match=input --property-match=ID_VENDOR_ID=6d7d
    rm -f {{ user_unit }}
    mkdir -p "$(dirname {{ user_unit }})"
    sed "s|^ExecStart=.*|ExecStart={{ bin_dir }}/herdr-vibepad|" packaging/herdr-vibepad.service > {{ user_unit }}
    systemctl --user daemon-reload
    systemctl --user reenable herdr-vibepad
    systemctl --user restart herdr-vibepad

# Undo `just install`
uninstall:
    -systemctl --user disable --now herdr-vibepad
    rm -f {{ user_unit }}
    systemctl --user daemon-reload
    sudo rm -f {{ udev_rule }}
    rm -f {{ bin_dir }}/herdr-vibepad {{ bin_dir }}/side-keyboard-keys {{ bin_dir }}/side-keyboard-led

# Follow the service log
logs:
    journalctl --user -u herdr-vibepad -f

# Restart the service (run `just install` after editing Rust code, since there's no editable install)
restart:
    systemctl --user restart herdr-vibepad

# Show the service status
status:
    systemctl --user status herdr-vibepad

# Remove build output and caches
clean:
    rm -rf build dist target
