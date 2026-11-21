# Shopping sessions profile after compact IDs

Commit `7645344` took 8.74 seconds in the middle of three profiled runs. Reading, checking, and storing shopping events and ad clicks took 4.59 seconds. This includes date conversion, ID lookups, and preparing records for duplicate checks. It does not measure file reads alone.

| Where the time went | Seconds | Share |
| --- | ---: | ---: |
| Read and check events and clicks | 4.59 | 52.5% |
| Group each shopper's activity, build visits, and calculate ad credit | 1.41 | 16.1% |
| Keep the earliest event copy and send the survivors to their shopper groups | 1.05 | 12.1% |
| Link shopper IDs before reading events | 1.00 | 11.4% |
| Write shopper reports | 0.54 | 6.2% |
| Load the small catalogs, finalize names, merge and write campaigns, and finish | 0.15 | 1.7% |

This profile used seed 7, 21.6 million event rows and 5.1 million click rows, in a Linux amd64 container on an Arm Mac. The container had six pinned CPUs and an 8 GiB memory limit. The three profiled times were 9.19, 8.74, and 8.11 seconds. When stages overlapped, each stage received a share of that interval proportional to its active workers. The breakdown describes elapsed time, not CPU seconds. The separate release benchmarks time the process through exit and exclude Docker startup.

## Four experiments

Each subagent changed one area of the same release. I alternated run order and measured each program three times using fresh containers, the same mounted data, and the same CPU and memory limits. I did not clear the operating system's file cache.

| Version | Whole job, median | What changed |
| --- | ---: | --- |
| Main, unchanged | 8.42 s | Baseline at commit `7645344`. |
| CSV parser only | 8.14 s | On x86, find commas and quotes together, 16 bytes at a time, for rows between 16 and 127 bytes. Keep the original parser for other lengths and CPUs. |
| Shopper ID lookup only | 7.28 s | Store nine-byte aliases directly in the lookup table. Keep other lengths as full text and choose the smallest original name for each linked shopper. |
| Duplicate removal only | 7.71 s | Use a bitmap to spot compact IDs seen exactly once in a shard. Compare full IDs for repeated slots, collisions, and IDs stored as text. |
| Visit and daily totals only | 8.75 s | Keep the current visit day's totals together instead of looking up the day for every event. |
| All four changes | 5.87 s | Lowered the median by 2.55 seconds. The median of the container memory peaks was 3.31 GiB versus 3.71 GiB for unchanged main. |

The timings are not additive. For example, a report-only change does not alter the input code, yet input times varied among those trials. The combined build matched all 36 public fixtures. Each candidate individually and the combined build matched all 2,242,393 shopper rows and 28,014 campaign rows of the saved practice reports.

## Which changes to keep

I ran a second comparison using the shopper lookup and duplicate filter together as the starting point.

| Changes enabled | Whole job, median of three |
| --- | ---: |
| Shopper lookup and duplicate filter | 6.92 s |
| Both plus the CSV parser | 5.90 s |
| Both plus the report rewrite | 6.40 s |
| All four | 6.18 s |

The selected build keeps the shopper lookup, duplicate filter, and CSV parser. Its three runs took 5.97, 5.90, and 5.86 seconds. With the report rewrite added, the three runs took 6.09, 6.18, and 6.78 seconds. This test did not show a benefit from adding the report rewrite to the selected build, so the report code stays the same.

These benchmarks compare revisions on one machine. They do not predict a hidden practice score or a final-size time.

## Final build

In a fresh alternating comparison, main took 8.48, 7.63, and 8.80 seconds. The selected build took 6.10, 5.61, and 6.31 seconds. That cuts the median from 8.48 to 6.10 seconds, or 28%. The median of the container memory peaks fell from 3.67 GiB to 3.30 GiB.

| What still takes time | Main, profiled | Selected build, profiled |
| --- | ---: | ---: |
| Read and check events and clicks | 4.59 s | 2.83 s |
| Group shoppers, build visits, and credit ads | 1.41 s | 1.11 s |
| Link shopper IDs | 1.00 s | 0.72 s |
| Remove duplicates and distribute surviving events | 1.05 s | 0.49 s |
| Write shopper reports | 0.54 s | 0.46 s |
| Other work | 0.15 s | 0.13 s |

Each profile column uses the middle of three instrumented runs. They are separate runs from the release comparison above. The report code is identical in both columns, so the difference in that row is not evidence for the report rewrite. Input preparation remains the largest cost, at 49% of the selected profile.

The selected build passed all six Rust tests, formatting and Clippy checks. Both output tables matched the public dev answer and the saved 20-million-event practice results, including duplicate-key checks. All 36 public fixtures passed.
