#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${ROOT}/docs/assets"
SHOT_HOME="${SHOT_HOME:-/tmp/readme-shot-home}"
BIN="${BIN:-${ROOT}/target/release/ai-usage-tui}"
DB="${DB:-${ROOT}/tests/fixtures/opencode_test.db}"
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

RUN_CMD="${BIN} --db ${DB} --journal ${JOURNAL} --config ${CONFIG} --all"

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

echo "Wrote screenshots to ${OUT_DIR}"
