#!/usr/bin/env bash
# Port a Claude Code session transcript into this repo's project directory, so
# `claude --resume <id>` works from here.
#
# WHY THIS IS NEEDED: Claude Code keys sessions to the working directory they
# ran in. This project was started from ~/projects/camaps, so its transcript
# lives under that directory's project folder and `--resume` cannot see it from
# here. Copying the .jsonl across makes it resumable in this repo.
#
# RE-RUN IT AT THE END OF A SESSION. A live session is still being appended to,
# so a copy taken mid-session stops wherever it was taken.
#
# Usage:  tools/port-session.sh <session-uuid>
#         tools/port-session.sh            # lists candidates, newest first

set -euo pipefail

ROOT="${HOME}/.claude/projects"
# Claude Code munges the absolute path: / and _ become -
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="${ROOT}/$(printf '%s' "$HERE" | tr '/_' '--')"

if [[ $# -eq 0 ]]; then
  echo "Sessions, newest first. Re-run with the one you want:"
  echo
  find "$ROOT" -maxdepth 2 -name '*.jsonl' -printf '%T@ %TY-%Tm-%Td %TH:%TM  %10s  %p\n' \
    | sort -rn | head -15 | cut -d' ' -f2-
  echo
  echo "  this repo resumes from: $DEST"
  exit 0
fi

ID="$1"
SRC="$(find "$ROOT" -maxdepth 2 -name "${ID}.jsonl" -print -quit)"

if [[ -z "$SRC" ]]; then
  echo "no transcript for session ${ID}" >&2
  exit 1
fi

mkdir -p "$DEST"
cp -p "$SRC" "${DEST}/${ID}.jsonl"

echo "  from  $SRC"
echo "  to    ${DEST}/${ID}.jsonl"
echo "  size  $(stat -c%s "${DEST}/${ID}.jsonl" | numfmt --to=iec)"
echo
echo "  cd ${HERE} && claude --resume ${ID}"
