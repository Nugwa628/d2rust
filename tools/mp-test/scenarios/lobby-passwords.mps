# Lobby passwords (owner 2026-10-19): a game created with a blank password
# is public and anyone can join; a game with a password needs it.
name   lobby-passwords
about  A public game takes anyone; a password game refuses a blank (PasswordNeeded) or wrong (WrongPassword) password and takes the right one.
rule   PUBLIC    a game created with a blank password is public: anyone can join it without a password (contracts 3.6, owner decision 5)
rule   PASSWORD  a game created with a password can be joined only with it: blank is PasswordNeeded, wrong is WrongPassword (contracts 3.6, 3.8)
rule   NOLEAK    the game list never shows a game's password, only that it has one (contracts 3.6)
bots   4

all connect
b0  register hank hank-pass-1             expect ok
b1  register ivy  ivy-pass-1              expect ok
b2  register jack jack-pass-1             expect ok
b3  register kim  kim-pass-1              expect ok
b0  char-create Hankchar 0                expect ok
b1  char-create Ivychar 1                 expect ok
b2  char-create Jackchar 2                expect ok
b3  char-create Kimchar 3                 expect ok
b0  char-select Hankchar                  expect ok
b1  char-select Ivychar                   expect ok
b2  char-select Jackchar                  expect ok
b3  char-select Kimchar                   expect ok
b0  create-game open1                     expect ok                         rule PUBLIC
b1  join-game open1                       expect ok                         rule PUBLIC
b2  create-game locked1 password=s3cret   expect ok                         rule PASSWORD
b3  games                                 expect has.locked1=yes locked1.locked=yes   within 3s   rule NOLEAK
b3  join-game locked1                     expect refused=PasswordNeeded     rule PASSWORD
b3  join-game locked1 password=wrong      expect refused=WrongPassword      rule PASSWORD
b3  join-game locked1 password=s3cret     expect ok                         rule PASSWORD
