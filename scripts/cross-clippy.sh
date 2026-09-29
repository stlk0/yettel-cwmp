#!/usr/bin/env bash
# Runs Clippy for the Windows, macOS and musl code paths from a Linux host, so a
# change that breaks another platform fails here in seconds instead of on a
# paid native runner. It only type-checks: nothing is linked or run, so native
# tests (the Native workflow) are still needed for runtime behavior.
#
# Needs rustup, clang, the MinGW-w64 C compiler and musl-gcc. On Debian/Ubuntu:
#   sudo apt-get install clang gcc-mingw-w64-x86-64 musl-tools
set -euo pipefail
cd "$(dirname "$0")/.."

# One target per operating system family is enough: the code has no
# CPU-specific cfgs, and windows-gnu compiles the same cfg(windows) code as
# windows-msvc.
targets=(x86_64-pc-windows-gnu aarch64-apple-darwin x86_64-unknown-linux-musl)

missing=()
for tool in clang x86_64-w64-mingw32-gcc musl-gcc; do
  command -v "$tool" >/dev/null || missing+=("$tool")
done
if ((${#missing[@]})); then
  echo "cross-clippy: missing ${missing[*]}" >&2
  echo "cross-clippy: on Debian/Ubuntu run: sudo apt-get install clang gcc-mingw-w64-x86-64 musl-tools" >&2
  exit 1
fi

rustup target add "${targets[@]}"

# ring compiles C for the target and there is no macOS SDK here. Build it
# against clang's own headers (ring supports this for sysroot-less
# cross-compiling) plus a stub for the one Apple header it reads. The objects
# are never linked; Clippy only needs the build scripts to finish. The stub
# path is stable so repeated runs reuse the built ring.
mkdir -p "${CARGO_TARGET_DIR:-target}/cross-clippy"
stub="$(cd "${CARGO_TARGET_DIR:-target}/cross-clippy" && pwd)"
printf '#define TARGET_OS_MAC 1\n#define TARGET_OS_OSX 1\n#define TARGET_OS_IPHONE 0\n' \
  > "$stub/TargetConditionals.h"
export CC_aarch64_apple_darwin=clang
export CFLAGS_aarch64_apple_darwin="-nostdlibinc -DRING_CORE_NOSTDLIBINC=1 -I$stub"

for target in "${targets[@]}"; do
  echo "cross-clippy: $target"
  cargo clippy --all-targets --locked --target "$target" -- -D warnings
  cargo clippy --all-targets --all-features --locked --target "$target" -- -D warnings
done
