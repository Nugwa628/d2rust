# One session per account: a new login takes over, the old connection is
# told why and closed.
name   accounts-session-takeover
about  b0 is logged in; b1 logs in to the same account; b0 gets SessionTaken and its connection ends; logout ends a session.
rule   TAKEOVER  a new login to an account with a session takes over: the old connection gets SessionTaken and is closed (contracts 3.5)
rule   LOGOUT    logout ends the session: the server answers with BYE and the connection closes (contracts 3.5)
bots   2

all connect
b0  register dana dana-pass-1             expect ok
b1  login    dana dana-pass-1             expect ok                     rule TAKEOVER
b0  stats                                 expect closed      within 5s  rule TAKEOVER
b1  logout                                expect ok                     rule LOGOUT
b1  stats                                 expect closed      within 5s  rule LOGOUT
