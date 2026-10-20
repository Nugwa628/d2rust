#!/bin/sh
# Session sync: fetch staging and merge it into the current branch (every
# session, every 20 minutes). Merge, never rebase.
#
#   sh tools/coord/sync.sh [--base BRANCH] [--no-fetch]
#
# Conflicts that are always mechanical are resolved here:
#   specs/**.md, block inside the <!-- index --> table  -> HEAD side, then tools/spec_index.py
#                                                          (strips the old block: the index is generated)
#   docs/handoff/provisional-index*.tsv                 -> untracked (generated, gitignored since
#                                                          2026-10-14; an old branch's edit loses)
#   docs/HANDOFF.md, docs/handoff/build-queue.tsv,
#   docs/handoff/pc1-data.md (append-only)              -> tools/coord/union.py (both sides)
# Any other conflict stops the sync with the merge left in progress and the
# files listed. The merge commit is refused while a conflict marker is left in
# docs/ specs/ crates/ tools/ facts/ traces/.
#
# Exit 0 merged (or already up to date), 1 stopped (conflicts or markers;
# resolve, then `git commit`), 2 error (dirty tree, fetch failed, usage).
# Base branch: --base, else $COORD_BASE, else main.
# The regeneration command can be replaced for tests: $SYNC_SPEC_INDEX.

BASE=${COORD_BASE:-main}
FETCH=1
HERE=$(cd "$(dirname "$0")" && pwd)
while [ $# -gt 0 ]; do
  case $1 in
    --base) BASE=$2; shift ;;
    --no-fetch) FETCH=0 ;;
    -h|--help) sed -n '2,22p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "sync: unknown argument '$1' (try --help)" >&2; exit 2 ;;
  esac
  shift
done

ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || { echo "sync: not in a git repository" >&2; exit 2; }
cd "$ROOT" || exit 2
PY=python3; command -v python3 >/dev/null 2>&1 || PY=py
SPEC_INDEX=${SYNC_SPEC_INDEX:-$PY tools/spec_index.py}
UNION="$PY $HERE/union.py"
MARKER_DIRS="docs specs crates tools facts traces"

if [ -f .git/MERGE_HEAD ] || [ -f "$(git rev-parse --git-path MERGE_HEAD)" ]; then
  echo "sync: a merge is already in progress; finish it (git commit) or abort it (git merge --abort)" >&2; exit 2
fi
if ! git diff --quiet || ! git diff --cached --quiet; then
  echo "sync: uncommitted changes to tracked files; commit or stash them first" >&2; exit 2
fi
BRANCH=$(git symbolic-ref --short -q HEAD) || { echo "sync: detached HEAD" >&2; exit 2; }

if [ $FETCH = 1 ]; then
  n=0; d=2
  until git fetch -q origin "$BASE"; do
    n=$((n + 1)); [ $n -ge 4 ] && { echo "sync: fetch of $BASE failed" >&2; exit 2; }
    sleep $d; d=$((d * 2))
  done
fi
REF=origin/$BASE
git rev-parse -q --verify "$REF" >/dev/null || { echo "sync: no $REF" >&2; exit 2; }
if git merge-base --is-ancestor "$REF" HEAD; then
  echo "sync: $BRANCH already contains $REF ($(git rev-parse --short "$REF"))"; exit 0
fi

markers() { # conflict-marker lines in tracked files of the marker dirs (tools/conflict_markers.py rule)
  git grep -n -I -E '^(<<<<<<<|\|\|\|\|\|\|\||=======$|>>>>>>>)' -- $MARKER_DIRS 2>/dev/null
}

OUT=$(git -c merge.conflictstyle=merge merge --no-ff --no-commit "$REF" 2>&1); RC=$?
CONFLICTS=$(git diff --name-only --diff-filter=U)
if [ $RC != 0 ] && [ -z "$CONFLICTS" ]; then
  echo "sync: merge of $REF failed:" >&2; echo "$OUT" >&2; exit 2
fi
LEFT=""; REGEN_SPEC=0
for f in $CONFLICTS; do
  case $f in
    docs/handoff/provisional-index*.tsv)
      git rm -q --cached --ignore-unmatch "$f" ;;
    docs/HANDOFF.md|docs/handoff/build-queue.tsv|docs/handoff/pc1-data.md)
      if [ -f "$f" ] && $UNION "$f" >/dev/null; then git add "$f"; else LEFT="$LEFT $f"; fi ;;
    specs/*.md)
      if [ -f "$f" ] && $UNION --index "$f" >/dev/null; then git add "$f"; REGEN_SPEC=1; else LEFT="$LEFT $f"; fi ;;
    *) LEFT="$LEFT $f" ;;
  esac
done

if [ -n "$LEFT" ]; then
  echo "sync: STOPPED: merge of $REF into $BRANCH has conflicts that need a person:"
  for f in $LEFT; do echo "  $f"; done
  echo "Resolve them (the mechanical ones above are already staged), then: $SPEC_INDEX; git add -A; git commit"
  echo "Or abort: git merge --abort"
  exit 1
fi

# Regenerate after every union, from the merged inputs. Spec index: always
# run it when any spec changed, since clean merges can stale an index too.
if [ $REGEN_SPEC = 1 ] || ! git diff --cached --quiet HEAD -- specs; then
  sh -c "$SPEC_INDEX" >/dev/null || { echo "sync: STOPPED: $SPEC_INDEX failed" >&2; exit 1; }
  git add -u specs
fi

M=$(markers)
if [ -n "$M" ]; then
  echo "sync: STOPPED: conflict markers left (merge not committed):"
  echo "$M" | head -n 40
  exit 1
fi
git commit -q --no-edit -m "Merge $BASE into $BRANCH (tools/coord/sync.sh)" || { echo "sync: commit failed" >&2; exit 1; }
echo "sync: merged $REF ($(git rev-parse --short "$REF")) into $BRANCH${CONFLICTS:+; auto-resolved: $(echo $CONFLICTS)}"
exit 0
