#!/usr/bin/env bash
# Cross-build zerocast_desktop for Linux armv6 (Raspberry Pi Zero W).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

LINKER="${CARGO_TARGET_ARMV6_UNKNOWN_LINUX_GNUEABIHF_LINKER:-arm-linux-gnueabihf-gcc}"

if ! command -v "$LINKER" >/dev/null 2>&1; then
  echo "error: cross linker '$LINKER' not found" >&2
  echo "  install: sudo apt-get install -y gcc-arm-linux-gnueabihf" >&2
  exit 1
fi

if [[ -f "$HOME/.cargo/env" ]]; then
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
fi

export CARGO_TARGET_ARMV6_UNKNOWN_LINUX_GNUEABIHF_LINKER="$LINKER"
export PKG_CONFIG_ALLOW_CROSS=1

echo "==> adding rust target armv6-unknown-linux-gnueabihf (if needed)"
if command -v rustup >/dev/null 2>&1; then
  rustup target add armv6-unknown-linux-gnueabihf
fi

echo "==> cross-building zerocast_desktop (release, headless)"
cargo build -p zerocast_desktop --release --target armv6-unknown-linux-gnueabihf --no-default-features

OUT="$ROOT/target/armv6-unknown-linux-gnueabihf/release/zerocast_desktop"
echo ""
echo "OK: $OUT"
file "$OUT" 2>/dev/null || true
