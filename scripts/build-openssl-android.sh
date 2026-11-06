#!/usr/bin/env bash
# Run under MSYS2 on Windows (or adapt NDK host directory on Linux).
# Arguments are POSIX paths without spaces, e.g. /z via a temporary subst drive.
set -euo pipefail
source_root=$1
build_root=$2
output_root=$3
ndk_root=$4
architecture=$5
script_root=$(cd "$(dirname "$0")" && pwd)
export ANDROID_NDK_ROOT="$ndk_root"
export PATH="$ndk_root/toolchains/llvm/prebuilt/windows-x86_64/bin:/usr/bin:$PATH"
mkdir -p "$build_root" "$output_root"
cd "$build_root"
perl "$source_root/Configure" "android-$architecture" shared no-tests no-apps \
    -D__ANDROID_API__=28 -Wl,-z,max-page-size=16384 --prefix="$output_root" --libdir=lib
case "$architecture" in
    arm64) compiler_target=aarch64-linux-android28 ;;
    x86_64) compiler_target=x86_64-linux-android28 ;;
    *) echo "Unsupported architecture: $architecture" >&2; exit 2 ;;
esac
make -j6 "CC=bash $script_root/android-clang-response.sh $compiler_target" build_libs
make install_dev
