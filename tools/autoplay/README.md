# tools/autoplay: real-input smoke routes

`d2-client autoplay-host` runs the `play` client headless and takes mouse
and key events on stdin (line protocol `autoplay-1`), put where the
window's input goes. A route in `routes/` is a fixed list of those events
with state checks (`check level N`, `check sent 2F >= N`). The route
keeps its seed and milestones; UI and unit targets are resolved against
the current client model.

A 60 Hz route starts with `autoplay-route 1 tick_rate=60`. A missing rate
or a 25 Hz rate is refused with a re-recording message. Re-record the
inputs and checks at 60 Hz; changing an old header alone does not migrate
the route. The historical routes below remain refused until replaced.

The routes start on the main menu (`host --front`): they click through
the front end by UI element id (`click ui:menu.single_player`,
`click ui:char_select.char(AutoTown2)`, `text AutoProbe` for a name), so a
layout or resolution change does not break them. The `ui` command lists
the ids on screen with their rectangles; an unknown id fails with that
list. The id table is in `crates/d2-client/src/ui/front_end/screens/element_ids.rs`.
In game the open panels list theirs the same way (`click ui:npc.menu(0)`,
`npc.topic(N)`, `dialog.skip`, `waypoint.tab(N)`, `waypoint.row(N)`,
`waypoint.close`, the Esc menu's `options.<page>.<row>`); a panel lists
them in its `Panel::ui_elements`, in the 800-wide UI box, and the host
moves them to the frame, so in-game ids need no x shift either.
NPCs and objects can be clicked with T2's `clickunit T C[,C..]|* [DX DY]`
(or `rclickunit` for the right button). The host uses T2's parser and
unit selector, moves the pointer to the unit's current screen position,
lets hover settle for one pass, then queues the press and release. Missing
or off-screen units fail explicitly. These steps are not shifted by
`--res`; their target is resolved in the active frame. `check client`
accepts dotted fields, for example `check client npc_menu.class == 154`,
so a talk milestone can verify which NPC is active.

`interactunit T C[,C..]|*` approaches the nearest matching unit and
requests the normal client interaction. It reads the host's unit snapshot
and collision map to choose short walking waypoints, including targets
outside the client's streamed rooms. Movement uses ordinary pointer
input; the command never mutates server state. It stops after at most
64 approach clicks and fails explicitly if no target or path exists.
NPC approaches leave space for the NPC's footprint and use the normal
client interact sender, with the server enforcing talk range. Object
approaches queue the client's pending interaction.
Follow it with `step N` and a state check: an interaction request does
not promise the server will accept it. An already open menu for that
NPC is kept. A repeated interaction advances an active NPC greeting with
Space (at most 64 lines), preserving the menu after its last line.

The act probes use this step for NPC approach and waypoint objects, and
UI ids for Talk, topic, dialog skip and waypoint rows. Waypoints follow
the pending approach with `useunit T C[,C..]|*`, which requests the normal
client interact sender immediately. The server checks range. This handles
objects whose pending reach check still reads an older model position. Units without a
Talk entry in `tables/ui/npc-menus.tsv` are visits, not dialog claims.
The existing navigation walks stream town rooms and spawn their NPCs.
Map-guided unit targeting repairs the approach independently of those
walks' endpoints, including paths around obstacles. `walklevel N` uses
the same read-only map and ordinary walking input to enter a connected
level (at most 64 short walks); Act I uses it to leave town for Blood Moor. A second interaction handles a first-visit greeting.
After a verified talk, Clear Screen closes the talk panels before walking.
Each talk verifies `npc_menu.class` and `npc_menu.talking`; the waypoint
milestone verifies UI state 20 is open. Cumulative message minima count
these verified talks, rather than incidental clicks during navigation.

With `--front` the route's `save` character is written into the host's
`--save-dir` folder (the character select lists it) instead of being
passed as `--save`.

```sh
cargo build --release -p d2-client -p d2s-tool
export D2_GAME_DIR=/path/to/game-files     # the game data repo's install
python3 tools/autoplay/replay.py --all      # ~1-3 min per route
python3 tools/autoplay/replay.py tools/autoplay/routes/act2-probe.route
python3 tools/autoplay/replay.py --selftest # no game files
```

| Route | Character | Coverage |
|---|---|---|
| `act1-probe` | fresh Amazon, seed 1234 | 5 town NPC talks, town waypoint menu, walk into Blood Moor |
| `act2-probe` | level-30 sorceress in Lut Gholein | 11 NPC talks, 5 unit visits, waypoint menu, waypoint out and back |
| `act3-probe` | same, Kurast Docks | 6 NPC talks, 2 unit visits, waypoint menu, waypoint out and back |
| `act4-probe` | same, Pandemonium Fortress | 2 NPC talks, waypoint menu, waypoint out and back |
| `act5-probe` | same, Harrogath | 5 NPC talks, 3 unit visits, waypoint menu, waypoint out and back |

A failing check prints the line, the milestone and why; the host's stderr
is in the work dir. When the host still answers, `<route>.failure.json`
captures the failed step, state, visible UI ids and collision map.
Logs default to `<work>/<route-name>/logs/`: `game.log`, `game.prev.log`
and, on a panic, `crash.log`. Set `D2RUST_LOG_DIR` to choose another log
directory. Replay also defaults `D2RUST_HOME` to `<work>/user/` to keep
startup migration, settings and saves away from the player's Documents
folder; an explicit override wins.
See [route log options](../../docs/tools/scenarios.md#route-runs-and-logs).


When a route goes stale, inspect `state` and `ui` around the failing
milestone. Use `click ui:ID` for menu rows and `clickunit` for NPCs and
objects, preserving the intended action and milestone. Keep logs and
settings local by setting `D2RUST_HOME` and `D2RUST_LOG_DIR` to the run
directory before starting the host.
