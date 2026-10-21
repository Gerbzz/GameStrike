#!/usr/bin/env bash
# Records the Collection style's startup from the real launcher, for CI evidence: a small synthetic
# library, the launcher started normally (no capture mode), and the screen recorded with ffmpeg.
# Writes startup.mp4 and startup.gif to OUT_DIR. Software rendering under Xvfb is slow, so the
# recording is choppier than on a real GPU; the startup's timing is the same, only frames drop.
#
# Usage: scripts/record-startup.sh BIN_DIR OUT_DIR
# Run under `xvfb-run -a -s "-screen 0 1250x800x24"` so the 1250 x 800 window fills the screen.
set -euo pipefail

bin_dir=${1:?usage: record-startup.sh BIN_DIR OUT_DIR}
out_dir=${2:?usage: record-startup.sh BIN_DIR OUT_DIR}
command -v ffmpeg >/dev/null || { echo "ffmpeg is not installed; no recording" >&2; exit 1; }
mkdir -p "$out_dir"
work=$(mktemp -d)
launcher=""
cleanup() {
  if [[ -n "$launcher" ]]; then
    kill "$launcher" 2>/dev/null || true
    wait "$launcher" 2>/dev/null || true
  fi
  rm -rf "$work"
}
trap cleanup EXIT
library="$work/library"
mkdir -p "$library" "$work/games"
# Marks the library as synthetic test data; the launcher then shows a badge saying so.
: >"$library/SYNTHETIC-TEST-LIBRARY"
for title in 'Orbit Garden' 'Pixel Harbor' 'Lantern Maze' 'Comet Courier' 'Tiny Tactics' 'Signal Tower' 'Glass Canyon' 'Rocket Bakery' 'Paper Pilots' 'Hollow Moon'; do
  printf '<!doctype html><title>%s</title><h1>%s</h1><p>GameStrike synthetic test fixture.</p>' "$title" "$title" >"$work/games/$title.html"
  "$bin_dir/gamestrike" --library "$library" import "$work/games/$title.html" >/dev/null
done

"$bin_dir/gamestrike-launcher" --library "$library" >"$out_dir/startup-launcher.log" 2>&1 &
launcher=$!
ffmpeg -loglevel error -y -f x11grab -framerate 25 -video_size 1250x800 -i "${DISPLAY:-:99}" -t 9 -pix_fmt yuv420p "$out_dir/startup.mp4"
# A desktop recording can exist even when the app has already crashed or exited cleanly.
# Check before our intentional teardown so either unexpected exit fails the recording gate.
if ! kill -0 "$launcher" 2>/dev/null; then
  status=0
  wait "$launcher" 2>/dev/null || status=$?
  launcher=""
  echo "launcher exited during recording (status $status)" >&2
  exit 1
fi
kill "$launcher" 2>/dev/null || true
wait "$launcher" 2>/dev/null || true
launcher=""
if [[ -s "$out_dir/startup.mp4" ]]; then
  ffmpeg -loglevel error -y -i "$out_dir/startup.mp4" \
    -vf "fps=12,scale=625:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=160[p];[b][p]paletteuse=dither=bayer:bayer_scale=4" \
    "$out_dir/startup.gif"
  # A few stills from the recording, about one per second.
  ffmpeg -loglevel error -y -i "$out_dir/startup.mp4" -vf "fps=1" "$out_dir/startup-frame-%02d.png"
  ls -l "$out_dir"/startup*
else
  echo "no recording was made" >&2
  exit 1
fi
