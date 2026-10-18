#!/usr/bin/env bash
# macOS 本地打包：构建 .app → ad-hoc 签名 → 自验 → 造 dmg
# （CI 自 v14.4.5 起只产 Windows；macOS 包由维护者在本机执行本脚本产出）
set -euo pipefail
cd "$(dirname "$0")/.."
export PDFIUM_STATIC_LIB_PATH="$PWD/src-tauri/bin"

node scripts/tauri-build.mjs build --bundles app

APP="src-tauri/target/release/bundle/macos/Arkmind.app"
codesign --force --deep --sign - "$APP"
codesign --verify --deep --strict --verbose=2 "$APP"

VERSION=$(grep -m1 '^version' src-tauri/Cargo.toml | cut -d'"' -f2)
DMG="Arkmind_${VERSION}_aarch64.dmg"
STAGE="$(mktemp -d)"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
cp scripts/mac-install-readme.txt "$STAGE/安装说明.txt"
hdiutil create -volname Arkmind -srcfolder "$STAGE" -ov -format UDZO "$DMG"
rm -rf "$STAGE"

echo "✓ 产物: $DMG（上传至 release draft / 网盘 Arkmind/ 目录）"
