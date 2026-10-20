# Two players in one game see each other; a leave is seen by the other.
name   join-leave
about  Two bots enter one game, each sees the other, one leaves and the other sees it gone.
rule   OWN    every player that enters a game gets its own player (M1b)
rule   SEE    every player in a game is told about every other player in it (M1b)
rule   LEAVE  a player that leaves is gone for the players who stay (M1b)
bots   2

all connect
all enter game1                    expect ok                       rule OWN
b0  see                            expect players=2 b1=yes         within 3s   rule SEE
b1  see                            expect players=2 b0=yes         within 3s   rule SEE
b1  leave
b0  see                            expect players=1 b1=no          within 3s   rule LEAVE
b0  stats                          expect connected=yes            rule LEAVE
