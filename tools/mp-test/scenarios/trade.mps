# Play together: a trade moves an item from one player to the other, once.
# The test server (dev = test) gives b0's character the item: a cap, which no
# class starts with.
name   trade
about  b0 trades a cap to b1; after both confirm, b1 has it and b0 doesn't; nothing is copied or lost.
rule   TRADE  a confirmed trade moves each offered item to the other player exactly once: no copy, no loss (multiplayer.md: authoritative server)
bots   2
server dev=test

all connect
b0  register nora nora-pass-1             expect ok
b1  register owen owen-pass-1             expect ok
b0  char-create Norapal 3                 expect ok
b1  char-create Owensorc 1                expect ok
b0  char-select Norapal                   expect ok
b1  char-select Owensorc                  expect ok
b0  create-game tradegame                 expect ok
b1  join-game tradegame                   expect ok
server dev Norapal item cap
b0  inventory                             expect has.cap=yes   within 3s
b0  trade b1                              expect ok
b1  trade-accept                          expect ok
b0  trade-offer cap                       expect ok
all trade-confirm                         expect ok
b1  inventory                             expect has.cap=yes   within 3s
b0  inventory                             expect has.cap=no    within 3s
