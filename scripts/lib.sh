# Shared by the scripts in this directory. Source it; don't run it.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOX=wisp

# TypeScript 7 ships as a static native binary, so the UI build needs no Node.
# Keep in step with the archive source in flatpak/io.github.peterwalker78.Wisp.yml.
TSC_VERSION=7.0.2
TSC_SHA256=7ecad6f67377e831856367ab062ef394f21506a611405bf8ac0ff039348637d3
TSC="$ROOT/.tools/typescript-$TSC_VERSION/lib/tsc"

inside_box() { [[ -e /run/.containerenv ]]; }

# On an immutable desktop the toolchain lives in a container. If a distrobox
# named `wisp` exists, re-run the calling script inside it; otherwise build
# right here.
require_box() {
  if ! inside_box && command -v distrobox >/dev/null \
    && distrobox list --no-color 2>/dev/null | grep -qw "$BOX"; then
    exec distrobox enter "$BOX" -- "$ROOT/scripts/$(basename "$0")" "$@"
  fi
}

require_host() {
  if inside_box; then
    echo "$(basename "$0") runs on the host, not in a container" >&2
    exit 1
  fi
}

# flatpak-builder's state in .flatpak-builder can't be shared by two builds at
# once: the second fails with "rofiles-... not initialized". Holds a lock on it
# until the calling script exits, waiting first for any build already running.
lock_builder() {
  mkdir -p "$ROOT/.flatpak-builder"
  exec 9>"$ROOT/.flatpak-builder/wisp-build.lock"
  if ! flock -n 9; then
    echo "Waiting for another Flatpak build of Wisp to finish..." >&2
    flock 9
  fi
}

# Turns Cargo.lock into the offline sources the Flatpak build needs.
CARGO_GENERATOR_COMMIT=de2225a6dee4818c1339b3cdbf29f90c471fcb7e
CARGO_GENERATOR_SHA256=b373c8ab1a05378ec5d8ed0645c7b127bcec7d2f7a1798694fbc627d570d856c
CARGO_GENERATOR="$ROOT/.tools/flatpak-cargo-generator-${CARGO_GENERATOR_COMMIT:0:12}.py"
