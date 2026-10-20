D2RUST playtest build (Windows)
=============================

Build of commit @COMMIT@ (branch @REF@).
A private development build of a Diablo II: Lord of Destruction
reimplementation. It contains no game files: it reads your own
Diablo II 1.14d install.

Start
-----
1. Copy D2RUST-Game.exe into your Diablo II folder,
   the one holding Game.exe and d2data.mpq.
2. Double-click D2RUST-Game.exe. The main menu opens.

Or keep it anywhere and point it at the game:
   D2RUST-Game.exe --game-dir "C:\Program Files (x86)\Diablo II"
(or set the D2_GAME_DIR environment variable to that folder).

Shortcuts straight into a game (from a command prompt):
   D2RUST-Game.exe --new sorceress MyName     new character
   D2RUST-Game.exe --save "path\to\MyName.d2s" load a save
   D2RUST-Game.exe --difficulty nightmare ...  normal / nightmare / hell
   D2RUST-Game.exe --res 640x480               the 640 x 480 frame

Characters are saved in Documents\D2RUST\saves (never inside the game
folder); an old Documents\d2rs folder is moved there on the first start.
Key bindings: Documents\D2RUST\controls.toml (the original game's
default keys).

Dev tools (not part of Diablo II; on by default in this alpha)
-------------------------------------------------------------
Press ` (backquote, left of 1) to open or close the dev panel (Esc also
closes it): click its buttons (god, heal, speed, level, gold, points,
waypoints, acts, towns, warp list per act, monster/item picker). While
it is open the game gets no keys or clicks. Or type a command in the
console at the bottom left: Enter runs it; Up/Down history, Tab completes. Turn the tools off with
--no-dev or the environment variable D2RUST_DEV=0.

   help [command]            list commands / usage
   god [on|off]              no death, life/mana refilled every tick
   heal                      full life, mana, stamina
   level <1..99> | +<n>      raise to that level / n levels
   gold <n>                  add gold (capped at level x 10000)
   statpts <n>, skillpts <n> add stat / skill points
   town <1..5>               warp to an act's town
   warp <level id|name>      warp to a level (warp 35, warp cold plains)
   waypoints                 know every waypoint
   acts                      open acts 2-5 (then travel by waypoint)
   item <code|name> [quality] [ilvl <n>] [ground]
                             create an item (item hax magic ilvl 30)
   identify                  identify everything you carry
   kill [screen|level]       kill monsters near you / in the level
   spawn <monster> [count]   spawn monsters (spawn fallen 5)
   reveal                    (not built yet)
   speed <percent>           faster walk/run (speed 0: off)
   nocd [on|off]             no skill cooldown
   freemana [on|off]         mana refilled every tick
   difficulty <name>         how to restart on another difficulty
   info                      info overlay (area, position, tick, FPS)
   bugreport [note]          bug report folder (screenshot, log, state)
   save <name>, load [name]  save / load a slot (load restarts there)
   clear                     clear the console

When it crashes
---------------
A message box names the reason. The details are in
Documents\D2RUST\logs: crash.log (backtrace and what it was doing) and
game.log (everything it printed). Send both, a screenshot, and the save
(Documents\D2RUST\saves\<name>.d2s). Full guide: docs/PLAYTEST.md in the
repository.
