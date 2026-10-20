# The game list (owner 2026-10-19: name, difficulty, players, password
# lock) follows games as they are made, joined, left and closed.
name   lobby-game-list
about  The list shows a new game with its difficulty, player count and lock; counts follow joins and leaves; an empty game leaves the list.
rule   LIST    the game list shows every open game with its name, difficulty, player count and whether it has a password (contracts 3.6)
rule   COUNT   a game's player count in the list follows joins and leaves (contracts 3.6, 6)
rule   CLOSE   a game with no players and no held slots closes and leaves the list (GameGone) (contracts 6)
bots   3
server grace=0

all connect
b0  register lena lena-pass-1             expect ok
b1  register mo   mo-pass-1               expect ok
b2  register nia  nia-pass-1              expect ok
b0  char-create Lenachar 0                expect ok
b1  char-create Mochar 1                  expect ok
b2  char-create Niachar 2                 expect ok
b0  char-select Lenachar                  expect ok
b1  char-select Mochar                    expect ok
b2  char-select Niachar                   expect ok
b0  create-game listed password=pw difficulty=normal   expect ok
b2  games                                 expect has.listed=yes listed.players=1 listed.difficulty=normal listed.locked=yes   within 3s   rule LIST
b1  join-game listed password=pw          expect ok
b2  games                                 expect listed.players=2      within 3s   rule COUNT
b1  leave-game
b2  games                                 expect listed.players=1      within 3s   rule COUNT
b0  leave-game
b2  games                                 expect has.listed=no         within 5s   rule CLOSE
