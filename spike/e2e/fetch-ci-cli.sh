#!/usr/bin/env bash
# fetch-ci-cli.sh — download the latest successful ci.yml linux-x86_64
# build artifact and print the binary path on stdout (last line).
#
# This is the GitHub-side of the two build paths (AGENTS.md "build
# paths"): the real-device harnesses (loop.py / benchmark.py /
# loop_v3.py / ghost_repro.py / inspector.py / t6_verify.py) all honor
# the PILOTDROID_CLI override, so a typical device-verify round becomes
#
#     export PILOTDROID_CLI="$(spike/e2e/fetch-ci-cli.sh)"
#     python3 spike/e2e/agent-loop/benchmark.py ...
#
# Local `cargo build -p pilotdroid-cli` remains the other path and is
# what you want when testing unpushed serve.rs changes — the artifact
# only exists for commits CI has already built.
#
# Requires: gh (authenticated). Optional arg: explicit workflow run id.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

if [ "${1:-}" != "" ]; then
    RUN_ID="$1"
else
    RUN_ID=$(gh run list --workflow ci.yml --branch main --status success \
        --limit 1 --json databaseId --jq '.[0].databaseId')
fi
if [ -z "$RUN_ID" ] || [ "$RUN_ID" = "null" ]; then
    echo "fetch-ci-cli: no successful ci.yml run on main found" >&2
    exit 1
fi

DEST="${XDG_CACHE_HOME:-$HOME/.cache}/pilotdroid/ci-cli/$RUN_ID"
mkdir -p "$DEST"
# Skip re-download when this run's artifact is already cached.
if [ ! -x "$DEST/pilotdroid-cli" ]; then
    gh run download "$RUN_ID" --name pilotdroid-cli-linux-x86_64 \
        --dir "$DEST"
    chmod +x "$DEST/pilotdroid-cli"
fi
echo "fetch-ci-cli: run $RUN_ID -> $DEST/pilotdroid-cli" >&2
"$DEST/pilotdroid-cli" --version >&2 || true
echo "$DEST/pilotdroid-cli"
