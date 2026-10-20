# Server restart with persistence: accounts and characters survive a clean
# stop and a crash; sessions don't (clients log in again).
name   server-restart-persistence
about  An account and its character, after a game, survive a clean restart and a kill with their progress; old sessions are gone.
rule   KEEP     accounts and their characters survive a server restart, clean or crashed, with what the last save had (contracts 4, 6)
rule   SESSION  a restart ends every session: the old one is not taken back (the client returns to a fresh session) and logs in again (contracts 4)
bots   1
needs  persist

b0  connect
b0  register quinn quinn-pass-1           expect ok
b0  char-create Quinnsorc 1               expect ok
b0  char-select Quinnsorc                 expect ok
b0  create-game savegame                  expect ok
b0  leave-game
server restart
b0  reconnect                             expect ok same=no             rule SESSION
b0  login quinn quinn-pass-1              expect ok                      rule KEEP
b0  char-list                             expect has.Quinnsorc=yes       rule KEEP
server kill
b0  stats                                 expect closed      within 10s  rule SESSION
server start
b0  connect
b0  login quinn quinn-pass-1              expect ok                      rule KEEP
b0  char-list                             expect has.Quinnsorc=yes       rule KEEP
