#!/usr/bin/env bash
# Capture the README screenshots on a Wayland session.
#
# The X11 script beside this one needs xfce4-terminal, xdotool and scrot, none of which work on a
# native Wayland session. This uses the Wayland equivalents: foot for the terminal, wtype to send
# keystrokes, and grim to capture. Window geometry comes from the compositor rather than being
# guessed, so no fixed crop offsets are baked in.
#
#   sudo pacman -S --needed foot wtype grim   # Arch; most Wayland setups already have these
#
# Requires a compositor implementing wlr-foreign-toplevel (Hyprland, Sway, river, ...). The
# terminal is visible while this runs and holds keyboard focus — wtype delivers to whatever is
# focused, so do not type into another window until it finishes (about 20 seconds).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${ROOT}/docs/assets"
SHOT_HOME="${SHOT_HOME:-/tmp/readme-shot-home}"
BIN="${BIN:-${ROOT}/target/release/ai-usage-tui}"
DB="${DB:-${ROOT}/tests/fixtures/opencode_test.db}"
JOURNAL="${SHOT_HOME}/.local/share/ai-usage-tui/usage.db"
CONFIG="${SHOT_HOME}/.config/ai-usage-tui/config.toml"
CLAUDE_DIR="${SHOT_HOME}/.claude"
APP_ID="ai-usage-shot"
# 132x38 is the same geometry the X11 script used, and comfortably above the 110-column
# threshold where the footer switches to its compact form.
COLS="${COLS:-132}"
ROWS="${ROWS:-38}"
# Tuned so a tiling compositor's window height yields roughly 40 rows: dense enough to fill the
# frame, large enough to read in a README at half width.
FONT_SIZE="${FONT_SIZE:-20}"

for tool in foot wtype grim; do
  command -v "${tool}" >/dev/null || { echo "missing ${tool}" >&2; exit 1; }
done
command -v hyprctl >/dev/null || command -v swaymsg >/dev/null || {
  echo "need hyprctl or swaymsg to locate the window" >&2; exit 1; }

mkdir -p "${OUT_DIR}" "${SHOT_HOME}/.config/ai-usage-tui" "${SHOT_HOME}/.local/share/ai-usage-tui"
[[ -x "${BIN}" ]] || cargo build --release --locked --manifest-path "${ROOT}/Cargo.toml" >/dev/null

cat >"${CONFIG}" <<'EOF'
[[budgets.entry]]
scope = "global"
period = "monthly"
limit = 25.0
warn = 50.0
critical = 80.0

[[budgets.entry]]
scope = "provider"
name = "anthropic"
period = "monthly"
limit = 20.0
warn = 75.0
critical = 90.0

[[budgets.entry]]
scope = "model"
name = "claude-opus-5"
period = "monthly"
limit = 6.0
warn = 60.0
critical = 85.0

[collectors.opencode]
enabled = false

[collectors.journal]
enabled = false
EOF

rm -f "${JOURNAL}"

# The OpenCode fixture alone leaves four panels empty; see the generator's docstring.
python3 "${ROOT}/scripts/make-demo-fixture.py" "${CLAUDE_DIR}"

record_routing() {
  echo "$1" | HOME="${SHOT_HOME}" "${BIN}" --record-routing --journal "${JOURNAL}" --db "${DB}" >/dev/null
}
record_routing '{"agent":"implementer","model":"claude-opus-5","provider":"anthropic","task":"auth-refactor","phase":"implementation","tokens":142000,"cost":0.71,"retries":0,"escalations":0,"test_result":true,"review_defects":0}'
record_routing '{"agent":"implementer","model":"claude-opus-5","provider":"anthropic","task":"rate-limit","phase":"implementation","tokens":98000,"cost":0.49,"retries":1,"escalations":0,"test_result":true,"review_defects":1}'
record_routing '{"agent":"drafter","model":"claude-haiku-4-5","provider":"anthropic","task":"changelog","phase":"polish","tokens":31000,"cost":0.04,"retries":2,"escalations":1,"test_result":false,"review_defects":2}'
record_routing '{"agent":"drafter","model":"claude-haiku-4-5","provider":"anthropic","task":"docstrings","phase":"polish","tokens":26000,"cost":0.03,"retries":0,"escalations":0,"test_result":true,"review_defects":0}'
record_routing '{"agent":"reviewer","model":"claude-sonnet-5","provider":"anthropic","task":"api-review","phase":"verification","tokens":74000,"cost":0.22,"retries":0,"escalations":0,"test_result":true,"review_defects":0}'

