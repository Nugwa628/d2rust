# Lobby chat (owner: "chat if cheap"). When the lobby doesn't build it,
# the server answers NotSupported and this scenario shows that.
name   chat
about  A line one player says in the lobby is heard by another with the speaker's character name; too many lines are rate-limited.
rule   HEAR    a chat line reaches every session in the lobby with the speaker's character name (contracts 3.6)
rule   LIMIT   more than 5 chat lines in 10 s are RateLimited (contracts 5)
bots   2

all connect
b0  register olga olga-pass-1             expect ok
b1  register pat  pat-pass-1              expect ok
b0  char-create Olgachar 0                expect ok
b1  char-create Patchar 1                 expect ok
b0  char-select Olgachar                  expect ok
b1  char-select Patchar                   expect ok
b0  chat hello-everyone                   expect ok                                    rule HEAR
b1  heard                                 expect last=hello-everyone from=Olgachar   within 3s   rule HEAR
b0  chat two                              expect ok                                    rule LIMIT
b0  chat three                            expect ok                                    rule LIMIT
b0  chat four                             expect ok                                    rule LIMIT
b0  chat five                             expect ok                                    rule LIMIT
b0  chat six                              expect refused=RateLimited                   rule LIMIT
