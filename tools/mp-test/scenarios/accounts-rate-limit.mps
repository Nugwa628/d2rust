# Failed logins are limited per account (contracts 5): ten in a row lock
# the account for a while; then the right password works again. A test
# server shortens every window and lock (`limits = test`: 15 min -> 15 s),
# so the end of the lock runs in seconds.
name   accounts-rate-limit
about  Ten wrong passwords lock the account (AccountLocked with a wait); the right password is refused while locked and works after.
rule   LOCK    ten failed logins to one account lock it: the next login, even with the right password, is AccountLocked with wait_s above 0 (contracts 5)
rule   AFTER   a lock ends: once its time has passed, the right password logs in again (contracts 5)
bots   2
server limits=test

all connect
b0  register carol carol-pass-1           expect ok
b1  login carol wrong-01                  expect refused=WrongLogin   rule LOCK
b1  login carol wrong-02                  expect refused=WrongLogin   rule LOCK
b1  login carol wrong-03                  expect refused=WrongLogin   rule LOCK
b1  login carol wrong-04                  expect refused=WrongLogin   rule LOCK
b1  login carol wrong-05                  expect refused=WrongLogin   rule LOCK
b1  login carol wrong-06                  expect refused=WrongLogin   rule LOCK
b1  login carol wrong-07                  expect refused=WrongLogin   rule LOCK
b1  login carol wrong-08                  expect refused=WrongLogin   rule LOCK
b1  login carol wrong-09                  expect refused=WrongLogin   rule LOCK
b1  login carol wrong-10                  expect refused=WrongLogin   rule LOCK
b1  login carol carol-pass-1              expect refused=AccountLocked wait_s>=1   rule LOCK
wait 16s
b1  login carol carol-pass-1              expect ok                                rule AFTER
