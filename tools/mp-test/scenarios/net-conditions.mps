# Bad networks: latency, jitter and loss slow play down but don't break it.
name   net-conditions
about  Four bots on 150 ms latency, 40 ms jitter and 5% loss each way connect, enter one game, see each other and agree; nobody drops.
rule   SLOW   with 150 ms latency, 40 ms jitter and 5% loss each way, players still connect, enter, see each other and agree, and no connection drops (multiplayer.md: QUIC)
bots   4
net    latency=150 jitter=40 loss=5
seed   7

all connect                        within 20s
all enter lag                      within 20s
all see                            expect players=4                within 10s
all agree                          expect agree=yes                within 10s
wait 3s
all stats                          expect connected=yes bad=0
