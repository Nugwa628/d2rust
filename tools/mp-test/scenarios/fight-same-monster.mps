# Play together: two players fight the same monster and both see it die.
# The test server (dev = test) sets the scene with dev commands: both
# characters can't die (god), both go out of town to the Blood Moor (no
# fighting in town), and a monster is placed next to b0's character.
name   fight-same-monster
about  A monster is placed next to b0 outside town; b0 and b1 attack it; both see it, and both see it die.
rule   SHARED  a monster two players fight is one monster: both see it, and both see it die (multiplayer.md: authoritative server)
bots   2
server dev=test

all connect
b0  register hank hank-pass-1             expect ok
b1  register ivy  ivy-pass-1              expect ok
b0  char-create Hankbar 4                 expect ok
b1  char-create Ivyama 0                  expect ok
b0  char-select Hankbar                   expect ok
b1  char-select Ivyama                    expect ok
b0  create-game arena                     expect ok
b1  join-game arena                       expect ok
server dev Hankbar god
server dev Ivyama god
server dev Hankbar warp 2
server dev Ivyama warp 2
b0  area                                  expect area=blood-moor  within 5s
b1  see                                   expect b0=yes           within 5s
server dev Hankbar spawn fallen
all monster fallen                        expect seen=yes   within 3s    rule SHARED
all attack fallen                         expect ok attacks>=1  within 20s   rule SHARED
b0  monster fallen                        expect dead=yes   within 3s    rule SHARED
b1  monster fallen                        expect dead=yes   within 3s    rule SHARED
