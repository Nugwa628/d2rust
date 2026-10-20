# The handshake: a bot that matches the server is let in, pinned to the
# certificate the server presents; a bot that expects another certificate
# never talks to it.
name   connect-handshake
about  Bots connect with the server's own hello values and its pinned certificate; a wrong pin fails.
rule   HELLO  a client with the server's protocol, build and tick rate is welcomed with a session token (contracts 3.4)
rule   PIN    a client pinned to another certificate does not connect (contracts 8)
bots   3

b0 connect                    expect ok tick>=1 session=yes     rule HELLO
b1 connect trust=pinned       expect ok session=yes             rule HELLO
b2 connect trust=wrong        expect error                      rule PIN
wait 1s
b0 stats                      expect connected=yes              within 3s   rule HELLO
