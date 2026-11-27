#!/usr/bin/env bash
# Linux counterpart of fetch-mysql.ps1 (ADR 0031): downloads, verifies (pinned
# size + sha256 from the catalog) and unpacks the official MySQL tarball to
# <destination>, then copies in the files of its `extras` (official Debian
# packages of the libraries some systems lack, e.g. libncurses.so.6 on
# SteamOS), exactly as the launcher does.
#
# Superseded releases move from cdn.mysql.com/Downloads/ to /archives/: tried
# as a fallback (same bytes, same sha256).
#
# Needs: bash, curl, sha256sum, tar with xz, ar (binutils), node.
# Usage: tools/ci/fetch-mysql.sh <component.json> <destination> [platform]
set -euo pipefail

manifest="$1"
dest="$2"
platform="${3:-linux-x86_64}"

if [ -x "$dest/bin/mysqld" ]; then
  echo "MySQL already present at $dest"
  exit 0
fi

field() { node -e 'const c=require(require("path").resolve(process.argv[1])); const b=c.platforms[process.argv[2]]; if (!b) { console.error(`no ${process.argv[2]} build`); process.exit(1) } console.log(eval(process.argv[3]))' "$manifest" "$platform" "$1"; }
version="$(node -p 'require(require("path").resolve(process.argv[1])).version' "$manifest")"
series="${version%.*}"
upstream="$(field b.upstream)"
size="$(field b.size)"
sha256="$(field b.sha256)"
name="$(basename "$upstream")"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# <out> <size> <sha256> <url...>: first URL that gives the pinned bytes.
download() {
  local out="$1" want_size="$2" want_sha="$3"
  shift 3
  for url in "$@"; do
    for attempt in 1 2 3; do
      if curl -fsSL --retry 2 --connect-timeout 60 -A rk-ci -o "$out" "$url"; then
        local got_size got_sha
        got_size="$(stat -c %s "$out")"
        got_sha="$(sha256sum "$out" | cut -d' ' -f1)"
        if [ "$got_size" = "$want_size" ] && [ "$got_sha" = "$want_sha" ]; then
          echo "downloaded $url (sha256 verified)"
          return 0
        fi
        echo "attempt $attempt: $url gave $got_size bytes, sha256 $got_sha"
      else
        echo "attempt $attempt failed for $url"
      fi
      sleep 10
    done
  done
  echo "::error::could not download a verified $(basename "$out")" >&2
  return 1
}

download "$work/$name" "$size" "$sha256" "$upstream" "https://cdn.mysql.com/archives/mysql-$series/$name"
mkdir -p "$work/unpacked"
tar -xJf "$work/$name" -C "$work/unpacked"
root="$(find "$work/unpacked" -mindepth 1 -maxdepth 1 -type d)"
if [ "$(echo "$root" | wc -l)" != 1 ]; then
  echo "::error::$name does not hold a single root directory" >&2
  exit 1
fi

# Extras: Debian packages (ar archive, data.tar.*), regular files copied.
count="$(field '(b.extras ?? []).length')"
for ((i = 0; i < count; i++)); do
  url="$(field "b.extras[$i].upstream")"
  pkg="$work/extra-$i.deb"
  download "$pkg" "$(field "b.extras[$i].size")" "$(field "b.extras[$i].sha256")" "$url"
  mkdir -p "$work/extra-$i"
  (cd "$work/extra-$i" && ar x "$pkg")
  data="$(find "$work/extra-$i" -maxdepth 1 -name 'data.tar.*' | head -1)"
  mkdir -p "$work/extra-$i/data"
  tar -xf "$data" -C "$work/extra-$i/data"
  while IFS=$'\t' read -r from to; do
    src="$work/extra-$i/data/$from"
    if [ ! -f "$src" ] || [ -L "$src" ]; then
      echo "::error::$from is not a regular file in $url" >&2
      exit 1
    fi
    mkdir -p "$(dirname "$root/$to")"
    cp "$src" "$root/$to"
    echo "extra: $from -> $to"
  done < <(field "Object.entries(b.extras[$i].files).map(([f, t]) => f + '\t' + t).join('\n')")
done

mkdir -p "$(dirname "$dest")"
rm -rf "$dest"
mv "$root" "$dest"
for exe in $(field 'Object.values(b.executables).join(" ")'); do
  if [ ! -x "$dest/$exe" ]; then
    echo "::error::$exe missing from $name" >&2
    exit 1
  fi
done
echo "MySQL $version ready at $dest"
