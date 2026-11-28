#!/usr/bin/env bash
# Makes the Linux servers in <dist> independent of the build machine's
# libraries (ADR 0031): every shared library they need, directly or through
# another library, that is not part of the system base below is copied into
# <dist>/lib under its soname, and the executables look there first
# (RUNPATH $ORIGIN/lib; the copied libraries get $ORIGIN). SteamOS has no
# package installation: what is not in the base must travel with the server.
#
# System base (present on every glibc distribution the launcher supports,
# SteamOS included): glibc, libstdc++/libgcc_s (newer on the target than on
# the Ubuntu 22.04 build runner) and zlib. OpenSSL 3 is bundled: Arch moved
# to OpenSSL 4 (libssl.so.4; libssl.so.3 only in its openssl-3 compat
# package, 2026-11-28) and SteamOS follows Arch. Its legacy provider goes to
# lib/ossl-modules: the launcher passes that directory as OPENSSL_MODULES,
# since the bundled libcrypto looks in the build runner's module directory.
#
# Needs: ldd, patchelf. Usage: tools/ci/bundle-libs.sh <dist> <executable>...
set -euo pipefail

dist="$(cd "$1" && pwd)"
shift
mkdir -p "$dist/lib"
# Absolute paths: ldd reports $ORIGIN-resolved libraries from them.
set -- $(for exe in "$@"; do realpath "$exe"; done)

base='^(linux-vdso\.so\.1|ld-linux-x86-64\.so\.2|libc\.so\.6|libm\.so\.6|libpthread\.so\.0|libdl\.so\.2|librt\.so\.1|libresolv\.so\.2|libutil\.so\.1|libstdc\+\+\.so\.6|libgcc_s\.so\.1|libz\.so\.1)$'

# Queue of ELF files whose dependencies are resolved; libraries copied into
# lib/ are added to it, so dependencies of dependencies come along.
queue=("$@")
declare -A seen=()
while [ "${#queue[@]}" -gt 0 ]; do
  file="${queue[0]}"
  queue=("${queue[@]:1}")
  [ -n "${seen[$file]:-}" ] && continue
  seen[$file]=1
  while read -r name _ path _; do
    [ -z "$name" ] && continue
    if [[ "$name" =~ $base ]] || [[ "$name" == /* ]]; then
      continue
    fi
    if [ "$path" = "not" ]; then
      echo "::error::$(basename "$file") needs $name, which is not found on the build machine" >&2
      exit 1
    fi
    if [ ! -f "$dist/lib/$name" ]; then
      cp -L "$path" "$dist/lib/$name"
      chmod 0755 "$dist/lib/$name"
      echo "bundled $name (from $path, for $(basename "$file"))"
      queue+=("$dist/lib/$name")
    fi
  done < <(ldd "$file" | sed -E 's/^\s+//' | awk '{print $1, $2, $3, $4}')
done

# OpenSSL legacy provider (RC4 of the world session crypt) of the bundled
# libcrypto, from the module directory it was built with.
if [ ! -f "$dist/lib/libcrypto.so.3" ]; then
  echo "::error::libcrypto.so.3 was not bundled: do the servers still link OpenSSL 3?" >&2
  exit 1
fi
modules="$(openssl version -m | sed -E 's/^MODULESDIR: "(.*)"$/\1/')"
mkdir -p "$dist/lib/ossl-modules"
cp -L "$modules/legacy.so" "$dist/lib/ossl-modules/legacy.so"
chmod 0755 "$dist/lib/ossl-modules/legacy.so"
echo "bundled ossl-modules/legacy.so (from $modules)"

for exe in "$@"; do
  patchelf --set-rpath '$ORIGIN/lib' "$exe"
done
for lib in "$dist"/lib/*.so*; do
  patchelf --set-rpath '$ORIGIN' "$lib"
done
patchelf --set-rpath '$ORIGIN/..' "$dist/lib/ossl-modules/legacy.so"

# What is left outside lib/ must be in the base: checked once more after the
# rpaths changed, so the artifact never depends on the runner. Without
# LD_LIBRARY_PATH (the caller may set it to find libmysqlclient), which would
# be searched before the bundle's RUNPATH.
unset LD_LIBRARY_PATH
status=0
for file in "$@" "$dist"/lib/*.so* "$dist/lib/ossl-modules/legacy.so"; do
  while read -r name _ path _; do
    [ -z "$name" ] && continue
    if [[ "$name" =~ $base ]] || [[ "$name" == /* ]]; then continue; fi
    case "$path" in
      "$dist/lib/"*) ;;
      *)
        echo "::error::$(basename "$file") resolves $name outside the bundle ($path)" >&2
        status=1
        ;;
    esac
  done < <(ldd "$file" | sed -E 's/^\s+//' | awk '{print $1, $2, $3, $4}')
done
exit "$status"
