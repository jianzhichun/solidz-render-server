#!/bin/bash
# Memory per scenario (Linux, /proc): the M1 snapshot run (30 frames, read 3 s into the hold), then a
# leg-A orbit (120 moves at 120 Hz), at dpr 1 and 2. Samples every 100 ms: "ms VmRSS RssAnon VmHWM
# RssShmem" (KiB). On lavapipe the device's memory is shared memory (RssShmem), malloc is RssAnon,
# RssFile is the binary's pages. On a Mac read `footprint PID` instead.
#   DESIGN=fan.solidz spike/mem.sh LABEL [VAR=VALUE ...]     (from the workspace root; MSAA=N adds --msaa N)
set -u
label=$1; shift
for kv in "$@"; do export "$kv"; done
BIN=${BIN:-${CARGO_TARGET_DIR:-target}/release/solidcraft-render-server}
NODE=${NODE:-node}
S=apps/solidcraft-render-server/spike
# The `gpu memory:` line at stop is an info line. On a host with a CPU quota and no GPU, also set
# LP_NUM_THREADS and RAYON_NUM_THREADS to the quota (M3).
export SOLIDCRAFT_RENDER_LOG=${SOLIDCRAFT_RENDER_LOG:-info}
DESIGN=${DESIGN:?a design to open, e.g. the fan}
O=out/mem/$label; mkdir -p "$O"
if [ -n "${MSAA:-}" ]; then
  # leg-a.mjs passes fixed flags: a wrapper adds this one.
  printf '#!/bin/sh\nexec %s "$@" --msaa %s\n' "$BIN" "$MSAA" > "$O/bin.sh"; chmod +x "$O/bin.sh"; BIN=$PWD/$O/bin.sh
fi
sample() {
  local pid=$1 out=$2 t0; t0=$(date +%s%N)
  while [ -e /proc/$pid/status ]; do
    awk -v t=$(( ($(date +%s%N) - t0) / 1000000 )) '/^VmRSS/{r=$2}/^RssAnon/{a=$2}/^VmHWM/{h=$2}/^RssShmem/{s=$2}END{if(r)print t,r,a,h,s}' /proc/$pid/status >> "$out" 2>/dev/null
    sleep 0.1
  done
}
summ() { awk '{if($2>r)r=$2;if($3>a)a=$3;if($5>s)s=$5;h=$4;la=$3;lr=$2;ls=$5}END{printf "max RSS %.0f Anon %.0f Shmem %.0f HWM %.0f | last RSS %.0f Anon %.0f Shmem %.0f MiB\n",r/1024,a/1024,s/1024,h/1024,lr/1024,la/1024,ls/1024}' "$1"; }
for dpr in ${DPRS:-1 2}; do
  out=$O/m1-dpr$dpr; mkdir -p "$out"
  export SOLIDCRAFT_CONTROL_TOKEN=$(openssl rand -hex 32)
  $BIN --snapshot "$out" --frames 30 --open "$DESIGN" --size 1280x800 --dpr $dpr --hold-ms 6000 2> "$out/run.log" &
  pid=$!
  sample $pid "$out/rss.txt" &
  until grep -q 'frames done' "$out/run.log" 2>/dev/null || ! kill -0 $pid 2>/dev/null; do sleep 0.2; done
  sleep 3
  hold=$(awk '/^VmRSS/{r=$2}/^RssAnon/{a=$2}/^RssFile/{f=$2}/^RssShmem/{s=$2}END{printf "RSS %.0f Anon %.0f Shmem %.0f File %.0f MiB",r/1024,a/1024,s/1024,f/1024}' /proc/$pid/status)
  wait $pid; rc=$?; wait
  echo "$label m1 dpr$dpr exit $rc hold: $hold | $(summ "$out/rss.txt")"
  name=$O/orbit-dpr$dpr
  $NODE $S/leg-a.mjs --bin "$BIN" --open "$DESIGN" --size 1280x800 --dpr $dpr --case orbit --moves 120 --hz 120 > "$name.json" 2> "$name.log" &
  np=$!
  c=; for _ in $(seq 200); do c=$(pgrep -P $np | head -1); [ -n "$c" ] && break; sleep 0.05; done
  [ -n "$c" ] && sample "$c" "$name.rss.txt" &
  wait $np; rc=$?; wait
  echo "$label orbit dpr$dpr exit $rc | $(summ "$name.rss.txt")"
  grep -h 'gpu memory' "$name.log" | sed 's/^.*gpu memory: /  gpu: /'
done
