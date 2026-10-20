# Difficulty comes from character progression (owner 2026-10-19): a
# character creates games at every difficulty it has unlocked, and joins
# only games of a difficulty it has unlocked. The mask comes from the
# stored character, never from the client. A test server sets progression
# on its stdin (`dev = test`, `set-progression`; contracts 10.1).
name   difficulty-rule
about  A new character creates only Normal games and can't join Nightmare; one with Nightmare unlocked creates and joins both.
rule   CREATE  a character can create a game at every difficulty it has unlocked and at no other (DifficultyLocked) (contracts 6, difficulty rule)
rule   JOIN    a character can join only games of a difficulty it has unlocked (DifficultyLocked) (contracts 6, difficulty rule)
bots   2
server dev=test

all connect
b0  register frank frank-pass-1           expect ok
b1  register gina  gina-pass-1            expect ok
b0  char-create Frankpal 3                expect ok
b1  char-create Ginaama 0                 expect ok
server progression Frankpal 5
b0  char-select Frankpal                  expect ok difficulties=3      rule CREATE
b1  char-select Ginaama                   expect ok difficulties=1      rule CREATE
b1  create-game hellrun difficulty=hell        expect refused=DifficultyLocked   rule CREATE
b1  create-game nmrun difficulty=nightmare     expect refused=DifficultyLocked   rule CREATE
b0  create-game nmgame difficulty=nightmare    expect ok                         rule CREATE
b1  games                                 expect has.nmgame=yes nmgame.joinable=66   within 3s   rule JOIN
b1  join-game nmgame                      expect refused=DifficultyLocked   rule JOIN
b1  create-game normrun difficulty=normal expect ok                         rule CREATE
b0  leave-game
b0  join-game normrun                     expect ok                         rule JOIN
