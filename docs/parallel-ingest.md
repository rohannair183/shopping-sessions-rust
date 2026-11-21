# Overlap, parallel shopper links and chunk scanning for clicks

The combined release cut median process time by 13.7% on seed 7 and 14.2% on seed 17 compared with main at `05483d3`. It beat main in 11 of 12 rounds. Against overlap alone, it won 2 of six rounds on seed 7 and 3 of six on seed 17.

| Seed | Release | Time through exit | Time until reports start | Peak memory | Wins against main |
| --- | --- | ---: | ---: | ---: | ---: |
| 7 | Unchanged main | 3.755 s | 2.710 s | 2.408 GiB |  |
| 7 | Overlap only | 3.228 s | 2.176 s | 2.432 GiB | 6/6 |
| 7 | All three changes | 3.241 s | 2.135 s | 2.414 GiB | 5/6 |
| 17 | Unchanged main | 3.797 s | 2.767 s | 2.148 GiB |  |
| 17 | Overlap only | 3.151 s | 2.151 s | 2.164 GiB | 6/6 |
| 17 | All three changes | 3.259 s | 2.147 s | 2.129 GiB | 6/6 |

The program loads the catalogs, then runs two branches in one Rayon pool. One parses events and removes duplicate copies. The other builds 32 tables of linked shopper IDs and then scans click chunks. Every occurrence of an alias goes to the same table. If two link rows share an ID, they join the same shopper group even across files. After both branches finish, the program routes surviving events and writes the two reports.

Each seed used 20 million requested events and 5 million requested clicks. All three builds ran on six pinned CPUs in Linux amd64 containers on an Arm Mac, with 8 GiB and no swap. Each build had one warm-up per seed. The six measured rounds used all six orders. The [raw timings](../experiments/parallel-ingest/release-times.csv) include the warm-ups as round zero. The [timer](../experiments/report-benchmarks/time-release.sh) starts inside the container and stops after process exit. The file cache was warm.

The combined release passed the Rust tests, formatting and Clippy. Both reports matched all 36 public fixtures at one and six workers and the official dev answer at six. For each practice seed, the checker compared every column against the saved reference and checked for duplicate keys.
