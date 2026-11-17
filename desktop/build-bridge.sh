#!/bin/sh
# Builds the bridge that goes into the app bundle, for the target Tauri is building (its beforeBuildCommand): both
# architectures for a universal app. The bundle takes it from target/release/github-magic-bridge.
set -eu
cd "$(dirname "$0")"
if [ "${TAURI_ENV_TARGET_TRIPLE:-}" = universal-apple-darwin ]; then
  for target in aarch64-apple-darwin x86_64-apple-darwin; do
    cargo build --release -p github-magic-bridge --target "$target"
  done
  mkdir -p target/release
  lipo -create -output target/release/github-magic-bridge \
    target/aarch64-apple-darwin/release/github-magic-bridge target/x86_64-apple-darwin/release/github-magic-bridge
else
  cargo build --release -p github-magic-bridge
fi
