# Re-entry: a player that comes back sees the game as it is now, the same
# as a player that never dropped (contracts 3.7). While b1 is away, b2
# joins the game; after the resume b1 must know about b2 exactly as b0,
# which never dropped, does.
name   reconnect-view
about  b1 drops mid-game, b2 joins while it is away, and b1's resumed view matches b0's, which never dropped.
rule   RESUME  a player that comes back within the grace period gets its own slot back (same client id, same player) (contracts 3.7)
rule   VIEW    after the resume, the player holds the same view of the game as a player that never dropped: the same players at the same places (contracts 3.7)
bots   3
server grace=20000

all connect
b0-1 enter game1                   expect ok                       rule RESUME
b0  see                            expect players=2                within 3s   rule RESUME
net b1 cut=on
b1  stats                          expect closed                   within 10s  rule RESUME
b2  enter game1                    expect ok                       within 5s
b0  see                            expect players=3 b2=yes         within 3s   rule VIEW
net b1 cut=off
b1  reconnect                      expect ok same=yes              rule RESUME
b1  see                            expect players=3 b0=yes b2=yes  within 5s   rule VIEW
all agree                          expect agree=yes                within 5s   rule VIEW
b1  stats                          expect connected=yes bad=0      within 3s   rule VIEW
