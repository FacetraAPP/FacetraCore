#!/bin/bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUTPUT=${FACETRA_APPLE_OUTPUT:-"$ROOT/build/apple"}
CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-"$ROOT/target"}
RUST_TOOLCHAIN=1.90.0
JOBS=${FACETRA_BUILD_JOBS:-4}
BUILD_ONLY=${FACETRA_APPLE_BUILD_ONLY:-0}
SLICES_DIR=${FACETRA_APPLE_SLICES_DIR:-}
DEFAULT_TARGETS=(
    aarch64-apple-darwin
    aarch64-apple-ios
    aarch64-apple-ios-sim
)
export CARGO_TARGET_DIR

if [ "$(uname -s)" != Darwin ]; then
    echo "Apple artifacts must be built on macOS." >&2
    exit 2
fi
if [ "$BUILD_ONLY" != 0 ] && [ "$BUILD_ONLY" != 1 ]; then
    echo "FACETRA_APPLE_BUILD_ONLY must be 0 or 1." >&2
    exit 2
fi
if [ "$BUILD_ONLY" = 1 ] && [ -n "$SLICES_DIR" ]; then
    echo "FACETRA_APPLE_BUILD_ONLY and FACETRA_APPLE_SLICES_DIR are mutually exclusive." >&2
    exit 2
fi

TARGETS=("${DEFAULT_TARGETS[@]}")
if [ -n "${FACETRA_APPLE_TARGETS:-}" ]; then
    read -r -a TARGETS <<< "$FACETRA_APPLE_TARGETS"
fi
if [ "${#TARGETS[@]}" -eq 0 ]; then
    echo "At least one Apple target is required." >&2
    exit 2
fi
for target in "${TARGETS[@]}"; do
    case "$target" in
        aarch64-apple-darwin | aarch64-apple-ios | aarch64-apple-ios-sim) ;;
        *)
            echo "Unsupported Apple target: $target" >&2
            exit 2
            ;;
    esac
done

COMMANDS=(xcrun)
if [ "$BUILD_ONLY" = 0 ]; then
    COMMANDS+=(rsync zip shasum python3)
fi
if [ -z "$SLICES_DIR" ]; then
    COMMANDS+=(cargo rustup)
fi
for command in "${COMMANDS[@]}"; do
    if ! command -v "$command" >/dev/null 2>&1; then
        echo "Missing required command: $command" >&2
        exit 2
    fi
done
if ! xcrun --find xcodebuild >/dev/null 2>&1 || ! xcrun --find lipo >/dev/null 2>&1; then
    echo "A full Xcode installation is required for xcodebuild and lipo." >&2
    exit 2
fi

mkdir -p "$OUTPUT"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/facetra-apple-artifacts.XXXXXX")
trap 'rm -rf "$WORK"' EXIT

platform_for_target() {
    case "$1" in
        aarch64-apple-darwin) echo macosx ;;
        aarch64-apple-ios) echo iphoneos ;;
        aarch64-apple-ios-sim) echo iphonesimulator ;;
        *)
            echo "Unsupported Apple target: $1" >&2
            exit 2
            ;;
    esac
}

build_slice() {
    local target=$1
    local platform=$2
    local sdkroot
    case "$platform" in
        macosx)
            env -u SDKROOT \
                MACOSX_DEPLOYMENT_TARGET=14.0 \
                cargo +"$RUST_TOOLCHAIN" build --release --locked \
                --target "$target" -p facetra-pipeline -j "$JOBS"
            ;;
        iphoneos)
            sdkroot=$(xcrun --sdk macosx --show-sdk-path)
            SDKROOT="$sdkroot" \
                IPHONEOS_DEPLOYMENT_TARGET=17.0 \
                cargo +"$RUST_TOOLCHAIN" build --release --locked \
                --target "$target" -p facetra-pipeline -j "$JOBS"
            ;;
        iphonesimulator)
            sdkroot=$(xcrun --sdk macosx --show-sdk-path)
            SDKROOT="$sdkroot" \
                IPHONEOS_DEPLOYMENT_TARGET=17.0 \
                cargo +"$RUST_TOOLCHAIN" build --release --locked \
                --target "$target" -p facetra-pipeline -j "$JOBS"
            ;;
        *)
            echo "Unsupported platform: $platform" >&2
            exit 2
            ;;
    esac
}

