# Rewrite agentpad in Rust, with working CI and a devcontainer

## Goal

Port `agentpad` — the daemon and two CLI tools that turn the SDINNOVATION
SIDE-KEYBOARD into a herdr controller — from Python to Rust, with no
behavior change, working GitHub Actions CI, and a devcontainer for local
development. When this is done, the repo has no Python left: no
`pyproject.toml`, no `uv.lock`, no `.venv`, no `src/agentpad/*.py`, no
`tests/*.py`.

## Scope

In scope:
- `src/agentpad/daemon.py` → the `agentpad` daemon binary
- `src/agentpad/keys.py` → the `side-keyboard-keys` binary
- `src/agentpad/led.py` → the `side-keyboard-led` binary
- `scripts/diagrams.py` → a `diagrams` dev-only binary (regenerates
  `docs/*.svg`; not packaged in the `.deb`)
- The full Python test suite (`tests/*.py`, `tests/conftest.py`), ported to
  `cargo test`
- `.github/workflows/ci.yml`: lint/test/build steps for Rust
- `packaging/`: `.deb` control file and build script adjusted for compiled
  binaries instead of installed `.py` files
- `.pre-commit-config.yaml`: Rust lint hooks in place of the ruff/uv hooks
- `justfile`: recipes re-targeted at `cargo`/the new binaries
- New: `.devcontainer/devcontainer.json`
- `README.md`: install instructions and any other Python-specific mentions

Out of scope / unchanged:
- `packaging/70-side-keyboard.rules`, `packaging/agentpad.service`,
  `packaging/deb/postinst`, `packaging/deb/prerm` — none reference Python
- Runtime behavior, protocol details, LED colors, key mappings, herdr
  JSON-RPC methods — this is a language port, not a behavior change
- `docs/*.svg` content (regenerated from the same logic, now in Rust)

## Architecture

One Cargo package, not a workspace — the current project is ~1200 lines
total including tests; a multi-crate workspace would be ceremony without
benefit at this size.

```
Cargo.toml
src/
  lib.rs       # re-exports the modules below
  hid.rs       # shared hidraw find/send/request helpers (keys.py and
               # led.py each currently duplicate find_device/send/request;
               # unify them here)
  keys.rs      # key-mapping protocol + slot/key parsing (keys.py)
  led.rs       # LED protocol (led.py)
  herdr.rs     # herdr JSON-RPC client over a Unix socket
  daemon.rs    # AgentPad/Pad/State: the daemon's behavior and event loop
src/bin/
  agentpad.rs             # thin main() calling daemon::run
  side-keyboard-keys.rs    # thin main() calling keys::run
  side-keyboard-led.rs     # thin main() calling led::run
  diagrams.rs              # dev-only: regenerates docs/*.svg, depends on
                            # daemon's real layout/color logic as a library
```

Each binary's `--help`/no-args output must still print the same usage text
the corresponding Python module's docstring prints today — that text is
part of the tool's interface (`deb-smoke-test.sh` greps for it) and is
hand-maintained per binary rather than derived from a CLI-parsing crate.

## Dependencies

- `serde` + `serde_json` for the herdr JSON-RPC protocol (request/response
  framing, matching `daemon.py`'s `herdr()`)
- `nix` for `ioctl` (`EVIOCGRAB`), non-blocking reads, and the
  `poll`/`select`-equivalent event loop (`Pad.events`)
- `std::os::unix::net::UnixStream` / `UnixListener` for the herdr socket —
  no separate socket crate
- No CLI-parsing crate (see Architecture above: hand-rolled arg parsing,
  matching the existing hand-written usage text)

This keeps the dependency tree small and well-vetted rather than
hand-rolling JSON and raw libc calls, while preserving the original
project's preference for a minimal footprint (the Python package shipped
with zero runtime dependencies).

## Testing strategy

Mirrors the existing Python tests directly — no mock/trait abstraction
layer over the hardware or the socket, matching how `tests/test_pad.py`
and `tests/conftest.py` already test through real OS primitives:

- `Pad::events` tests feed raw hidraw report bytes through file
  descriptors from `nix::unistd::pipe()`, the same technique
  `test_pad.py::feed()` uses with `os.pipe()`.
- The herdr client is tested against a real `std::os::unix::net::UnixListener`
  spun up on a background thread in the test, serving canned JSON
  responses — the same approach as `conftest.py`'s `FakeHerdr`
  (`socketserver.ThreadingUnixStreamServer`).
- `cargo test` replaces `pytest`; test coverage should match the existing
  suite's cases (position/slot mapping, key parsing round-trips, knob/key
  behavior, LED colors, brightness persistence).

## Packaging (`.deb`)

The current package is `Architecture: all` because it's pure Python.
Compiled Rust binaries are architecture-specific:

- `packaging/deb/control`: `Architecture: amd64`; drop the
  `python3 (>= 3.10)` dependency (keep `udev`, `systemd`)
