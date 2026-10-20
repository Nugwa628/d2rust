# After the grace period a lost player is gone, and its token is refused.
name   reconnect-expired
about  b1's link stays cut past a 2 s grace; it leaves the game and its old token is refused; a fresh connect works.
rule   EXPIRE  a lost player who doesn't come back within the grace period leaves the game, and its old token is refused (UnknownResume) (contracts 3.4, 3.8, 6: grace)
bots   2
server grace=2000

all connect
all enter game1
b0  see                            expect players=2                within 3s   rule EXPIRE
net b1 cut=on
b1  stats                          expect closed                   within 10s  rule EXPIRE
b0  see                            expect players=1 b1=no          within 10s  rule EXPIRE
net b1 cut=off
b1  reconnect                      expect refused=UnknownResume   rule EXPIRE
b1  connect                        expect ok                       rule EXPIRE
