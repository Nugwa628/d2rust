# Disconnect and reconnect: a player whose link drops comes back to its
# own place within the grace period.
name   reconnect
about  b1's link is cut until its connection times out; it reconnects with its token within the grace and is still in the game.
rule   PARK    a player whose connection is lost stays in the game during the grace period (contracts 6: grace_s)
rule   RESUME  a player that comes back within the grace period gets its own slot back (same client id, same player) (contracts 3.7)
bots   2
server grace=20000

all connect
all enter game1
b0  see                            expect players=2                within 3s   rule PARK
net b1 cut=on
b1  stats                          expect closed                   within 10s  rule PARK
b0  see                            expect players=2 b1=yes         rule PARK
net b1 cut=off
b1  reconnect                      expect ok same=yes              rule RESUME
b1  stats                          expect connected=yes            within 3s   rule RESUME
b0  see                            expect players=2 b1=yes         within 3s   rule RESUME
