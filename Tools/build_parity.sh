#!/bin/bash
# Builds the parity-dump harness.
#
# The shipping sources in Sources/ are NEVER modified. They are copied to a
# temporary directory where a few `private` members are widened to internal so
# the harness can read the window coefficients, band edges and per-band
# energies. The dumped numbers therefore come from the real algorithm code,
# not from a re-implementation of it.
set -euo pipefail
cd "$(dirname "$0")/.."

SDK="$(xcrun --show-sdk-path)"
WORK="build/parity-src"
OUT="build/paritydump"

rm -rf "$WORK"; mkdir -p "$WORK" build
cp Sources/DupliDetect/*.swift "$WORK"/
# Drop the GUI: it drags in SwiftUI and is irrelevant to parity.
# AppModel.swift is kept: KeepRule lives there and the scan report must use the
# real ranking code, not a copy that could drift from it.
rm -f "$WORK"/ContentView.swift "$WORK"/ResultsView.swift "$WORK"/DupliDetectApp.swift \
      "$WORK"/PreviewPlayer.swift

# Widen exactly the members the harness needs to observe.
sed -i '' \
  -e 's/^    private let window: \[Float\]/    let window: [Float]/' \
  -e 's/^    private let bandEdges: \[Int\]/    let bandEdges: [Int]/' \
  -e 's/^    private static func makeBandEdges/    static func makeBandEdges/' \
  -e 's/^    private func spectrum/    func spectrum/' \
  "$WORK"/Fingerprint.swift

# AppModel uses remove(atOffsets:), a SwiftUI extension that only resolves in the
# app because a sibling file imports SwiftUI. The harness compiles it alone.
sed -i '' '1i\
import SwiftUI
' "$WORK"/AppModel.swift

cp Tools/ParityDump/*.swift "$WORK"/

for SRC in Sources/CVorbis/*.c; do
  clang -c "$SRC" -o "build/$(basename "${SRC%.c}").parity.o" \
    -target x86_64-apple-macos12.0 -isysroot "$SDK" \
    -I Sources/CVorbis/include -O2 -w -DSTB_VORBIS_NO_PUSHDATA_API
done

swiftc "$WORK"/*.swift -o "$OUT" \
  -target x86_64-apple-macos12.0 -sdk "$SDK" -O \
  -import-objc-header Sources/DupliDetect/Bridging.h \
  -I Sources/CVorbis/include \
  -framework AVFoundation -framework Accelerate -framework AppKit -framework SwiftUI \
  build/*.parity.o

echo "built $OUT"
