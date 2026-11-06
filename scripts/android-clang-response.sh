#!/usr/bin/env bash
# Avoid Windows' command-line limit when linking OpenSSL's large x86_64 object list.
set -euo pipefail
target=$1
shift
response=$(mktemp)
trap 'rm -f "$response"' EXIT
for argument in "$@"; do
    argument=${argument//\\/\\\\}
    argument=${argument//\"/\\\"}
    printf '"%s"\n' "$argument" >> "$response"
done
"$ANDROID_NDK_ROOT/toolchains/llvm/prebuilt/windows-x86_64/bin/clang.exe" \
    --target="$target" @"$response"
