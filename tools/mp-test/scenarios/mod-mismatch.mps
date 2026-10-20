# A mod is its own game: a client of another mod (or of none) is refused.
name   mod-mismatch
about  A client running another mod than the server is refused, and told it is the game (contracts 3.4: WrongGame; owner: each mod is its own game).
rule   MOD   a client running another mod (or the base game against a mod's server) is refused, and told it is the game (contracts 3.4: WrongGame; owner: each mod is its own game)
bots   2
needs  mods

b0 connect                         expect ok
b1 connect game=some-other-mod     expect refused=WrongGame
