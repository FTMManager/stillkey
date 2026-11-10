#!/usr/bin/env bash
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/stillkey/target}"
exec cargo xtask "$@"
