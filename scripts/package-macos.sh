#!/usr/bin/env bash
# Cria target/EcoReset.app universal (Apple Silicon e Intel), com o libusb embutido.
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
for t in aarch64-apple-darwin x86_64-apple-darwin; do
  cargo build --release --target "$t" -p ecoreset -p ecoreset-cli
done
APP=target/EcoReset.app
rm -rf "$APP"; mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
for bin in ecoreset ecoreset-cli; do
  lipo -create -output "$APP/Contents/MacOS/$bin" \
    "target/aarch64-apple-darwin/release/$bin" "target/x86_64-apple-darwin/release/$bin"
done
cat > "$APP/Contents/Info.plist" <<PL
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>EcoReset</string>
<key>CFBundleIdentifier</key><string>com.ilmar.ecoreset</string>
<key>CFBundleExecutable</key><string>ecoreset</string>
<key>CFBundleShortVersionString</key><string>$VERSION</string>
<key>CFBundleVersion</key><string>$VERSION</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PL
codesign --force --deep --sign - "$APP" 2>/dev/null || true
echo "Pronto: $APP"
