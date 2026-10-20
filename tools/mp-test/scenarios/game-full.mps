# The player limit: a game at its limit refuses the next player; a leave
# makes room again.
name   game-full
about  Two bots fill a game of limit 2; the third gets no player; after a leave it gets in.
rule   LIMIT  a game at its player limit gives the next player no place (M1d; contracts 3.8: GameFull)
rule   ROOM   a player that leaves frees its place for the next one (M1d)
bots   3
server players=2

all  connect
b0-1 enter game1                   expect ok                       rule LIMIT
b2   enter game1                   expect refused=no-player        within 2s   rule LIMIT
b0   see                           expect players=2                rule LIMIT
b2   leave
b1   leave
b2   enter game1                   expect ok                       rule ROOM
b0   see                           expect players=2 b2=yes b1=no   within 3s   rule ROOM
