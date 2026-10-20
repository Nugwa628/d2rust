# Play together: an item on the ground is seen by both players, and once one
# picks it up it is gone for the other. The test server (dev = test) gives
# b0's character the item: a cap, which no class starts with.
name   items-drop-pickup
about  b0 drops an item; b1 sees it on the ground; b1 picks it up; b0 no longer sees it and b1 has it.
rule   GROUND  an item dropped on the ground is seen by every player near it (multiplayer.md: authoritative server)
rule   PICKUP  an item picked up is gone from the ground for every player, and only the one who took it has it (multiplayer.md)
bots   2
server dev=test

all connect
b0  register jack jack-pass-1             expect ok
b1  register kim  kim-pass-1              expect ok
b0  char-create Jackpal 3                 expect ok
b1  char-create Kimsorc 1                 expect ok
b0  char-select Jackpal                   expect ok
b1  char-select Kimsorc                   expect ok
b0  create-game loot                      expect ok
b1  join-game loot                        expect ok
b1  see                                   expect b0=yes   within 5s   rule GROUND
server dev Jackpal item cap
b0  inventory                             expect has.cap=yes  within 3s   rule GROUND
b0  drop cap                              expect ok                    rule GROUND
b1  ground                                expect has.cap=yes  within 3s   rule GROUND
b1  pickup cap                            expect ok                    rule PICKUP
b0  ground                                expect has.cap=no   within 3s   rule PICKUP
b1  inventory                             expect has.cap=yes  within 3s   rule PICKUP
b0  inventory                             expect has.cap=no            rule PICKUP
