#!/usr/bin/env bash
# Linux and macOS counterpart of hosted-library-capture.ps1.
#
# 1. Imports clearly labeled synthetic test games through the CLI.
# 2. Launches the real player with the bundled save fixture twice (--smoke) and checks that its
#    localStorage counter persisted across launches.
# 3. Captures the real launcher's WebView for several views and checks each PNG.
#
# Usage: scripts/hosted-library-capture.sh BIN_DIR OUT_DIR
# On a headless Linux machine run it under `xvfb-run -a`.
set -euo pipefail

bin_dir=${1:?usage: hosted-library-capture.sh BIN_DIR OUT_DIR}
out_dir=${2:?usage: hosted-library-capture.sh BIN_DIR OUT_DIR}
repo=$(cd "$(dirname "$0")/.." && pwd)
cli="$bin_dir/gamestrike"
launcher="$bin_dir/gamestrike-launcher"
player="$bin_dir/gamestrike-player"
for program in "$cli" "$launcher" "$player"; do
  [[ -x "$program" ]] || { echo "missing $program" >&2; exit 1; }
done

# macOS has no `timeout` by default; perl's alarm gives the same time limit.
run_limited() {
  local seconds=$1
  shift
  if command -v timeout >/dev/null; then
    timeout "$seconds" "$@"
  else
    perl -e 'alarm shift; exec @ARGV or die "exec: $!"' "$seconds" "$@"
  fi
}

mkdir -p "$out_dir"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
library="$work/library"
fixtures="$work/fixtures"
mkdir -p "$library" "$fixtures"

# --- Player smoke test with the exact bundled save fixture -----------------------------------
smoke_library="$work/smoke-library"
mkdir -p "$smoke_library"
cp "$repo/fixtures/save-counter.html" "$work/Save Counter (Synthetic Test).html"
"$cli" --library "$smoke_library" import "$work/Save Counter (Synthetic Test).html"
smoke_id=$("$cli" --library "$smoke_library" list | cut -f1)
for launch in 1 2; do
  output=$(run_limited 60 "$player" --library "$smoke_library" --id "$smoke_id" --smoke 2>"$out_dir/player-smoke-$launch.err.txt")
  printf '%s\n' "$output" | tee "$out_dir/player-smoke-$launch.txt"
  grep -qx "GameStrike fixture launch $launch" <<<"$output" \
    || { echo "player smoke launch $launch did not report its saved counter" >&2; cat "$out_dir/player-smoke-$launch.err.txt" >&2; exit 1; }
  grep -qx "gamestrike-network-blocked=0" <<<"$output" \
    || { echo "player smoke launch $launch reported blocked requests" >&2; exit 1; }
done
"$player" --library "$smoke_library" --id "$smoke_id" --clear-engine-data
echo "Player smoke test: the save counter persisted across two launches."

# --- Launcher captures ------------------------------------------------------------------------
titles=(
  'Orbit Garden (Synthetic Test)'
  'Pixel Harbor (Synthetic Test)'
  'Lantern Maze (Synthetic Test)'
  'Comet Courier (Synthetic Test)'
  'Tiny Tactics (Synthetic Test)'
  'A Deliberately Long Title For Layout Checks In The Library (Synthetic Test)'
)
for title in "${titles[@]}"; do
  file="$fixtures/$title.html"
  printf '<!doctype html><title>%s</title><h1>%s</h1><p>GameStrike synthetic test fixture.</p>' "$title" "$title" >"$file"
  "$cli" --library "$library" import "$file"
done

# Give the synthetic games real library data: sources, favorites, a custom cover, and a ZIP whose
# own icon becomes its automatic cover. Every image is generated here; none is third-party art.
id_of() { "$cli" --library "$library" list | grep -F "$1" | cut -f1 | head -1; }
"$cli" --library "$library" set-source "$(id_of 'Orbit Garden')" itch.io
"$cli" --library "$library" set-source "$(id_of 'Pixel Harbor')" itch.io
"$cli" --library "$library" set-source "$(id_of 'Lantern Maze')" Newgrounds
"$cli" --library "$library" favorite "$(id_of 'Orbit Garden')" on
"$cli" --library "$library" favorite "$(id_of 'Comet Courier')" on
node "$repo/scripts/make-test-cover.cjs" "$work/cover.png" 125
"$cli" --library "$library" set-cover "$(id_of 'Pixel Harbor')" "$work/cover.png"
icon_game="$work/icon-game"
mkdir -p "$icon_game"
printf '<!doctype html><title>Signal Tower (Synthetic Test)</title><h1>Signal Tower (Synthetic Test)</h1>' >"$icon_game/index.html"
node "$repo/scripts/make-test-cover.cjs" "$icon_game/icon.png" 280
node "$repo/scripts/make-test-zip.cjs" "$work/Signal Tower (Synthetic Test).zip" "$icon_game/index.html" "$icon_game/icon.png"
"$cli" --library "$library" import "$work/Signal Tower (Synthetic Test).zip"
"$cli" --library "$library" list | tee "$out_dir/library-list.txt"
grep -F 'Signal Tower' "$out_dir/library-list.txt" | grep -q 'cover: auto' \
  || { echo "the ZIP's icon did not become its automatic cover" >&2; exit 1; }
grep -F 'Pixel Harbor' "$out_dir/library-list.txt" | grep -q 'cover: custom' \
  || { echo "the custom cover was not stored" >&2; exit 1; }

# Reads the width and height from a PNG's IHDR chunk.
png_size() {
  local bytes
  bytes=$(od -An -tu1 -j16 -N8 "$1" | tr -s ' \n' ' ')
  set -- $bytes
  echo "$(( ($1 << 24) | ($2 << 16) | ($3 << 8) | $4 )) $(( ($5 << 24) | ($6 << 16) | ($7 << 8) | $8 ))"
}

capture() {
  local lib=$1 name=$2
  shift 2
  local png="$out_dir/$name.png"
  rm -f "$png"
  if ! run_limited 90 "$launcher" --library "$lib" --capture-preview "$png" "$@" \
      >"$out_dir/$name.out.txt" 2>"$out_dir/$name.err.txt"; then
    cat "$out_dir/$name.out.txt" "$out_dir/$name.err.txt" >&2
    echo "launcher did not capture $name" >&2
    exit 1
  fi
  [[ -f "$png" ]] || { echo "launcher wrote no PNG for $name" >&2; exit 1; }
  [[ "$(od -An -tx1 -N8 "$png" | tr -d ' \n')" == "89504e470d0a1a0a" ]] \
    || { echo "$name capture is not a PNG" >&2; exit 1; }
  read -r width height < <(png_size "$png")
  if (( width < 880 || height < 620 )); then
    echo "$name capture is too small: ${width}x${height}" >&2
    exit 1
  fi
  local sum
  sum=$( (command -v sha256sum >/dev/null && sha256sum "$png" || shasum -a 256 "$png") | cut -d' ' -f1)
  echo "$name : ${width}x${height} sha256=$sum"
  grep -h "scene renderer" "$out_dir/$name.out.txt" | sed "s/^/  /" || true
}

capture "$library" library
capture "$library" focus --capture-view focus
capture "$library" long-title --capture-view long
capture "$library" network-dialog --capture-view launch
capture "$library" manage --capture-view manage
capture "$library" about --capture-view about
capture "$library" favorites --capture-view favorites
capture "$library" collapsed --capture-view collapsed
capture "$library" search --capture-view search
capture "$library" boot --capture-view boot
capture "$library" minimum --window-size 880 620
empty="$work/empty-library"
mkdir -p "$empty"
capture "$empty" empty
echo "Launcher captures are in $out_dir"
