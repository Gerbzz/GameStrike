#!/usr/bin/env bash
# Exercise the real recorder's exit/cleanup contract without a GUI, encoder or GameStrike binary.
set -euo pipefail
script=${1:-$(dirname "$0")/record-startup.sh}
work=$(mktemp -d)
mkdir -p "$work/bin" "$work/fake-path"
cat >"$work/bin/gamestrike" <<'CLI'
#!/usr/bin/env bash
exit 0
CLI
cat >"$work/bin/gamestrike-launcher" <<'LAUNCHER'
#!/usr/bin/env bash
printf '%s\n' "$$" >"$RECORD_PID_FILE"
case "$RECORD_CASE" in
  launcher-zero) exit 0 ;;
  launcher-error) exit 42 ;;
esac
exec sleep 3
LAUNCHER
cat >"$work/fake-path/ffmpeg" <<'ENCODER'
#!/usr/bin/env bash
case "$RECORD_CASE" in
  encoder-error) exit 9 ;;
  empty) exit 0 ;;
esac
if [[ " $* " == *' x11grab '* ]]; then
  sleep 0.2
elif [[ "$RECORD_CASE" == conversion-error ]]; then
  exit 9
fi
printf 'test recording bytes\n' >"${@: -1}"
ENCODER
chmod +x "$work/bin/"* "$work/fake-path/ffmpeg"
export PATH="$work/fake-path:$PATH"
failed=0
for mode in launcher-zero launcher-error encoder-error empty conversion-error success; do
  export RECORD_CASE="$mode" RECORD_PID_FILE="$work/pid-$mode"
  mkdir -p "$work/out-$mode"
  status=0
  bash "$script" "$work/bin" "$work/out-$mode" >"$work/result-$mode.log" 2>&1 || status=$?
  expected=1
  [[ "$mode" != success ]] || expected=0
  if { [[ "$expected" = 0 && "$status" != 0 ]] || [[ "$expected" = 1 && "$status" = 0 ]]; }; then
    echo "FAIL $mode: unexpected recorder status $status" >&2
    cat "$work/result-$mode.log" >&2
    failed=1
  else
    echo "PASS $mode (status $status)"
  fi
  if [[ -s "$RECORD_PID_FILE" ]] && kill -0 "$(cat "$RECORD_PID_FILE")" 2>/dev/null; then
    echo "FAIL $mode: recorder left the fake launcher running" >&2
    kill "$(cat "$RECORD_PID_FILE")" 2>/dev/null || true
    failed=1
  fi
done
echo "Test evidence: $work"
exit "$failed"
