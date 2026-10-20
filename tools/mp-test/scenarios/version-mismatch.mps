# Version refusal: a client that doesn't match the server is told why and
# never gets in.
name   version-mismatch
about  Another game build, protocol or tick rate is refused with its reason; the matching client plays on.
rule   BUILD     a client of another game build is refused, and told it is the build (contracts 3.4: Build)
rule   PROTOCOL  a client of another protocol version is refused, and told it is the protocol (contracts 3.4: Protocol)
rule   TICK      a client built for another tick rate is refused, and told it is the tick rate (contracts 3.4: TickRate)
rule   STAY      refusing a client doesn't disturb the clients already connected (contracts 3.4)
bots   4

b0 connect                         expect ok                  rule STAY
b1 connect build=0.0.0-other       expect refused=Build       rule BUILD
b2 connect protocol=999            expect refused=Protocol    rule PROTOCOL
b3 connect tick=61                 expect refused=TickRate   rule TICK
wait 1s
b0 stats                           expect connected=yes       within 3s   rule STAY
