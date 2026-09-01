#!/usr/bin/env bash
# Regenerate the README images.
#
# These are rendered, not photographed. The dashboard is drawn through ratatui's off-screen
# backend into an SVG (see src/ui/svg.rs), then rasterised. Nothing opens a terminal and nothing
# takes a picture of a screen, so this runs headlessly, gives the same render on any machine, and
# cannot put any part of the author's desktop into a file this repository publishes.
#
# The images are not byte-reproducible across days: the demo dataset is anchored to today, so the
# burn window and the spend-over-time chart have recent activity to show. What is reproducible is
# the render — the same data always produces the same picture.
#
#   sudo pacman -S --needed librsvg    # Arch; provides rsvg-convert
#   sudo apt install librsvg2-bin      # Debian/Ubuntu
#
# Pass --svg to keep the intermediate SVGs alongside the PNGs.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${OUT_DIR:-${ROOT}/docs/assets}"
# Named, because the header renders the source path and it lands in every image: a reader
# should be able to see at a glance that these are demo files, not the author's own.
WORK="$(mktemp -d -t ai-usage-demo-XXXXXX)"
trap 'rm -rf "${WORK}"' EXIT

KEEP_SVG=0
[[ "${1:-}" == "--svg" ]] && KEEP_SVG=1

command -v rsvg-convert >/dev/null || {
  echo "missing rsvg-convert (librsvg); see the header of this script" >&2; exit 1; }
command -v python3 >/dev/null || { echo "missing python3" >&2; exit 1; }

CLAUDE_DIR="${WORK}/claude"
DEMO_DB="${CLAUDE_DIR}/opencode.db"
JOURNAL="${WORK}/usage.db"
CONFIG="${WORK}/config.toml"
SVG_DIR="${WORK}/svg"

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

[collectors.copilot]
enabled = false

[collectors.gemini]
enabled = false
EOF

# The panels need data with sessions, projects, several days and a model escalation; see the
# generator's docstring for why the test fixture cannot stand in.
python3 "${ROOT}/scripts/make-demo-fixture.py" "${CLAUDE_DIR}"

cargo build --release --locked --manifest-path "${ROOT}/Cargo.toml" >/dev/null
BIN="${ROOT}/target/release/ai-usage-tui"

record_routing() {
  echo "$1" | "${BIN}" --record-routing --journal "${JOURNAL}" >/dev/null
}
record_routing '{"agent":"implementer","model":"claude-opus-5","provider":"anthropic","task":"auth-refactor","phase":"implementation","tokens":142000,"cost":0.71,"retries":0,"escalations":0,"test_result":true,"review_defects":0}'
record_routing '{"agent":"implementer","model":"claude-opus-5","provider":"anthropic","task":"rate-limit","phase":"implementation","tokens":98000,"cost":0.49,"retries":1,"escalations":0,"test_result":true,"review_defects":1}'
record_routing '{"agent":"drafter","model":"claude-haiku-4-5","provider":"anthropic","task":"changelog","phase":"polish","tokens":31000,"cost":0.04,"retries":2,"escalations":1,"test_result":false,"review_defects":2}'
record_routing '{"agent":"drafter","model":"claude-haiku-4-5","provider":"anthropic","task":"docstrings","phase":"polish","tokens":26000,"cost":0.03,"retries":0,"escalations":0,"test_result":true,"review_defects":0}'
record_routing '{"agent":"reviewer","model":"claude-sonnet-5","provider":"anthropic","task":"api-review","phase":"verification","tokens":74000,"cost":0.22,"retries":0,"escalations":0,"test_result":true,"review_defects":0}'

cargo run --release --locked --quiet --manifest-path "${ROOT}/Cargo.toml" \
  --example render-screenshots -- \
  "${SVG_DIR}" --claude-dir "${CLAUDE_DIR}" --codex-dir "${CLAUDE_DIR}/no-codex-home" \
  --copilot-dir "${CLAUDE_DIR}/no-copilot-home" --gemini-dir "${CLAUDE_DIR}/no-gemini-home" \
  --omarchy-dir "${CLAUDE_DIR}/no-omarchy" \
  --db "${DEMO_DB}" --journal "${JOURNAL}" \
  --config "${CONFIG}" >/dev/null

mkdir -p "${OUT_DIR}"
for svg in "${SVG_DIR}"/*.svg; do
  name="$(basename "${svg}" .svg)"
  # 2x the SVG's own geometry, so the images stay sharp on a high-DPI display at README width.
  rsvg-convert --zoom=2 --format=png --output="${OUT_DIR}/${name}.png" "${svg}"
  [[ -s "${OUT_DIR}/${name}.png" ]] || { echo "rasterising ${name} produced nothing" >&2; exit 1; }
  # Terminal output is flat colour: 64 of them is visually lossless here and cuts the files to
  # roughly a quarter, which matters for something committed to the repository. Optional, so a
  # machine without ImageMagick still produces correct images, just larger ones.
  if command -v magick >/dev/null; then
    magick "${OUT_DIR}/${name}.png" -strip -colors 64 \
      -define png:compression-level=9 "${OUT_DIR}/${name}.png"
  fi
  (( KEEP_SVG )) && cp "${svg}" "${OUT_DIR}/${name}.svg"
  echo "${OUT_DIR}/${name}.png"
done
