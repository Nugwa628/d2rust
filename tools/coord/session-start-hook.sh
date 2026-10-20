#!/bin/sh
# Claude Code SessionStart hook (.claude/settings.json): runs when a session
# starts, resumes, or comes back from auto-compaction; what it prints goes
# into the session's context. It makes the coord-thread rule (docs/LANE-BRIEF.md)
# reach every session, including ones that started
# before a rule changed or forgot it at compaction.
cd "$(dirname "$0")/../.." || exit 0
cat <<'EOF'
[coord] Sessions talk through `coord` threads on the forge (docs/LANE-BRIEF.md, docs/METHODS.md).
Now, before other work:
1. Read the threads you were given (and the run thread): `sh tools/coord/forge.sh read <n>`.
2. Start the watcher as a background monitor and keep it running for the whole session:
   `FORGE_FROM=<your name> sh tools/coord/forge.sh watch <n>...` (a coordinator: no numbers = every thread).
   It checks every second, prints only other sessions' posts, costs no tokens while quiet,
   and catches up on posts made while it was down. A monitor expires after 30 min: re-arm it every time.
3. Answer each post as it lands. No acknowledgement posts.
EOF
if [ -z "${FORGEJO_TOKEN:-}" ]; then
  echo "[coord] FORGEJO_TOKEN is not set in this environment: threads cannot be read until it is (ask the owner)."
  exit 0
fi
echo "[coord] Open threads:"
sh tools/coord/forge.sh inbox 2>/dev/null | head -15 || echo "  (could not reach the forge)"
exit 0
