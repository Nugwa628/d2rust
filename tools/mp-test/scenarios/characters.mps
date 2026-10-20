# Online characters belong to an account and live on the server (owner
# 2026-10-19); the player picks one before the lobby.
name   characters
about  Create, list, select and delete online characters; another account neither lists nor selects them; names are unique in any case.
rule   OWNED   an online character belongs to the account that made it: only that account lists and selects it; another gets UnknownChar (contracts 3.5, 3.8)
rule   NAMES   a character name is taken once on the server, in any case (CharNameTaken) (contracts 3.8, 4)
rule   DELETE  deleting a character needs the account password (a wrong one is WrongLogin); a deleted character leaves its account's list (contracts 3.5, 4)
rule   LOGIN   character requests need a logged-in session (NotLoggedIn) (contracts 3.5)
bots   3

all connect
b2  char-list                             expect refused=NotLoggedIn    rule LOGIN
b0  register dave dave-pass-1             expect ok
b1  register erin erin-pass-1             expect ok
b0  char-create Davesorc 1                expect ok                     rule OWNED
b0  char-create Davebarb 4                expect ok                     rule OWNED
b1  char-create DAVESORC 1                expect refused=CharNameTaken  rule NAMES
b0  char-list                             expect count=2 has.Davesorc=yes has.Davebarb=yes   rule OWNED
b1  char-select Davesorc                  expect refused=UnknownChar    rule OWNED
b1  char-list                             expect count=0 has.Davesorc=no                     rule OWNED
b0  char-select Davesorc                  expect ok difficulties=1      rule OWNED
b0  char-delete Davebarb wrong-password   expect refused=WrongLogin    rule DELETE
b0  char-delete Davebarb dave-pass-1      expect ok                     rule DELETE
b0  char-list                             expect count=1 has.Davebarb=no                     rule DELETE
