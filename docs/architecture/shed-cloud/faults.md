# The fake link, its faults and the soak

## Where the fake link lives

Not in shed's server. shed's link to a machine is one program it starts, `ssh <target> shed connect`, with a
socket pair for its input and output. That program is the link layer's seam, and shed's machine tests already
put their own `ssh` first on the server's `PATH`. The fake link is that stand-in, made to inject faults:

- `scripts/fakelink.py`: one file, Python 3, standard library only. No line of it is in a crate.
- It replaces the fake `ssh` shell script and the `forward.py` that `crates/shed/tests/machines/main.rs`
  writes today. One fake, used by shed's link tests, by the helper's tests and by drover's harness.
- Each release carries it as the asset `fakelink.py`, under the same `SHA256SUMS`. drover's CI
  (drover#1648) takes it with the shed binary it tests, so both are of one build.

The server gets no transport trait, no fault switch and no test-only branch. What it gets, if a fault test
fails, is the fix.

## How a test reads

```rust
let world = World::new("hang");                      // a temp folder; its bin/ssh is the fake link
let m1 = world.host("user@m1.test");                 // a second shed server in its own folders, with its own host key
let main = world.main();                             // the server under test, the world's bin first on its PATH
main.machine_add("m1", &m1);
main.run_on("m1", "count");                          // a terminal that prints one numbered line a second
let seq = main.last_event();

world.fault("user@m1.test", "hang = true");          // rewrites faults.toml; open relays see it within 100 ms
main.wait_link("m1", LinkState::Reconnecting, secs(35));
assert!(main.stats("m1").is_offline());              // within 5 s, as `machine.stats` promises

world.clear("user@m1.test");
main.wait_link("m1", LinkState::Up, secs(10));
assert!(m1.terminal("count").is_running());
assert!(main.events_since(seq).are_contiguous());    // nothing lost, nothing twice
assert!(main.read_on("m1", "count").lines_are_contiguous());
```

## The world

```
<world>/bin/ssh        the fake link (a copy of scripts/fakelink.py)
<world>/hosts.toml     the targets that exist
<world>/faults.toml    the faults now; every relay reads it again each 100 ms
<world>/ssh.log        each call's arguments, for the tests that check them
<world>/<host>/        one shed server's own config, state and socket
```

```toml
# hosts.toml
["user@m1.test"]                         # a target, as `machine add` names it; `ssh://user@m1.test:2222` works too
home = "<world>/m1"                      # `shed connect` runs against the server in these folders
host_key = "ssh-ed25519 AAAA…m1"         # a machine pinned to another key is refused, with ssh's own exit code

# faults.toml
["user@m1.test"]
latency_ms = 200
stall = { pct = 5, ms = 3000 }
cut = true
hang = true
gone = true
moved_to = "user@m1b.test"
host_key = "ssh-ed25519 AAAA…new"
```

It takes the arguments shed gives `ssh`: the `-o` options (it honours `ConnectTimeout`, `HostKeyAlias` and
`UserKnownHostsFile`, and skips the rest), `-R` for the token-proxy forward, and `-- <target> shed connect`.

## The faults

| fault | key | what the fake link does |
|---|---|---|
| latency | `latency_ms` | holds every chunk, both ways, that long |
| loss | `stall` | holds that share of chunks much longer: what packet loss looks like on a stream that resends |
| loss, past the stream's patience | `cut` | ends every open relay at once (exit 255); a new connect works |
| hang | `hang` | relays nothing and closes nothing; a new connect hangs the same way |
| host gone | `gone` | ends every open relay; a new connect fails after `ConnectTimeout` |
| address change | `moved_to` | the old target is gone; the same server answers at the new target |
| key change | `host_key` | the target answers with another key: a resumed VM, or someone in between |

## What must hold

The link tests (`crates/shed/tests/machines/faults.rs`, slice C8) are this table. Times come from constants
the server has today: a 15 s heartbeat, a link read as dropped after 30 s of silence, 5 s for `machine.stats`,
10 s to connect, a retry every 5 s for the first five tries and then up to 60 s apart.

| fault | `machine.links` shows | a call on the machine | when the fault lifts |
|---|---|---|---|
| latency 200 ms | `up` | answers; `machine.stats` inside its 5 s | - |
| stall | `up` while no stall passes 30 s | answers late; never an error for a stall under 30 s | - |
| cut | `reconnecting`, `down_since` set, then `up` within 10 s | `NO_LINK` until it is up | terminals untouched; events since the last one replayed once |
| hang | `reconnecting` within 35 s | `machine.stats` says offline within 5 s; any other call answers `NO_LINK` within 35 s and blocks no call on another machine | `up` within 10 s after a hang of under a minute; the same |
| host gone | `reconnecting`, `down_since` still the time of the drop, `error` the last connect's | `NO_LINK` | `up` within 65 s of its return |
| address change | as host gone | `NO_LINK` | `machine add --replace` with the new target: `up` within 10 s, with no wait for a retry |
| key change | a pinned machine: never `up`; `error` names the key | `NO_LINK` | `--replace` with the new key: `up`. An unpinned machine is out of scope: ssh's own files decide |

A row that fails on today's server is a bug in the link, fixed in the same slice. The table is the contract
drover builds on in drover#1648.

The helper's tests use the same world. Their provider record is shell lines over it: `create` starts a
server in a new folder and adds a `hosts.toml` entry, `address` prints the target and its key, `destroy` stops
the server and sets `gone`. So demand, create, the machine link, a terminal, drain, zero and delete run end to
end on one box, and `moved_to` with a new `host_key` plays a resumed VM.

drover's harness (drover#1648) is drover's: lanes placed over several fake hosts while a fault is on. It needs
nothing from shed but the shed binary and `fakelink.py`.

## The nightly soak

`scripts/soak.sh`: real machines, real ssh, the installed `shed` and `shed-cloud`. It builds nothing and
changes no config. drover's nightly job on the main host starts it (drover#1648) and reads its exit code and
its report; shed adds no schedule, no runner and no job on a host.

| input | default | |
|---|---|---|
| `SOAK_MINUTES` | 60 | how long it runs |
| `SOAK_MACHINES` | every machine whose link is up | the machines it uses |
| `SOAK_POOL` | none | a pool to rent one VM from each round |
| `SOAK_REPORT` | `soak.json` in the folder it runs in | where the report goes |

Every terminal it starts carries the label `soak=<run>`; it reads and removes only those.

1. On each machine it starts two terminals that print one numbered line a second.
2. Every 5 minutes one real fault, in turn:
   - **cut**: the link's own ssh processes on the main host are ended.
   - **server restart**: the shed service on one machine is restarted; its terminals are kept.
   - **drain**: `shed machine drain` on one machine; a `run` there must be refused; then lifted.
   - **pool round** (with `SOAK_POOL`): demand for one lane as `--by soak`; a VM appears as a machine; a
     terminal on it; `shed-cloud drain`; the terminal removed; the VM and its machine gone; demand set back to
     nothing.
3. After each fault it waits for every link to be `up` again and writes down how long that took.
4. At the end it checks, and exits 1 on any miss:
   - every soak terminal's lines run from 1 to the last with no gap and none twice;
   - the events of its label have no gap per machine;
   - every link is `up`, no machine is draining, no soak terminal is left;
   - every VM the run created has a `removed` line, and the helper's spend for the run matches its up time.

The report is one JSON file: the faults in order, each with its recovery time, and the failed checks. Host
gone, address change and key change are left to the fake link: making them real means deleting or resuming a
VM behind the helper's back. The cost is one pool VM for a few minutes a round, inside `[caps]`: under $0.10 a
night at Boat's `large` rate.
