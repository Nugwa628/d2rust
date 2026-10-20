# A crashed server: every client notices, and a restarted server takes new
# connections (tokens from before are refused).
name   server-crash
about  The server is killed mid-game; every bot sees its connection end; after a start they connect again.
rule   NOTICE  when the server dies, every client's connection ends (no client hangs) (multiplayer.md: QUIC timeouts)
rule   AGAIN   a restarted server takes connections again; tokens from before the restart are refused (contracts 4: sessions live in memory)
bots   2

all connect
all enter game1
server kill
all stats                          expect closed                   within 10s  rule NOTICE
server start
b0  reconnect                      expect refused=UnknownResume   rule AGAIN
all connect                        expect ok                       rule AGAIN
