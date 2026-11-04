#!/usr/bin/env bash
# Builds a universal (Apple Silicon + Intel) "Cat Tetris.app" and a zip of it.
# Run on a Mac with Rust installed (https://rustup.rs) and Xcode command line
# tools (xcode-select --install).
set -euo pipefail
cd "$(dirname "$0")"

APP="Cat Tetris"
OUT="dist/$APP.app"

rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin

rm -rf "$OUT"
mkdir -p "$OUT/Contents/MacOS" "$OUT/Contents/Resources"
lipo -create -output "$OUT/Contents/MacOS/tetris" \
    target/aarch64-apple-darwin/release/tetris \
    target/x86_64-apple-darwin/release/tetris
cp macos/Info.plist "$OUT/Contents/Info.plist"

# ad-hoc signature (lets it run locally; not a notarized release)
codesign --force --deep --sign - "$OUT"

ditto -c -k --keepParent "$OUT" "dist/CatTetris-macos.zip"
echo "Done: $OUT  and  dist/CatTetris-macos.zip"