require_arm64() {
    local library=$1
    local architectures
    architectures=$(xcrun lipo -archs "$library")
    if [ "$architectures" != arm64 ]; then
        echo "$library must contain only arm64 (found: $architectures)." >&2
        exit 1
    fi
}

if [ -z "$SLICES_DIR" ]; then
    rustup target add --toolchain "$RUST_TOOLCHAIN" "${TARGETS[@]}"
    for target in "${TARGETS[@]}"; do
        platform=$(platform_for_target "$target")
        build_slice "$target" "$platform"
        slice="$CARGO_TARGET_DIR/$target/release/libfacetra_pipeline.a"
        if [ ! -f "$slice" ]; then
            echo "Rust did not produce $slice" >&2
            exit 1
        fi
        require_arm64 "$slice"
        if [ "$BUILD_ONLY" = 1 ]; then
            output_slice="$OUTPUT/libfacetra_pipeline-$target.a"
            cp "$slice" "$output_slice"
            echo "Apple slice written to $output_slice"
        fi
    done
fi

if [ "$BUILD_ONLY" = 1 ]; then
    exit 0
fi

if [ -z "$SLICES_DIR" ]; then
    SLICES_DIR="$WORK/slices"
    mkdir -p "$SLICES_DIR"
    for target in "${TARGETS[@]}"; do
        cp "$CARGO_TARGET_DIR/$target/release/libfacetra_pipeline.a" \
            "$SLICES_DIR/libfacetra_pipeline-$target.a"
    done
fi

MACOS_LIBRARY="$SLICES_DIR/libfacetra_pipeline-aarch64-apple-darwin.a"
IOS_LIBRARY="$SLICES_DIR/libfacetra_pipeline-aarch64-apple-ios.a"
IOS_SIM_LIBRARY="$SLICES_DIR/libfacetra_pipeline-aarch64-apple-ios-sim.a"
for library in "$MACOS_LIBRARY" "$IOS_LIBRARY" "$IOS_SIM_LIBRARY"; do
    if [ ! -f "$library" ]; then
        echo "Missing Apple slice: $library" >&2
        exit 1
    fi
    require_arm64 "$library"
done

XCFRAMEWORK="$OUTPUT/FacetraPipeline.xcframework"
if [ -e "$XCFRAMEWORK" ]; then
    rm -rf "$XCFRAMEWORK"
fi
xcrun xcodebuild -create-xcframework \
    -library "$MACOS_LIBRARY" -headers "$ROOT/core/include" \
    -library "$IOS_LIBRARY" -headers "$ROOT/core/include" \
    -library "$IOS_SIM_LIBRARY" -headers "$ROOT/core/include" \
    -output "$XCFRAMEWORK"

SLICE_COUNT=$(find "$XCFRAMEWORK" -type f -name 'libfacetra_pipeline.a' | wc -l | tr -d ' ')
if [ "$SLICE_COUNT" != 3 ]; then
    echo "Expected three XCFramework slices, found $SLICE_COUNT." >&2
    exit 1
fi

