# Long-gap recording: summary

Written by `cargo run -p locsim-health --example gap_diagnostic`. Do not edit:
`tests/gap_recording.rs` compares this file with a fresh run. It records what
happened, not what should happen. The raw record of every case is `results.jsonl`.

A cell is `first poll after the gap|the next three polls`. `S` a sample of the
run that was running before the gap; `R` a sample of a later run (the supervisor
replaced the run); `N` nothing; `E(x)` the error `x`; `n/a` the placement does
not apply to the scenario.

## `late_poll`, `bare`

| scenario | placement | 0 s | 1 s | 2 s | 5 s | 10 s | 30 s | 60 s | 300 s | 1800 s | 3600 s | 86400 s |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| antimeridian_loop | after_initial | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| antimeridian_loop | mid_run | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| antimeridian_loop | near_route_end | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| circular | after_initial | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| circular | mid_run | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| circular | near_route_end | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` |
| driving | after_initial | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| driving | mid_run | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| driving | near_route_end | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` |
| fixed | after_initial | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| fixed | mid_run | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| fixed | near_route_end | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` |
| high_latitude | after_initial | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| high_latitude | mid_run | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| high_latitude | near_route_end | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` |
| random_walk | after_initial | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| random_walk | mid_run | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| random_walk | near_route_end | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` |
| route_replay | after_initial | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| route_replay | mid_run | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| route_replay | near_route_end | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| walking | after_initial | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| walking | mid_run | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| walking | near_route_end | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` |

## `late_poll`, `supervised`

| scenario | placement | 0 s | 1 s | 2 s | 5 s | 10 s | 30 s | 60 s | 300 s | 1800 s | 3600 s | 86400 s |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| antimeridian_loop | after_initial | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| antimeridian_loop | mid_run | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| antimeridian_loop | near_route_end | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| circular | after_initial | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| circular | mid_run | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| circular | near_route_end | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` |
| driving | after_initial | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| driving | mid_run | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| driving | near_route_end | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` |
| fixed | after_initial | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| fixed | mid_run | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| fixed | near_route_end | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` |
| high_latitude | after_initial | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| high_latitude | mid_run | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| high_latitude | near_route_end | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` |
| random_walk | after_initial | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| random_walk | mid_run | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| random_walk | near_route_end | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` |
| route_replay | after_initial | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| route_replay | mid_run | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| route_replay | near_route_end | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| walking | after_initial | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| walking | mid_run | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| walking | near_route_end | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` |

## `pause_resume`, `bare`

| scenario | placement | 0 s | 1 s | 2 s | 5 s | 10 s | 30 s | 60 s | 300 s | 1800 s | 3600 s | 86400 s |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| antimeridian_loop | after_initial | `S|SSS` | `S|E(AccelerationExceeded)NN` | `S|E(AccelerationExceeded)NN` | `S|E(AccelerationExceeded)NN` | `S|E(AccelerationExceeded)NN` | `S|E(AccelerationExceeded)NN` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| antimeridian_loop | mid_run | `S|SSS` | `E(DecelerationExceeded)|NNN` | `E(DecelerationExceeded)|NNN` | `E(DecelerationExceeded)|NNN` | `E(DecelerationExceeded)|NNN` | `E(DecelerationExceeded)|NNN` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| antimeridian_loop | near_route_end | `S|SSS` | `E(DecelerationExceeded)|NNN` | `E(DecelerationExceeded)|NNN` | `E(DecelerationExceeded)|NNN` | `E(DecelerationExceeded)|NNN` | `E(DecelerationExceeded)|NNN` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| circular | after_initial | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| circular | mid_run | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| circular | near_route_end | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` |
| driving | after_initial | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| driving | mid_run | `S|SSS` | `E(DecelerationExceeded)|NNN` | `E(DecelerationExceeded)|NNN` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| driving | near_route_end | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` |
| fixed | after_initial | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| fixed | mid_run | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| fixed | near_route_end | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` |
| high_latitude | after_initial | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| high_latitude | mid_run | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| high_latitude | near_route_end | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` |
| random_walk | after_initial | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| random_walk | mid_run | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| random_walk | near_route_end | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` |
| route_replay | after_initial | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| route_replay | mid_run | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| route_replay | near_route_end | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| walking | after_initial | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| walking | mid_run | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| walking | near_route_end | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` |

## `pause_resume`, `supervised`

| scenario | placement | 0 s | 1 s | 2 s | 5 s | 10 s | 30 s | 60 s | 300 s | 1800 s | 3600 s | 86400 s |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| antimeridian_loop | after_initial | `S|SSS` | `S|E(AccelerationExceeded)RR` | `S|E(AccelerationExceeded)RR` | `S|E(AccelerationExceeded)RR` | `S|E(AccelerationExceeded)RR` | `S|E(AccelerationExceeded)RR` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| antimeridian_loop | mid_run | `S|SSS` | `E(DecelerationExceeded)|RRR` | `E(DecelerationExceeded)|RRR` | `E(DecelerationExceeded)|RRR` | `E(DecelerationExceeded)|RRR` | `E(DecelerationExceeded)|RRR` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| antimeridian_loop | near_route_end | `S|SSS` | `E(DecelerationExceeded)|RRR` | `E(DecelerationExceeded)|RRR` | `E(DecelerationExceeded)|RRR` | `E(DecelerationExceeded)|RRR` | `E(DecelerationExceeded)|RRR` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| circular | after_initial | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| circular | mid_run | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| circular | near_route_end | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` |
| driving | after_initial | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| driving | mid_run | `S|SSS` | `E(DecelerationExceeded)|RRR` | `E(DecelerationExceeded)|RRR` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| driving | near_route_end | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` |
| fixed | after_initial | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| fixed | mid_run | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| fixed | near_route_end | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` |
| high_latitude | after_initial | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| high_latitude | mid_run | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| high_latitude | near_route_end | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` |
| random_walk | after_initial | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| random_walk | mid_run | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| random_walk | near_route_end | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` |
| route_replay | after_initial | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| route_replay | mid_run | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| route_replay | near_route_end | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| walking | after_initial | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| walking | mid_run | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` | `S|SSS` |
| walking | near_route_end | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` | `n/a` |

1056 cases, each run twice; 0 gave a different record the second time.
