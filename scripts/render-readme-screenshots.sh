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
# The demo GIF is the same renderer walking a key script: one frame per key, through the same
# dispatch the dashboard's event loop uses, assembled by ImageMagick. Its script is DEMO_SCRIPT
# below; `hold` repeats a frame, which is how a pause is spelled.
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
# The data root the renderer is pinned to. The statusline cache below lands here, and the
# update and pricing caches would if anything wrote them; nothing of the author's does.
DATA_HOME="${WORK}/data-home"

# The demo, key by key: open on the model list; the routing panel and a pause on it; sort by
# the next column and reverse it; the projects panel, two rows down, drill into the project;
# back out; the limits panel and a pause; the key reference. Every token is a real binding.
DEMO_SCRIPT="hold,t,hold,hold,>,o,hold,p,down,down,enter,hold,esc,l,hold,hold,?,hold"
# Hundredths of a second per frame. A held frame repeats, so a pause is a multiple of this.
FRAME_DELAY=110

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

# The limits panel reads what Claude Code last pushed to `--statusline`, from the data root. The
# fixture's payload goes through the real command into the scratch root, so the panel and the
# header's limits line show invented windows -- and the author's real cache, which lives in the
# real data root, is never in the picture.
XDG_DATA_HOME="${DATA_HOME}" "${BIN}" --statusline <"${CLAUDE_DIR}/statusline.json" >/dev/null

# The data root has no flag, and the statusline, update and pricing caches all resolve from it;
# the renderer refuses to run unless it is pinned. A statusline cache left unpinned would put
# the author's own rate-limit window into every header below.
XDG_DATA_HOME="${DATA_HOME}" \
cargo run --release --locked --quiet --manifest-path "${ROOT}/Cargo.toml" \
  --example render-screenshots -- \
  "${SVG_DIR}" --claude-dir "${CLAUDE_DIR}" --codex-dir "${CLAUDE_DIR}/no-codex-home" \
  --copilot-dir "${CLAUDE_DIR}/no-copilot-home" --gemini-dir "${CLAUDE_DIR}/no-gemini-home" \
  --omarchy-dir "${CLAUDE_DIR}/no-omarchy" \
  --db "${DEMO_DB}" --journal "${JOURNAL}" \
  --config "${CONFIG}" --script "${DEMO_SCRIPT}" >/dev/null

mkdir -p "${OUT_DIR}"
for svg in "${SVG_DIR}"/*.svg; do
  name="$(basename "${svg}" .svg)"
  [[ "${name}" == frame-* ]] && continue   # the demo's frames are assembled below, not kept
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

# The GIF: every frame at the SVG's own size (the README shows it at column width, and 2x
# would double a file that is committed), the same 64-colour quantisation as the stills, and
# ImageMagick's frame optimiser so a held frame costs nothing. Needs ImageMagick, unlike the
# stills, so it is skipped with a warning rather than failing the run.
FRAMES=("${SVG_DIR}"/frame-*.svg)
if [[ -e "${FRAMES[0]}" ]]; then
  if command -v magick >/dev/null; then
    FRAME_DIR="${WORK}/frames"
    mkdir -p "${FRAME_DIR}"
    for svg in "${FRAMES[@]}"; do
      rsvg-convert --format=png --output="${FRAME_DIR}/$(basename "${svg}" .svg).png" "${svg}"
    done
    magick -delay "${FRAME_DELAY}" -loop 0 "${FRAME_DIR}"/frame-*.png \
      -colors 64 -layers Optimize "${OUT_DIR}/demo.gif"
    echo "${OUT_DIR}/demo.gif ($(( $(stat -c %s "${OUT_DIR}/demo.gif") / 1024 )) KB, ${#FRAMES[@]} frames)"
  else
    echo "no ImageMagick (magick): the demo GIF was not assembled" >&2
  fi
fi
