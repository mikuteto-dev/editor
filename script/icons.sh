#!/bin/sh
# Fetches Material Icon Theme into assets/icons; kept out of git because it is a thousand files.
set -eu
cd "$(dirname "$0")/.."

version=5.40.0
sha256=6ea13a2dece7ca953c0315b1e93348660c02bb235706275d215be69f09d98efb
[ -f assets/icons/material-icons.json ] && exit 0

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -sfL "https://registry.npmjs.org/material-icon-theme/-/material-icon-theme-$version.tgz" -o "$tmp/p.tgz"
echo "$sha256  $tmp/p.tgz" | shasum -a 256 -c - >/dev/null
tar xzf "$tmp/p.tgz" -C "$tmp"
mkdir -p assets/icons
cp "$tmp"/package/icons/*.svg "$tmp/package/dist/material-icons.json" "$tmp/package/LICENSE" assets/icons/
