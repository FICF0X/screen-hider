#!/usr/bin/env bash
# Build Screen Hider: both payload DLLs (64 & 32 bit) + engine/injector/ui.
# Usage: ./build.sh [debug|release]   (default: release)
set -e
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")"

MODE="${1:-release}"
FLAG=""
[ "$MODE" = "release" ] && FLAG="--release"

echo ">> 64-bit payload"
cargo build $FLAG -p payload
echo ">> 32-bit payload"
cargo build $FLAG -p payload --target i686-pc-windows-msvc
echo ">> engine + injector + ui (64-bit host)"
cargo build $FLAG -p engine -p injector -p ui

OUT="target/$MODE"
cp -f "target/$MODE/payload.dll" "$OUT/payload64.dll"
cp -f "target/i686-pc-windows-msvc/$MODE/payload.dll" "$OUT/payload32.dll"

echo ">> done -> $OUT"
ls -la "$OUT/ui.exe" "$OUT/injector.exe" "$OUT/payload64.dll" "$OUT/payload32.dll"
