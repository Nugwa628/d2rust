# Moving on a bad network: a walk still reaches everyone, later.
name   move-under-lag
about  On 150 ms latency, 40 ms jitter and 5% loss, b0 walks; the others see it walk and all views agree once it stops.
rule   LAGMOVE  with 150 ms latency, 40 ms jitter and 5% loss each way, a walk reaches every other player and all views agree once it stops (multiplayer.md: QUIC)
bots   3
net    latency=150 jitter=40 loss=5
seed   11

all connect                        within 20s
all enter lagmove                  within 20s
b0  move +3 +0
b1  see                            expect b0.moved=yes             within 5s
b2  see                            expect b0.moved=yes             within 5s
wait 2s
all agree                          expect agree=yes                within 10s
