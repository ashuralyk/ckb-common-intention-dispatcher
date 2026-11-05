#!/usr/bin/env bash
# Build CID lock + Process example RISC-V binaries into build/release/.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

mkdir -p build/release
export TOP="$ROOT"
export BUILD_DIR="build/release"
export MODE="${MODE:-release}"

echo "Building CID lock contract..."
make -e -C contracts/ckb_common_intention_dispatcher build

echo "Building Process example contract..."
make -e -C contracts/cid_process_example build

echo ""
echo "All contracts built successfully:"
ls -lh build/release/
