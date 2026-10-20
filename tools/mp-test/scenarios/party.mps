# Play together: a party forms by invite and accept, and both see it.
name   party
about  b0 invites b1; b1 accepts; both see a party of two; b2, not invited, isn't in it.
rule   PARTY  an accepted invite puts both players in one party, and every member sees the same members
bots   3

all connect
all enter partygame
b0  party-invite b1                       expect ok
b1  party-accept b0                       expect ok
b0  party                                 expect members=2 b1=yes   within 3s
b1  party                                 expect members=2 b0=yes   within 3s
b2  party                                 expect b0=no b1=no
