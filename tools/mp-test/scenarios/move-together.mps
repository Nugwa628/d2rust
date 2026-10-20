# Moving: where one player walks, the others see it walk.
name   move-together
about  b0 walks; b1 and b2 see b0 walk to the new place; all views agree once nobody moves.
rule   MOVE   a player's walk is seen by every other player in the game, at the place it walks to (multiplayer.md: authoritative server)
rule   AGREE  once nobody moves, every player sees every player at the same place (Q2 desync rule)
bots   3

all connect
all enter town
b0  move +3 +0
b1  see                            expect b0.moved=yes             within 3s   rule MOVE
b2  see                            expect b0.moved=yes             within 3s   rule MOVE
wait 2s
all agree                          expect agree=yes                within 5s   rule AGREE
