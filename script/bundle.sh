#!/bin/sh
# Builds Editor.app into ~/Applications and registers its file types with macOS.
set -eu
cd "$(dirname "$0")/.."

cargo build --release
app="$HOME/Applications/Editor.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/editor "$app/Contents/MacOS/editor"
cp assets/Info.plist "$app/Contents/Info.plist"
# Assets.car carries the Liquid Glass icon for macOS 26+; AppIcon.icns covers older systems.
xcrun actool assets/AppIcon.icon --compile "$app/Contents/Resources" --platform macosx \
    --minimum-deployment-target 13.0 --app-icon AppIcon \
    --output-partial-info-plist /dev/null >/dev/null
codesign --force --sign - "$app"
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$app"
echo "$app"
