#!/usr/bin/env bash
# One-time download of the console font, Spleen 8x16 2.2.0, and its licence
# into assets/ (docs/10 section 5.3). Needs the network. Each file must have
# the SHA-256 written below; a file that does not is thrown away.
#
# The files are data. Nothing in them is run: `cargo xtask font` reads the
# glyph rows of the .bdf file and writes lib/sk-kcore/src/font.rs.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
base="https://raw.githubusercontent.com/fcambus/spleen/2.2.0"

# fetch <name at the source> <name in assets/> <sha256>
fetch() {
    local tmp got
    tmp="$(mktemp)"
    curl -fsSL --max-time 120 -o "$tmp" "$base/$1"
    got="$(sha256sum "$tmp" | cut -d' ' -f1)"
    if [ "$got" != "$3" ]; then
        rm -f "$tmp"
        echo "fetch-font: $1 has SHA-256 $got, expected $3" >&2
        exit 1
    fi
    mkdir -p "$root/assets"
    mv "$tmp" "$root/assets/$2"
    chmod 644 "$root/assets/$2"
    echo "assets/$2 sha256=$got"
}

fetch spleen-8x16.bdf spleen-8x16.bdf 4a3d97ee61a8c86a7525d8c723cb8a14081f395cd2feb4227ba5e3baf0629bae
fetch LICENSE spleen-LICENSE f33fe8679d5b2abecc4f1313ce6c6bfa58262964de5f7bca146596a7318047af
