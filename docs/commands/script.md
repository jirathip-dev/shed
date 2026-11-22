`shed script [--on M] (-f FILE | -) [--check] [--timeout S] [--role R]`

Run a program in shed's JavaScript subset on the server, so many dependent calls and a filter are one round trip (skill/script/SKILL.md).
It prints the program's logs, then what it returned; an error prints its name, line and column and exits 1.
`--check` only parses and compiles: nothing runs and no call is made.
A call that acts goes through the guard in the server's `[script]` config, and is refused without one.