TERM_PID=""
cleanup() {
  [[ -n "${TERM_PID}" ]] && kill "${TERM_PID}" 2>/dev/null || true
}
trap cleanup EXIT

foot --app-id="${APP_ID}" \
  --window-size-chars="${COLS}x${ROWS}" \
  --override=main.font="monospace:size=${FONT_SIZE}" \
  --override=colors.background=0a1014 \
  --override=colors.foreground=d8e2eb \
  --override=main.pad=8x8 \
  --override=csd.preferred=none \
  -- "${BIN}" --db "${DB}" --journal "${JOURNAL}" --config "${CONFIG}" \
     --claude-dir "${CLAUDE_DIR}" --all &
TERM_PID=$!

# Ask the compositor where it put the window instead of assuming a position or a crop.
geometry() {
  if command -v hyprctl >/dev/null; then
    hyprctl clients -j | python3 -c "
import sys, json
for c in json.load(sys.stdin):
    if c.get('class') == '${APP_ID}':
        x, y = c['at']; w, h = c['size']
        print(f'{x},{y} {w}x{h}')
        break
"
  else
    swaymsg -t get_tree | python3 -c "
import sys, json
def walk(n):
    if n.get('app_id') == '${APP_ID}':
        r = n['rect']; print(f\"{r['x']},{r['y']} {r['width']}x{r['height']}\"); return True
    return any(walk(c) for c in n.get('nodes', []) + n.get('floating_nodes', []))
walk(json.load(sys.stdin))
"
  fi
}

GEO=""
for _ in $(seq 1 60); do
  GEO="$(geometry)"
  [[ -n "${GEO}" ]] && break
  sleep 0.25
done
[[ -n "${GEO}" ]] || { echo "the capture window never appeared" >&2; exit 1; }

# Force a refresh and let it land. Without this the first captures show a dashboard of zeros,
# because the background collectors have not completed their first poll yet — which is exactly
# what the panels are supposed to look like before data arrives, and a terrible advert.
sleep 1.5
wtype r
sleep 3

# grim captures a screen *region*, not a window, so anything drawn on top of the terminal — a
# notification, an overlapping floating window — lands in the PNG. That is not a cosmetic risk:
# this repository's screenshots would then contain whatever else was on the author's desktop.
# Refuse to capture unless the target rectangle is clear.
assert_unobstructed() {
  hyprctl clients -j | APP_ID="${APP_ID}" GEO="${GEO}" python3 -c "
import json, os, sys

pos, size = os.environ['GEO'].split(' ')
tx, ty = (int(v) for v in pos.split(','))
tw, th = (int(v) for v in size.split('x'))
target = None
others = []
for c in json.load(sys.stdin):
    if not c.get('mapped', True) or c.get('hidden'):
        continue
    if c.get('class') == os.environ['APP_ID']:
        target = c
        continue
    others.append(c)

if target is None:
    sys.exit('the capture window disappeared')

blocking = []
for c in others:
    # Only windows sharing the target's workspace can be drawn over it.
    if c['workspace']['id'] != target['workspace']['id']:
        continue
    x, y = c['at']
    w, h = c['size']
    if x < tx + tw and tx < x + w and y < ty + th and ty < y + h:
        blocking.append(f\"{c['class']} {c['title'][:40]!r}\")

if blocking:
    sys.exit(
        'refusing to capture: these windows overlap the capture area and would appear in the '
        'screenshot:\\n  ' + '\\n  '.join(blocking)
    )
"
}

capture() {
  local name="$1"
  assert_unobstructed
  grim -g "${GEO}" "${OUT_DIR}/${name}.png"
  # An empty panel in the README is worse than no screenshot at all.
  [[ -s "${OUT_DIR}/${name}.png" ]] || { echo "capture failed: ${name}" >&2; exit 1; }
}

send_key() {
  wtype "$1"
  sleep 0.9
}

capture dashboard
send_key b; capture budgets
send_key t; capture routing
send_key p; capture projects
send_key g; capture timeseries
send_key w; capture burn
send_key s; capture sessions

echo "Wrote screenshots to ${OUT_DIR}"
