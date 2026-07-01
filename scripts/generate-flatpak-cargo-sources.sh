#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUTPUT="${ROOT}/flatpak/cargo-sources.json"
GENERATOR="${TMPDIR:-/tmp}/flatpak-cargo-generator.py"
VENV="${TMPDIR:-/tmp}/flatpak-cargo-generator-venv"

curl -fsSL \
  -o "${GENERATOR}" \
  https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py

if [[ ! -x "${VENV}/bin/python" ]]; then
  python3 -m venv "${VENV}"
  "${VENV}/bin/pip" install --quiet aiohttp toml aiofiles tomlkit
fi

"${VENV}/bin/python" "${GENERATOR}" "${ROOT}/Cargo.lock" -o "${OUTPUT}"
echo "Wrote ${OUTPUT}"
