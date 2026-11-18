#!/bin/sh
# What one session of the render server writes under the user's own folders (Linux): a fresh HOME
# with no XDG_* overrides, one snapshot run that opens DESIGN and edits it (m2-edit.json), then
# every file and folder made under that HOME, and the log. Extra arguments go to the render server
# (e.g. --config-dir DIR, to check that nothing lands in HOME then).
#   spike/userdirs.sh BIN DESIGN OUTDIR [render-server args...]
set -eu
bin=$1 design=$2 out=$3
shift 3
here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp -d)
mkdir -p "$t/home" "$out"
token=$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')
status=0
env -u XDG_CONFIG_HOME -u XDG_DATA_HOME -u XDG_CACHE_HOME -u XDG_STATE_HOME HOME="$t/home" SOLIDCRAFT_CONTROL_TOKEN="$token" \
    "$bin" --snapshot "$t/snap" --frames 20 --open "$design" --script "$here/m2-edit.json" "$@" 2> "$out/log.txt" || status=$?
echo "exit $status" >> "$out/log.txt"
(cd "$t/home" && find . -mindepth 1 | sort) > "$out/home-files.txt"
rm -rf "$t"
