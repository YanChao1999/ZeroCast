#!/usr/bin/env bash
# Cross-build zerocast_desktop for Linux aarch64 (Pi 3/4/5 class recv).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

LINKER="${CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER:-aarch64-linux-gnu-gcc}"

if ! command -v "$LINKER" >/dev/null 2>&1; then
  echo "error: cross linker '$LINKER' not found" >&2
  echo "  install: sudo apt-get install -y gcc-aarch64-linux-gnu" >&2
  exit 1
fi

if [[ -f "$HOME/.cargo/env" ]]; then
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo not found" >&2
  cat >&2 <<'EOF'
  Install Rust (pick one):

  A) rustup (recommended):
     curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
     source "$HOME/.cargo/env"
     rustup default stable

     If downloads time out (common in CN), use the TUNA mirror:
     export RUSTUP_DIST_SERVER=https://mirrors.tuna.tsinghua.edu.cn/rustup
     export RUSTUP_UPDATE_ROOT=https://mirrors.tuna.tsinghua.edu.cn/rustup/rustup
     rustup toolchain install stable

  B) Ubuntu packages (older, usually enough to build):
     sudo apt-get install -y rustc cargo

  Then re-run this script.
EOF
  exit 1
fi

export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER="$LINKER"
export PKG_CONFIG_ALLOW_CROSS=1
export PKG_CONFIG_PATH="${PKG_CONFIG_PATH:-/usr/lib/aarch64-linux-gnu/pkgconfig}"

# Headless cross-build skips minifb (no :arm64 libxcb needed). For `recv` window on ARM,
# build natively inside the VM: cargo build -p zerocast_desktop --release

echo "==> adding rust target aarch64-unknown-linux-gnu (if needed)"
if command -v rustup >/dev/null 2>&1; then
  rustup target add aarch64-unknown-linux-gnu
else
  echo "    (rustup not found — assuming target is already installed via apt/rustc)"
fi

echo "==> cross-building zerocast_desktop (release, headless — no minifb/xcb)"
cargo build -p zerocast_desktop --release --target aarch64-unknown-linux-gnu --no-default-features

OUT="$ROOT/target/aarch64-unknown-linux-gnu/release/zerocast_desktop"
echo ""
echo "OK: $OUT"
file "$OUT" 2>/dev/null || true
