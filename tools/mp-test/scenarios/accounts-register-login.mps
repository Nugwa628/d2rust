# Accounts come before play (owner 2026-10-19): register, then log in with
# the right password; a wrong one is refused.
name   accounts-register-login
about  Register, log in with the right password, the wrong one and an unknown name; a taken name (any case) can't be registered again.
rule   REGISTER  a new account name registers once and is logged in at once; the same name in any case is AccountNameTaken (contracts 3.5, 4)
rule   LOGIN     an account logs in with its password and only with it; an unknown account and a wrong password get the same WrongLogin (contracts 3.5, 3.8)
rule   SLOWFAIL  every failed login is answered after a fixed 500 ms, so the answer time gives nothing away (contracts 5)
rule   RULES     account names and passwords outside the rules are refused with BadAccountName and BadPassword (contracts 4)
bots   3

all connect
b0  register alice  alice-pass-1          expect ok                          rule REGISTER
b1  register ALICE  other-pass-2          expect refused=AccountNameTaken    rule REGISTER
b1  register 9lives nine-pass-1           expect refused=BadAccountName      rule RULES
b1  register bob    short                 expect refused=BadPassword         rule RULES
b1  login    alice  wrong-pass            expect refused=WrongLogin ms>=500  rule SLOWFAIL
b2  login    nobody nobody-pass           expect refused=WrongLogin          rule LOGIN
b2  login    alice  alice-pass-1          expect ok                          rule LOGIN
