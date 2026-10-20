"""Watch coord threads on the forge and print each new post from another
sender as it lands (run through `sh tools/coord/forge.sh watch`).

One long-running process; one API call per poll for any number of threads
(the repo-wide comment feed, `since` a few seconds back, deduplicated by id).
With no thread numbers it watches every `coord` thread, and a new thread
opened by another sender is printed too (checked every 10th poll).
Prints nothing while nothing happens, so it can run as a background monitor.

A restart catches up: the last seen post id and time are kept in
$FORGE_WATCH_STATE (default ~/.cache/forge-watch/<repo>-<from>.json), and on
start every post after them is printed first, so a gap between two watches
loses nothing.

A post signed with our own FORGE_FROM is skipped as ours, unless its
host tag (added by `forge.sh say/new`) names another host: then two sessions
share one name and would miss each other's posts, so it is printed with a
warning (escape 2026-10-15-two-sessions-one-sender).

Env: FORGEJO_TOKEN, FORGE_URL, FORGE_REPO, FORGE_FROM (set by forge.sh).
`--selftest` needs none of them.
"""
import datetime
import json
import os
import re
import socket
import sys
import time
import urllib.error
import urllib.request

API = os.environ.get("FORGE_URL", "") + "/api/v1/repos/" + os.environ.get("FORGE_REPO", "")
TOKEN = os.environ.get("FORGEJO_TOKEN", "")
ME = "**[%s]**" % os.environ.get("FORGE_FROM", "")
HOST = socket.gethostname()
HOST_TAG = re.compile(r"<!-- forge-host: (\S+) -->")
LOOKBACK = datetime.timedelta(seconds=30)  # covers clock skew and slow polls
STATE = os.environ.get("FORGE_WATCH_STATE") or os.path.join(
    os.path.expanduser("~"), ".cache", "forge-watch",
    "%s-%s.json" % (os.environ.get("FORGE_REPO", "").replace("/", "_"), os.environ.get("FORGE_FROM", "")))


def get(path):
    req = urllib.request.Request(API + path, headers={"Authorization": "token " + TOKEN})
    with urllib.request.urlopen(req, timeout=20) as r:
        return json.load(r)


def iso(t):
    return t.strftime("%Y-%m-%dT%H:%M:%SZ")


def number(c):
    """The issue or PR number a comment belongs to (PR comments have an empty
    issue_url and carry pull_request_url instead); None when neither does."""
    for key in ("issue_url", "pull_request_url"):
        tail = (c.get(key) or "").rstrip("/").rsplit("/", 1)[-1]
        if tail.isdigit():
            return int(tail)
    return None


def sender(body, me=None, host=None):
    """"other", "me", or the other host's name when a post carries our name
    but another host's tag (posts without a tag count as ours)."""
    me, host = me or ME, host or HOST
    if not body.startswith(me):
        return "other"
    m = HOST_TAG.search(body)
    if m and m.group(1).lower() != host.lower():
        return m.group(1)
    return "me"


def selftest():
    me, tag = "**[LC]**", "\n\n<!-- forge-host: %s -->"
    assert sender("**[MC]** hi" + tag % "BIGRIG", me, "BIGRIG") == "other"
    assert sender("**[LC]** hi", me, "BIGRIG") == "me"  # untagged (older posts)
    assert sender("**[LC]** hi" + tag % "bigrig", me, "BIGRIG") == "me"
    assert sender("**[LC]** hi" + tag % "LAPTOP", me, "BIGRIG") == "LAPTOP"
    assert sender("**[LC-laptop]** hi" + tag % "LAPTOP", me, "BIGRIG") == "other"
    print("forge_watch selftest: ok")


def emit(n, kind, item, body):
    who = sender(body)
    if who == "me":
        return
    if who != "other":
        kind += " WARNING: host %s also signs %s; give one session another FORGE_FROM" % (who, ME)
    print("=== #%s %s %d (%s)\n%s" % (n, kind, item["id"], item["created_at"][:16], body), flush=True)


def main(argv):
    if argv == ["--selftest"]:
        return selftest()
    interval, threads, i = 1.0, set(), 0
    while i < len(argv):
        if argv[i] == "-i":
            interval = float(argv[i + 1])
            i += 2
        else:
            threads.add(int(argv[i]))
            i += 1
    coord = {}  # issue number -> is a coord thread (only when watching all)

    def wanted(n):
        if threads:
            return n in threads
        if n not in coord:
            coord[n] = any(l["name"] == "coord" for l in get("/issues/%d" % n)["labels"])
        return coord[n]

    start = datetime.datetime.now(datetime.timezone.utc)
    last_id, last_at = 0, None
    try:
        with open(STATE, encoding="utf-8") as f:
            st = json.load(f)
        last_id, last_at = st["id"], datetime.datetime.fromisoformat(st["at"])
    except (OSError, ValueError, KeyError):
        pass
    seen = set()
    if last_at is not None:  # catch up on what landed since the last watch
        for c in sorted(get("/issues/comments?limit=50&since=" + iso(last_at - LOOKBACK)), key=lambda c: c["id"]):
            seen.add(c["id"])
            n = number(c)
            if c["id"] > last_id and n is not None and wanted(n):
                emit(n, "post", c, c["body"])
    seen |= {c["id"] for c in get("/issues/comments?limit=50&since=" + iso(start - LOOKBACK))}

    def save():
        os.makedirs(os.path.dirname(STATE), exist_ok=True)
        with open(STATE, "w", encoding="utf-8") as f:
            json.dump({"id": max(seen | {last_id}), "at": now.isoformat()}, f)
    seen_issues = {x["number"] for x in get("/issues?state=open&labels=coord&type=issues&limit=50")}
    polls, errors = 0, 0
    while True:
        time.sleep(interval)
        polls += 1
        now = datetime.datetime.now(datetime.timezone.utc)
        try:
            for c in get("/issues/comments?limit=50&since=" + iso(now - LOOKBACK)):
                if c["id"] in seen:
                    continue
                seen.add(c["id"])
                n = number(c)
                if n is not None and wanted(n):
                    emit(n, "post", c, c["body"])
            if not threads and polls % 10 == 0:
                for x in get("/issues?state=open&labels=coord&type=issues&limit=50"):
                    if x["number"] not in seen_issues:
                        seen_issues.add(x["number"])
                        coord[x["number"]] = True
                        emit(x["number"], "new thread: " + x["title"] + " |", x, x["body"])
            errors = 0
            save()
        except (urllib.error.URLError, OSError, ValueError, KeyError, IndexError, TypeError) as e:
            errors += 1
            if errors in (5, 60) or errors % 600 == 0:  # say so once it is not a blip
                print("forge watch: %d failed polls in a row (%s)" % (errors, e), flush=True)
        if len(seen) > 5000:
            seen = set(sorted(seen)[-1000:])


if __name__ == "__main__":
    try:
        main(sys.argv[1:])
    except KeyboardInterrupt:
        pass
