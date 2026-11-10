#!/usr/bin/env bash
# One-time setup of the Stillkey build environment inside WSL2 (docs/07 section 2).
# Safe to run again: a step that is already done is skipped, so a second run
# asks for no password. Changes nothing on the Windows side.
set -euo pipefail
cd "$(dirname "$0")/.."

me="${USER:-$(id -un)}"
pkgs=(build-essential qemu-system-x86 ovmf mtools gdisk gdb curl ca-certificates)

echo "== 1/7 packages (sudo)"
missing=()
for p in "${pkgs[@]}"; do
  if ! dpkg-query -W -f='${Status}' "$p" 2>/dev/null | grep -q 'install ok installed'; then
    missing+=("$p")
  fi
done
if ((${#missing[@]})); then
  echo "missing: ${missing[*]}"
  sudo apt-get update
  sudo apt-get install -y "${pkgs[@]}"
else
  echo "already installed"
fi

echo "== 2/7 kvm group (sudo)"
if id -nG "$me" | tr ' ' '\n' | grep -qx kvm; then
  echo "already a member"
else
  sudo usermod -aG kvm "$me"
  echo "added; takes effect after: wsl.exe --terminate Ubuntu-26.04"
fi

echo "== 3/7 rustup"
if [ ! -x "$HOME/.cargo/bin/rustup" ]; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain none
else
  echo "already installed"
fi

echo "== 4/7 cargo on PATH for this script"
# shellcheck disable=SC1091
. "$HOME/.cargo/env"

echo "== 5/7 pinned toolchain (rust-toolchain.toml)"
rustup toolchain install

echo "== 6/7 lock file"
if [ -f Cargo.lock ]; then
  echo "Cargo.lock exists"
else
  cargo generate-lockfile
fi

echo "== 7/7 setup-check"
bash tools/sk.sh setup-check
echo "== setup finished"