SUPPORT="$WORK/FacetraCoreSupport"
mkdir -p "$SUPPORT/renderers" "$SUPPORT/Contracts" "$SUPPORT/Docs/examples" "$SUPPORT/include"
rsync -a --exclude node_modules --exclude .DS_Store "$ROOT/renderers/" "$SUPPORT/renderers/"
rsync -a --exclude .DS_Store "$ROOT/Contracts/" "$SUPPORT/Contracts/"
rsync -a --exclude .DS_Store "$ROOT/Docs/examples/" "$SUPPORT/Docs/examples/"
rsync -a --exclude .DS_Store "$ROOT/core/include/" "$SUPPORT/include/"
cp "$ROOT/THIRD_PARTY_NOTICES.md" "$ROOT/LICENSE-MIT" "$ROOT/LICENSE-APACHE" "$SUPPORT/"

XCFRAMEWORK_ZIP="$OUTPUT/FacetraPipeline.xcframework.zip"
SUPPORT_ZIP="$OUTPUT/FacetraCoreSupport.zip"
MANIFEST="$OUTPUT/FacetraCore-main.json"
rm -f "$XCFRAMEWORK_ZIP" "$SUPPORT_ZIP" "$MANIFEST" "$OUTPUT/SHA256SUMS"
(
    cd "$OUTPUT"
    zip -q -r -X "$XCFRAMEWORK_ZIP" FacetraPipeline.xcframework
)
(
    cd "$WORK"
    zip -q -r -X "$SUPPORT_ZIP" FacetraCoreSupport
)

XCFRAMEWORK_SHA=$(shasum -a 256 "$XCFRAMEWORK_ZIP" | awk '{print $1}')
SUPPORT_SHA=$(shasum -a 256 "$SUPPORT_ZIP" | awk '{print $1}')
COMMIT_SHA=${COMMIT_SHA:-${GITHUB_SHA:-$(git -C "$ROOT" rev-parse HEAD)}}
REF=${GITHUB_REF:-$(git -C "$ROOT" symbolic-ref -q HEAD || git -C "$ROOT" rev-parse HEAD)}
RUN_ID=${GITHUB_RUN_ID:-local}
REPOSITORY=${GITHUB_REPOSITORY:-FacetraAPP/FacetraCore}
if [ -n "${GITHUB_SERVER_URL:-}" ] && [ -n "${GITHUB_REPOSITORY:-}" ] && [ -n "${GITHUB_RUN_ID:-}" ]; then
    RUN_URL="$GITHUB_SERVER_URL/$GITHUB_REPOSITORY/actions/runs/$GITHUB_RUN_ID"
else
    RUN_URL=""
fi
BUILT_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)

python3 - "$MANIFEST" "$REPOSITORY" "$REF" "$COMMIT_SHA" "$RUN_ID" "$RUN_URL" \
    "$BUILT_AT" "$XCFRAMEWORK_SHA" "$SUPPORT_SHA" <<'PY'
import json
import sys

path, repository, ref, commit, run_id, run_url, built_at, xcframework_sha, support_sha = sys.argv[1:]
manifest = {
    "schemaVersion": 1,
    "repository": repository,
    "ref": ref,
    "commit": commit,
    "runId": run_id,
    "runUrl": run_url,
    "builtAt": built_at,
    "rustToolchain": "1.90.0",
    "deploymentTargets": {"macOS": "14.0", "iOS": "17.0"},
    "slices": ["macOS arm64", "iOS arm64", "iOS simulator arm64"],
    "artifacts": {
        "xcframework": {
            "file": "FacetraPipeline.xcframework.zip",
            "sha256": xcframework_sha,
            "swiftPackageChecksum": xcframework_sha,
        },
        "support": {
            "file": "FacetraCoreSupport.zip",
            "sha256": support_sha,
        },
    },
}
with open(path, "w", encoding="utf-8") as stream:
    json.dump(manifest, stream, indent=2, sort_keys=True)
    stream.write("\n")
PY

(
    cd "$OUTPUT"
    shasum -a 256 \
        FacetraPipeline.xcframework.zip \
        FacetraCoreSupport.zip \
        FacetraCore-main.json > SHA256SUMS
)

echo "Apple artifacts written to $OUTPUT"
cat "$OUTPUT/SHA256SUMS"
