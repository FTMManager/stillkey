#!/usr/bin/env bash
# The acceptance matrix of Milestone 2 (docs/10 section 10, D-W-5): the four
# commands, one after the other, each with its whole output in a log file.
# Prints the SUMMARY line of each and the lines of any failure. Every row
# runs even if an earlier one failed. Run inside WSL from the workspace root:
#   bash -l tools/acceptance.sh [repeat]
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/stillkey/target}"
repeat="${1:-20}"
out=build/acceptance
mkdir -p "$out"
failed=0
row() {
    local name="$1"
    shift
    local log="$out/$name.log"
    local start=$SECONDS
    cargo xtask test all --repeat "$repeat" "$@" >"$log" 2>&1
    local status=$?
    local results fails
    results=$(grep -c '^RESULT ' "$log")
    fails=$(grep -c 'result=fail' "$log")
    echo "ROW $name status=$status results=$results fails=$fails seconds=$((SECONDS - start)) log=$log"
    grep -E '^SUMMARY |FAILED|^xtask:|^error' "$log" | cut -c1-300
    [ "$status" -eq 0 ] || failed=1
}
row dev-kvm --accel kvm
row dev-tcg --accel tcg
row release-kvm --release --accel kvm
row release-tcg --release --accel tcg
echo "MATRIX repeat=$repeat result=$([ "$failed" -eq 0 ] && echo pass || echo fail)"
exit "$failed"
