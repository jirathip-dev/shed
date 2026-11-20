load average 42.9, 15 runs each

| query | flags | lines / files | rg | rg -j1 | prototype | rg bytes | text bytes | json bytes | same lines |
|---|---|---|---|---|---|---|---|---|---|
| literal, rare | `Pattern::new` -F | 3 / 2 | 10.3 (17.8) ms | 10.3 (8.3) ms | 9.8 (7.8) ms | 295 | 419 | 829 | yes |
| literal, common | `spawn_blocking` -F | 9 / 4 | 8.9 (16.3) ms | 7.3 (6.4) ms | 6.9 (6.2) ms | 2,260 | 2,215 | 2,686 | yes |
| literal, whole word | `wait` -F -w -g *.rs -g !tests/ | 41 / 13 | 8.3 (8.1) ms | 7.4 (6.0) ms | 6.5 (5.3) ms | 4,083 | 4,369 | 7,460 | yes |
| literal, any case | `handover_refused` -F -i | 8 / 4 | 8.9 (16.4) ms | 8.0 (7.1) ms | 7.3 (6.5) ms | 1,339 | 1,622 | 2,468 | yes |
| pattern with a literal in it | `fn \w+_limit`  | 11 / 7 | 11.5 (20.1) ms | 12.4 (10.1) ms | 8.9 (7.7) ms | 1,223 | 2,113 | 3,529 | yes |
| alternation of literals | `TODO|FIXME|HACK`  | 4 / 2 | 8.9 (17.1) ms | 8.3 (7.3) ms | 9.1 (8.3) ms | 435 | 497 | 769 | yes |
| group and alternation | `Method::(Exec|FileRead|FileWrite)\b`  | 9 / 4 | 10.6 (18.7) ms | 11.2 (9.9) ms | 9.6 (8.5) ms | 820 | 860 | 1,506 | yes |
| counted repeat, no literal | `\d{2,}\.\d+\.\d+` -g *.toml | 0 / 0 | 9.8 (7.8) ms | 8.5 (7.2) ms | 4.9 (3.8) ms | 0 | 40 | 39 | yes |
| anchored, optional group | `^\s*pub (async )?fn [a-z_]+\(&mut self`  | 6 / 2 | 9.3 (15.8) ms | 8.5 (7.5) ms | 7.8 (6.9) ms | 519 | 688 | 1,509 | yes |
| no literal: every line tried | `[0-9a-f]{40}`  | 156 / 1 | 9.7 (19.7) ms | 11.5 (10.5) ms | 26.0 (24.0) ms | 14,544 | 13,675 | 15,603 | yes |
| no match | `zzzz_not_here` -F | 0 / 0 | 10.6 (18.0) ms | 10.7 (8.7) ms | 9.6 (7.7) ms | 0 | 41 | 40 | yes |

| other | rg or cat | prototype | their bytes | text bytes | json bytes |
|---|---|---|---|---|---|
| over the limit: `self` -F | | exit 3: `LIMIT matches: over 250 at crates/shed-core/src/history.rs` | | | |
| the same with `--count` | 8.2 (20.0) ms | 7.7 (6.8) ms | 2,350 | 2,467 | 5,710 |
| outline `crates/shed-server/src/registry.rs` | 1.7 (1.1) ms | 2.1 (1.3) ms | 74,305 | 3,493 | 7,961 |
| outline `docs/protocol.md` | 2.2 (1.3) ms | 2.2 (1.3) ms | 28,173 | 1,341 | 2,742 |
| find `handover` (rg -l -i) | 9.9 (18.9) ms | 10.9 (9.0) ms | 683 | 1,447 | 2,187 |
| find `line cap` (rg -l -i) | 9.4 (23.2) ms | 11.3 (10.3) ms | 4,376 | 1,679 | 2,640 |
| find `wait output pattern` (rg -l -i) | 9.4 (22.4) ms | 13.6 (11.3) ms | 2,991 | 1,866 | 3,049 |

| big tree | rg | rg -j1 | prototype, limits off | prototype, limits on |
|---|---|---|---|---|
| `spawn_blocking` -F | 1.1 (3.0) s, 131 files | 1.8 (1.6) s, 131 files | 2.4 (2.2) s, 131 files | 0.6 (0.6) s, exit 3: `LIMIT bytes: over 268435456 at icu_segmenter-2.4.0/tests/testdata/LineBreakTest_17.txt` |
| `fn poll_` -F | 0.5 (4.4) s, 1026 files | 2.2 (2.0) s, 1026 files | 2.6 (2.4) s, 1026 files | 0.3 (0.3) s, exit 3: `LIMIT matches: over 250 at futures-util-0.3.34/src/sink/unfold.rs` |
| `[0-9a-f]{40}`  | 0.6 (5.4) s, 2723 files | 3.4 (3.3) s, 2723 files | 18.0 (14.9) s, 2723 files | 1.3 (0.8) s, exit 3: `LIMIT matches: over 250 at compact_str-0.9.1/Cargo.lock` |
