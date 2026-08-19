#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Requires an X11 display plus xfce4-terminal, xdotool and scrot. On Arch:
#   sudo pacman -S --needed xfce4-terminal xdotool scrot imagemagick
# On a Wayland session, run it against Xwayland or an Xvfb display (`DISPLAY=:1 Xvfb :1 &`).
for tool in xfce4-terminal xdotool scrot convert; do
  command -v "${tool}" >/dev/null || { echo "missing ${tool} — see the note above" >&2; exit 1; }
done
OUT_DIR="${ROOT}/docs/assets"
SHOT_HOME="${SHOT_HOME:-/tmp/readme-shot-home}"
BIN="${BIN:-${ROOT}/target/release/ai-usage-tui}"
DB="${DB:-${ROOT}/tests/fixtures/opencode_test.db}"
# Claude Code transcripts, generated fresh so the day-based panels always end on today.
CLAUDE_DIR="${CLAUDE_DIR:-${SHOT_HOME}/.claude}"
JOURNAL="${SHOT_HOME}/.local/share/ai-usage-tui/usage.db"
CONFIG="${SHOT_HOME}/.config/ai-usage-tui/config.toml"
DISPLAY="${DISPLAY:-:1}"
XAUTHORITY="${XAUTHORITY:-${HOME}/.Xauthority}"

mkdir -p "${OUT_DIR}" "${SHOT_HOME}/.config/ai-usage-tui" "${SHOT_HOME}/.local/share/ai-usage-tui"
if [[ ! -x "${BIN}" ]]; then
  cargo +stable build --release --manifest-path "${ROOT}/Cargo.toml" >/dev/null
fi

cat >"${CONFIG}" <<'EOF'
[[budgets.entry]]
scope = "global"
period = "monthly"
limit = 1.0
warn = 50.0
critical = 80.0

[[budgets.entry]]
scope = "provider"
name = "opencode"
period = "monthly"
limit = 1.2
warn = 75.0
critical = 90.0

[[budgets.entry]]
scope = "model"
name = "gpt-5.6-sol"
period = "monthly"
limit = 0.30
warn = 75.0
critical = 90.0

[collectors.opencode]
enabled = false

[collectors.journal]
enabled = false
EOF

rm -f "${JOURNAL}"

# The OpenCode fixture alone cannot fill the dashboard: it is nine rows on one day in 2023 with no
# session ids and no project paths, so projects, sessions, spend-over-time and burn all capture
# empty. This writes a small invented dataset with the shape those panels exist to show.
python3 "${ROOT}/scripts/make-demo-fixture.py" "${CLAUDE_DIR}"

record_routing() {
  echo "$1" | HOME="${SHOT_HOME}" "${BIN}" --record-routing --journal "${JOURNAL}" --db "${DB}" >/dev/null
}

if [[ ! -s "${JOURNAL}" ]]; then
  record_routing '{"agent":"heavy","model":"gpt-5.1-codex","provider":"opencode","task":"refactor","phase":"implementation","tokens":15000,"cost":0.02,"retries":1,"escalations":0,"test_result":true,"review_defects":0}'
  record_routing '{"agent":"heavy2","model":"gpt-5.6-sol","provider":"opencode","task":"tests","phase":"verification","tokens":22000,"cost":0.03,"retries":0,"escalations":1,"test_result":false,"review_defects":2}'
  record_routing '{"agent":"junior","model":"nemotron-3-ultra-free","provider":"opencode","task":"docs","phase":"polish","tokens":8000,"cost":0.0,"retries":0,"escalations":0,"test_result":true,"review_defects":0}'
  record_routing '{"agent":"local","model":"qwen3.6-35b-a3b","provider":"llamacpp","task":"lint","phase":"cleanup","tokens":12000,"cost":0.0,"retries":0,"escalations":0,"test_result":true,"review_defects":0}'
fi

cleanup() {
  if [[ -n "${TERM_PID:-}" ]] && kill -0 "${TERM_PID}" 2>/dev/null; then
    kill "${TERM_PID}" 2>/dev/null || true
    wait "${TERM_PID}" 2>/dev/null || true
  fi
}
trap cleanup EXIT

RUN_CMD="${BIN} --db ${DB} --journal ${JOURNAL} --config ${CONFIG} --claude-dir ${CLAUDE_DIR} --all"

HOME="${SHOT_HOME}" DISPLAY="${DISPLAY}" XAUTHORITY="${XAUTHORITY}" xfce4-terminal \
  --title="ai-usage-tui-screenshot" \
  --geometry=132x38 \
  --hide-toolbar \
  --hide-menubar \
  --hide-scrollbar \
  --color-bg='#0a1014' \
  --color-text='#d8e2eb' \
  -e "bash -lc '${RUN_CMD}; read'" &
TERM_PID=$!

for _ in $(seq 1 50); do
  WIN_ID="$(xdotool search --name "ai-usage-tui-screenshot" 2>/dev/null | head -n1 || true)"
  if [[ -n "${WIN_ID}" ]]; then
    break
  fi
  sleep 0.2
done

if [[ -z "${WIN_ID:-}" ]]; then
  echo "failed to find screenshot terminal window" >&2
  exit 1
fi

xdotool windowactivate --sync "${WIN_ID}"
xdotool windowmove "${WIN_ID}" 40 40
sleep 2
xdotool key --window "${WIN_ID}" r
sleep 0.8

capture() {
  local output="$1"
  scrot -o -b "${output}" --window "${WIN_ID}"
  convert "${output}" -crop 1168x724+12+34 +repage "${output}"
}

send_key() {
  local key="$1"
  xdotool windowfocus --sync "${WIN_ID}"
  xdotool mousemove --window "${WIN_ID}" 600 360 click 1
  sleep 0.2
  xdotool key --window "${WIN_ID}" --clearmodifiers "${key}"
  sleep 0.8
}

capture "${OUT_DIR}/dashboard.png"

send_key b
capture "${OUT_DIR}/budgets.png"

send_key t
capture "${OUT_DIR}/routing.png"

send_key p
capture "${OUT_DIR}/projects.png"

send_key g
capture "${OUT_DIR}/timeseries.png"

send_key w
capture "${OUT_DIR}/burn.png"

send_key s
capture "${OUT_DIR}/sessions.png"

# Every capture must show its panel populated. An empty panel in the README is worse than none,
# and the fixture is the only thing standing between us and that.
for shot in dashboard budgets routing projects timeseries burn sessions; do
  [[ -s "${OUT_DIR}/${shot}.png" ]] || { echo "capture failed: ${shot}.png" >&2; exit 1; }
done

echo "Wrote screenshots to ${OUT_DIR}"
