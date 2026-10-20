# Play together: a waypoint trip and an act change, and what the others see.
# The test server (dev = test) gives both characters every waypoint, opens
# acts 2-5 and takes both to the act 4 town, whose start is next to its
# waypoint (a waypoint trip ends next to a waypoint too). An act change is a
# waypoint trip to that act's town.
name   waypoints-acts
about  b0 takes a waypoint to act 1 and leaves b1's sight; b0 goes to act 2; b1 follows and sees b0 again.
rule   TRAVEL  a player that travels by waypoint or to another act arrives there, and leaves the sight of the players it left behind (multiplayer.md)
rule   MEET    players in the same area see each other again after travelling (multiplayer.md)
bots   2
server dev=test

all connect
b0  register leo leo-pass-1               expect ok
b1  register mia mia-pass-1               expect ok
b0  char-create Leopal 3                  expect ok
b1  char-create Miaama 0                  expect ok
b0  char-select Leopal                    expect ok
b1  char-select Miaama                    expect ok
b0  create-game travel                    expect ok
b1  join-game travel                      expect ok
b0  area                                  expect area=rogue-encampment  within 5s rule TRAVEL
server dev Leopal waypoints
server dev Leopal acts
server dev Miaama waypoints
server dev Miaama acts
server dev Leopal town 4
server dev Miaama town 4
b0  area                                  expect area=pandemonium-fortress act=4  within 5s rule TRAVEL
b1  see                                   expect b0=yes                 within 5s rule MEET
b0  waypoint cold-plains                  expect ok                          within 20s rule TRAVEL
b0  area                                  expect area=cold-plains  within 5s rule TRAVEL
b1  see                                   expect b0=no             within 3s rule TRAVEL
b0  act 2                                 expect ok                          within 20s rule TRAVEL
b0  area                                  expect act=2             within 5s rule TRAVEL
b1  act 2                                 expect ok                          within 20s rule MEET
b1  see                                   expect b0=yes            within 5s rule MEET
