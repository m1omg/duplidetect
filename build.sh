#!/bin/bash
# Builds DupliDetect.app. Default is a universal (arm64 + x86_64) binary.
#   ./build.sh              universal release build
#   ./build.sh --fast       host-arch only, for quick iteration
set -euo pipefail

cd "$(dirname "$0")"

APP_NAME="DupliDetect"
BUNDLE_ID="com.duplidetect.app"
VERSION="1.0"
MIN_MACOS="12.0"
SDK="$(xcrun --show-sdk-path)"

BUILD_DIR="build"
APP_DIR="$BUILD_DIR/$APP_NAME.app"

ARCHS=(x86_64 arm64)
OPT=(-O)
if [[ "${1:-}" == "--fast" ]]; then
  ARCHS=("$(uname -m)")
  OPT=(-Onone)
  echo "==> fast mode: $(uname -m) only"
fi

rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR/obj" "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"

SWIFT_SOURCES=(Sources/DupliDetect/*.swift)
C_SOURCES=(Sources/CVorbis/*.c)

SLICES=()
for ARCH in "${ARCHS[@]}"; do
  echo "==> building $ARCH"
  TARGET="$ARCH-apple-macos$MIN_MACOS"
  OBJ_DIR="$BUILD_DIR/obj/$ARCH"
  mkdir -p "$OBJ_DIR"

  OBJECTS=()
  for SRC in "${C_SOURCES[@]}"; do
    OBJ="$OBJ_DIR/$(basename "${SRC%.c}").o"
    clang -c "$SRC" -o "$OBJ" \
      -target "$TARGET" -isysroot "$SDK" \
      -I Sources/CVorbis/include \
      -O2 -fno-strict-aliasing -w \
      -DSTB_VORBIS_NO_PUSHDATA_API
    OBJECTS+=("$OBJ")
  done

  BIN="$BUILD_DIR/obj/$APP_NAME-$ARCH"
  swiftc "${SWIFT_SOURCES[@]}" \
    -o "$BIN" \
    -target "$TARGET" -sdk "$SDK" \
    "${OPT[@]}" \
    -import-objc-header Sources/DupliDetect/Bridging.h \
    -I Sources/CVorbis/include \
    -framework AppKit -framework AVFoundation -framework Accelerate \
    -framework SwiftUI -framework UniformTypeIdentifiers \
    "${OBJECTS[@]}"
  SLICES+=("$BIN")
done

if [[ ${#SLICES[@]} -gt 1 ]]; then
  echo "==> merging universal binary"
  lipo -create "${SLICES[@]}" -output "$APP_DIR/Contents/MacOS/$APP_NAME"
else
  cp "${SLICES[0]}" "$APP_DIR/Contents/MacOS/$APP_NAME"
fi

cat > "$APP_DIR/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>$APP_NAME</string>
    <key>CFBundleDisplayName</key><string>DupliDetect</string>
    <key>CFBundleExecutable</key><string>$APP_NAME</string>
    <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>LSMinimumSystemVersion</key><string>$MIN_MACOS</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSPrincipalClass</key><string>NSApplication</string>
    <key>NSSupportsAutomaticTermination</key><true/>
    <key>NSHumanReadableCopyright</key><string>Public domain</string>
    <key>CFBundleDocumentTypes</key>
    <array>
        <dict>
            <key>CFBundleTypeName</key><string>Folder</string>
            <key>CFBundleTypeRole</key><string>Viewer</string>
            <key>LSHandlerRank</key><string>Alternate</string>
            <key>LSItemContentTypes</key>
            <array><string>public.folder</string></array>
        </dict>
        <dict>
            <key>CFBundleTypeName</key><string>Audio File</string>
            <key>CFBundleTypeRole</key><string>Viewer</string>
            <key>LSHandlerRank</key><string>Alternate</string>
            <key>LSItemContentTypes</key>
            <array><string>public.audio</string></array>
        </dict>
    </array>
</dict>
</plist>
PLIST

echo 'APPL????' > "$APP_DIR/Contents/PkgInfo"

if [[ -f Resources/AppIcon.icns ]]; then
  cp Resources/AppIcon.icns "$APP_DIR/Contents/Resources/AppIcon.icns"
fi

# Ad-hoc signature so macOS will launch it without a developer certificate.
codesign --force --deep --sign - "$APP_DIR" 2>/dev/null || echo "(codesign skipped)"

echo
echo "==> built $APP_DIR"
lipo -info "$APP_DIR/Contents/MacOS/$APP_NAME"
