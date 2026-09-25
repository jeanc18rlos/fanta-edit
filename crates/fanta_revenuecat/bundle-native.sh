#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "Usage: $0 /absolute/path/to/Fanta.app" >&2
    exit 2
fi

app_path="$1"
script_dir="$(cd "$(dirname "$0")" && pwd)"
repository_root="$(cd "$script_dir/../.." && pwd)"
native_package="$script_dir/native"
scratch_path="$repository_root/target/revenuecat-swift"

if [[ ! -d "$app_path/Contents/MacOS" ]]; then
    echo "App bundle is missing Contents/MacOS: $app_path" >&2
    exit 2
fi

swift build \
    --disable-sandbox \
    --package-path "$native_package" \
    --scratch-path "$scratch_path" \
    --configuration release \
    --product FantaRevenueCatBridge

bin_path="$(swift build \
    --disable-sandbox \
    --package-path "$native_package" \
    --scratch-path "$scratch_path" \
    --configuration release \
    --show-bin-path)"

dylib="$bin_path/libFantaRevenueCatBridge.dylib"
if [[ ! -f "$dylib" ]]; then
    echo "RevenueCat bridge was not produced: $dylib" >&2
    exit 1
fi
privacy_bundle="$bin_path/RevenueCat_RevenueCat.bundle"
if [[ ! -f "$privacy_bundle/Contents/Resources/PrivacyInfo.xcprivacy" ]]; then
    echo "RevenueCat privacy bundle was not produced: $privacy_bundle" >&2
    exit 1
fi

mkdir -p "$app_path/Contents/Frameworks" "$app_path/Contents/Resources"
ditto "$dylib" "$app_path/Contents/Frameworks/libFantaRevenueCatBridge.dylib"
ditto "$privacy_bundle" "$app_path/Contents/Resources/RevenueCat_RevenueCat.bundle"
