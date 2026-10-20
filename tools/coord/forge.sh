#!/bin/sh
# Forgejo access for sessions and tools (git.werdegames.com, org WerdeGames).
#
#   . tools/coord/forge.sh                  sourced: git over https to the forge
#                                           authenticates with $FORGEJO_TOKEN
#                                           (no token written to any file), and
#                                           forge_api is defined
#   sh tools/coord/forge.sh api GET <path>  one API call, JSON on stdout
#                                           (<path> after /api/v1, e.g. repos/$FORGE_REPO/pulls)
#   sh tools/coord/forge.sh automerge <n>   M27: merge PR <n> by itself once its
#                                           required checks pass (merge commit,
#                                           pinned to the current head)
#   sh tools/coord/forge.sh origin          point origin at the forge (a session
#                                           created from a GitHub copy works on
#                                           the forge from then on)
#
# Messages between sessions (local, BIGRIG, cloud): open issues labelled
# `coord` on the main repo; every post is tagged with the sender
# ($FORGE_FROM, default the host name), since all sessions share one token.
#   sh tools/coord/forge.sh inbox                     open threads, newest post first
#   sh tools/coord/forge.sh read <n> [<after-id>]     a thread (only posts after id)
#   sh tools/coord/forge.sh say <n> <text|->          post (- reads stdin)
#   sh tools/coord/forge.sh new <title> <text|->      open a thread, prints its number
#   sh tools/coord/forge.sh watch [<n>...] [-i <sec>] print each new post from another sender
#                                                     as it lands (default every 1 s, one API
#                                                     call per poll for all threads; no <n>:
#                                                     every coord thread, new ones included;
#                                                     silent otherwise: run it as a monitor)
#
# FORGEJO_TOKEN is the cloud environment's secret (local: the same token).
FORGE_HOST=${FORGE_HOST:-git.werdegames.com}
FORGE_REPO=${FORGE_REPO:-WerdeGames/D2RUST}
FORGE_URL=https://$FORGE_HOST
FORGE_FROM=${FORGE_FROM:-$(hostname)}
export FORGE_HOST FORGE_REPO FORGE_URL FORGE_FROM
# python3 on Linux; on Windows python3 (and sometimes python) is a store stub,
# so fall back to python, then the py launcher
PY=python3; $PY -c '' 2>/dev/null || PY=python
$PY -c '' 2>/dev/null || PY='py -3'

if [ -n "${FORGEJO_TOKEN:-}" ]; then
  # git reads extra config from the environment: the header goes only to the forge.
  n=${GIT_CONFIG_COUNT:-0}
  export "GIT_CONFIG_KEY_$n=http.$FORGE_URL/.extraHeader"
  export "GIT_CONFIG_VALUE_$n=Authorization: token $FORGEJO_TOKEN"
  export GIT_CONFIG_COUNT=$((n + 1))
fi

forge_api() { # METHOD PATH [JSON-BODY]
  [ -n "${FORGEJO_TOKEN:-}" ] || { echo "forge: FORGEJO_TOKEN is not set" >&2; return 2; }
  curl -sSf -X "$1" -H "Authorization: token $FORGEJO_TOKEN" -H 'Content-Type: application/json' \
    ${3:+--data "$3"} "$FORGE_URL/api/v1/$2"
}

case "$0" in
  *forge.sh)
    case "${1:-}" in
      api) shift; forge_api "$@" ;;
      automerge)
        [ -n "${2:-}" ] || { echo "usage: forge.sh automerge <pr>" >&2; exit 2; }
        sha=$(forge_api GET "repos/$FORGE_REPO/pulls/$2" | $PY -c 'import json,sys; print(json.load(sys.stdin)["head"]["sha"])') || exit 1
        forge_api POST "repos/$FORGE_REPO/pulls/$2/merge" \
          "{\"Do\":\"merge\",\"merge_when_checks_succeed\":true,\"head_commit_id\":\"$sha\"}" >/dev/null \
          && echo "#$2 auto-merge on at $sha" ;;
      origin)
        git remote set-url origin "$FORGE_URL/$FORGE_REPO.git" && git fetch -q origin \
          && echo "origin -> $FORGE_URL/$FORGE_REPO.git" ;;
      inbox)
        forge_api GET "repos/$FORGE_REPO/issues?state=open&labels=coord&type=issues&sort=recentupdate&limit=50" \n          | $PY -c 'import json,sys
for i in json.load(sys.stdin):
    print("#%d  %s  (%d posts, updated %s)" % (i["number"], i["title"], i["comments"] + 1, i["updated_at"][:16]))' ;;
      read)
        [ -n "${2:-}" ] || { echo "usage: forge.sh read <n> [<after-id>]" >&2; exit 2; }
        { forge_api GET "repos/$FORGE_REPO/issues/$2"; echo; forge_api GET "repos/$FORGE_REPO/issues/$2/comments"; } \
          | $PY -c 'import json,sys
issue, comments = [json.loads(l) for l in sys.stdin.read().splitlines() if l.strip()]
after = int(sys.argv[1] or 0)
if not after:
    print("#%d %s" % (issue["number"], issue["title"]))
    print("--- 0 (opened %s)" % issue["created_at"][:16])
    print(issue["body"])
for c in comments:
    if c["id"] > after:
        print("--- %d (%s)" % (c["id"], c["created_at"][:16]))
        print(c["body"])' "${3:-}" ;;
      say|new)
        cmd=$1; shift
        [ "$cmd" = say ] && { [ -n "${1:-}" ] || { echo "usage: forge.sh say <n> <text|->" >&2; exit 2; }; n=$1; shift; }
        [ "$cmd" = new ] && { [ -n "${1:-}" ] || { echo "usage: forge.sh new <title> <text|->" >&2; exit 2; }; title=$1; shift; }
        if [ "${1:--}" = - ]; then text=$(cat); else text=$*; fi
        # the host tag lets watchers spot two sessions signing one name (forge_watch.py)
        body=$(printf '**[%s]** %s\n\n<!-- forge-host: %s -->' "$FORGE_FROM" "$text" "$(hostname)" \
          | $PY -c 'import json,sys; print(json.dumps(sys.stdin.read()))')
        if [ "$cmd" = say ]; then
          forge_api POST "repos/$FORGE_REPO/issues/$n/comments" "{\"body\":$body}" \n          | $PY -c 'import json,sys; print("posted", json.load(sys.stdin)["id"])'
        else
          lid=$(forge_api GET "repos/$FORGE_REPO/labels?limit=100" | $PY -c 'import json,sys; print(next((l["id"] for l in json.load(sys.stdin) if l["name"]=="coord"), ""))')
          [ -n "$lid" ] || lid=$(forge_api POST "repos/$FORGE_REPO/labels" '{"name":"coord","color":"#1d76db","description":"messages between sessions (tools/coord/forge.sh)"}' \n          | $PY -c 'import json,sys; print(json.load(sys.stdin)["id"])')
          t=$(printf '%s' "$title" | $PY -c 'import json,sys; print(json.dumps(sys.stdin.read()))')
          forge_api POST "repos/$FORGE_REPO/issues" "{\"title\":$t,\"body\":$body,\"labels\":[$lid]}" \n          | $PY -c 'import json,sys; print(json.load(sys.stdin)["number"])'
        fi ;;
      watch)
        shift; [ -n "${FORGEJO_TOKEN:-}" ] || { echo "forge: FORGEJO_TOKEN is not set" >&2; exit 2; }
        exec $PY -u "$(dirname "$0")/forge_watch.py" "$@" ;;
      *) sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
    esac ;;
esac
