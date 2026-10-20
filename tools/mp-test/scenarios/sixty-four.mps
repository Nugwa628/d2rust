# The top player count: 64 players in one game, each seeing all of them.
name   sixty-four
about  64 bots enter one game raised to 64 players; every bot sees all 64 and their views agree.
rule   SIXTYFOUR  a game raised to 64 players takes 64 players, and every one of them sees all 64 in the same places (owner: 64 is a config change; M1d)
bots   64
server players=64

all connect
all enter big                      within 30s
all see                            expect players=64               within 10s
all agree                          expect agree=yes                within 10s