- `scripts/build-deb.sh`: build release binaries
  (`cargo build --release`) and install the four compiled binaries into
  `usr/bin/` instead of installing `.py` files into
  `usr/lib/python3/dist-packages/agentpad` and writing Python shim
  launchers
- `packaging/70-side-keyboard.rules`, `packaging/agentpad.service`,
  `packaging/deb/postinst`, `packaging/deb/prerm`: unchanged
- `scripts/deb-smoke-test.sh`: drop the
  `python3 -c "import agentpad.daemon"` check (nothing to import anymore);
  keep the `--help`/usage-text greps and the systemd/udev checks

## CI (`.github/workflows/ci.yml`)

Same four jobs (`lint`, `test`, `deb`, `release`), re-targeted:

- `lint`: `cargo clippy -- -D warnings`, `cargo fmt --check`, plus the
  existing `shellcheck` for the packaging shell scripts
- `test`: `cargo test`, single job on Rust `stable` (drop the Python
  3.10/3.12/3.13 matrix — Rust has no equivalent axis worth testing here
  without a stated MSRV; add one only if a need shows up later)
- Toolchain setup: `dtolnay/rust-toolchain@stable` with `clippy` and
  `rustfmt` components, plus caching for `~/.cargo` and `target/`
  (e.g. `Swatinem/rust-cache`)
- `deb`/`release`: unchanged in structure — still `just deb-test` building
  and smoke-testing the `.deb` in a container, still tag-triggered release

## Devcontainer (new)

`.devcontainer/devcontainer.json`, base image `ubuntu:24.04` (matches the
CI runner and the `.deb`'s build/target OS exactly). Features:

- `ghcr.io/devcontainers/features/rust:1` — Rust toolchain
- `ghcr.io/SrzStephen/devcontainer-features/just:1` — installs both `just`
  and `just-lsp` (defaults: latest of each)
- `ghcr.io/devcontainers/features/docker-outside-of-docker:1` — so
  `just deb-test` (which shells out to `docker run`) works inside the
  container
- `shellcheck` and `prek` available (feature or postCreate install —
  implementation detail for the plan)

No hidraw/input device access inside the container (no physical pad
attached there) — same ceiling as CI: build, lint, and test, not run
against real hardware.

## Pre-commit (`.pre-commit-config.yaml`)

- Remove the `astral-sh/ruff-pre-commit` and `astral-sh/uv-pre-commit`
  repos/hooks
- Add local hooks running `cargo fmt --check` and
  `cargo clippy -- -D warnings` (no actively-maintained upstream
  pre-commit-rust repo to depend on instead)
- Keep the generic hygiene hooks (`pre-commit-hooks`: trailing-whitespace,
  end-of-file-fixer, check-yaml, check-toml, check-added-large-files,
  check-merge-conflict, check-case-conflict,
  check-executables-have-shebangs, mixed-line-ending) and `shellcheck-py`
  as-is

## `justfile`

- `sync` → `cargo fetch`
- `lint` → `cargo clippy -- -D warnings`, `cargo fmt --check`, `shellcheck
  scripts/*.sh packaging/deb/postinst packaging/deb/prerm` (unchanged)
- `fmt` → `cargo fmt`, `cargo clippy --fix`
- `test *args` → `cargo test {{ args }}`
- `diagrams` → run the new `diagrams` binary
- `deb`, `deb-test`: unchanged in shape (still call
  `scripts/build-deb.sh` / run the smoke test in a container)
- `install`: behavior changes, not just syntax. Python's
  `uv tool install --editable .` means edits are picked up without a
  reinstall step; Rust has no editable-install equivalent. `install`
  becomes: build a release binary, install it to a user bin dir (e.g. via
  `cargo install --path . --root ~/.local`), point the service unit's
  `ExecStart` at that path. Editing code now requires re-running
  `just install` (or a new `just dev`/`just build` step) to pick up
  changes — `just restart` alone is no longer enough. The plan should add
  whichever of these best matches the existing `just restart` workflow.
- `uninstall`, `logs`, `restart`, `status`, `clean`: unchanged

## Cutover

Full replacement, not side-by-side. Once the Rust code and its tests are
in place and `cargo test`/`just check` are green:

- Delete `src/agentpad/` (Python), `tests/*.py`, `tests/conftest.py`,
  `pyproject.toml`, `uv.lock`; remove `.venv/` (gitignored, but stop
  referencing it)
- Update `.gitignore`: drop `__pycache__/`, `.venv/`, `.ruff_cache/`; add
  `target/`
- Update `README.md`: install instructions
  (`uv tool install` → the new `just install`), any other Python-specific
  mentions. The behavior/protocol descriptions (tables, SVG diagrams)
  don't change since behavior doesn't change.

## Resolved decisions (from brainstorming)

- `scripts/diagrams.py` is converted to Rust too, not left as Python —
  full parity, no Python left in the repo.
- Dependencies: minimal well-vetted crates (`serde`/`serde_json`, `nix`),
  not a hand-rolled zero-dependency implementation.
