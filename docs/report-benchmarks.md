# Shopper report and click ID benchmarks

The default build combines direct shopper lookup with inline click IDs, shopper name reuse, and indexed campaign totals. It cut the median seed-7 job from 6.08 to 5.00 seconds in the final four-build comparison, a 17.8% reduction. Direct lookup plus inline clicks took 4.84 seconds in that comparison. The two report changes sped up reporting but did not reliably shorten the whole job beyond lookup plus inline clicks.

## Individual experiments

Each experiment changed one part of commit `5cc01d1`. Each row below shows the median of eight release runs. A win means that the candidate finished before the original baseline in the same round.

| Build | Whole job | Reduction in median time | Reports and writing | Peak memory | Wins |
| --- | ---: | ---: | ---: | ---: | ---: |
| Original baseline | 6.295 s | | 1.756 s | 3.295 GiB | |
| [Reuse shopper names](../experiments/report-benchmarks/names.patch) | 6.013 s | 4.5% | 1.638 s | 3.291 GiB | 6/8 |
| [Index campaign totals](../experiments/report-benchmarks/campaigns.patch) | 6.013 s | 4.5% | 1.604 s | 3.279 GiB | 6/8 |
| [Store short click IDs inline](../experiments/report-benchmarks/clicks.patch) | 5.415 s | 14.0% | 1.210 s | 3.196 GiB | 7/8 |

The shopper writer keeps the last shopper's key and encoded name across Parquet row groups. The campaign accumulator uses 28,014 slots per group on seed 7. It writes only slots with credit or assists and switches to a hash map if the arrays would exceed 64 MiB across all groups and the final merge. The click reader stores up to nine bytes inside the click record and retains longer IDs as text. Sorting compares the original bytes in both cases.

The baseline ranged from 5.70 to 7.36 seconds. Even input times varied when only the output code changed. The reports and writing column includes grouping shoppers, building visits, crediting campaigns, and writing both tables. It gives a better check on the report changes than attributing the full elapsed difference to the writer or the campaign array.

## Combined builds

This comparison used a fresh baseline measurement and the same seed-7 files. Each median covers eight runs. All three candidates beat the original baseline in every round.

| Build | Whole job | Reduction vs. original baseline | Reports and writing | Peak memory |
| --- | ---: | ---: | ---: | ---: |
| Original baseline, `5cc01d1` | 6.083 s | | 1.749 s | 3.282 GiB |
| Direct shopper lookup, `fa24651` | 5.227 s | 14.1% | 1.769 s | 3.309 GiB |
| Direct lookup and inline click IDs | 4.838 s | 20.5% | 1.263 s | 3.194 GiB |
| Direct lookup and all three experiments | 4.997 s | 17.8% | 1.184 s | 3.192 GiB |

The full combination beat direct lookup alone in seven of eight rounds. Against direct lookup plus inline click IDs, it won three of eight whole jobs. Its report stage finished sooner in seven of those eight rounds. The two report changes cost almost no additional peak memory on this input.

I ran the two fastest builds back to back for eight more pairs, alternating which went first. Direct lookup plus inline clicks took 4.869 seconds at the median, and all changes took 4.849 seconds. Each won four pairs. The median difference within a pair was 0.001 seconds in favor of the smaller build. The paired comparison showed no consistent difference in whole-job time on this input.

## Correctness

Each individual candidate and both combinations passed the Rust tests, formatting, and Clippy. The click attribution test checks that the original byte order breaks equal-time ties, including a short ID against a long ID. The campaign test checks zero-revenue credit, assist-only rows, empty slots, and the sparse fallback.

All five builds matched both tables on all 36 public fixtures. They also matched the official dev answer, with 117,038 shopper rows and 27,400 campaign rows, and the saved practice output, with 2,242,393 shopper rows and 28,014 campaign rows. The checker found no missing rows, extra rows, or duplicate keys.

## How to repeat the measurements

All builds used the same seed-7 practice files: 21,593,666 event rows and 5,137,591 click rows. Each run used a fresh Linux amd64 container on the same Arm Mac, six pinned CPUs, an 8 GiB memory limit, and no swap. A baseline run warmed the input before each comparison. The timer started inside the container and stopped after the process exited. Peak memory came from the container's cgroup. In each four-build comparison, every build ran twice in each of the four positions. The last comparison alternated the order of two builds across eight pairs. Docker start and stop events confirmed that none of the 80 reported runs overlapped another container job.

To reproduce an individual build, start at `5cc01d1` and apply its patch from the table above. For the full combination, start at `fa24651` and apply the [combined patch](../experiments/report-benchmarks/combined-on-lookup.patch). For direct lookup plus inline clicks, apply only the click patch to `fa24651`. Build the release with `make build IMAGE=shopping-sessions:trial`. The [timer](../experiments/report-benchmarks/time-release.sh) takes the input directory and a new output directory:

```sh
IMAGE=shopping-sessions:trial sh experiments/report-benchmarks/time-release.sh data/practice-seed7 runs/trial-1
```

The [raw measurements](../experiments/report-benchmarks/release-times.csv) include run order, process time, the program's three stage timings, and peak memory for every measured release. The warm-up runs are excluded. A separate Docker build started during the fourth round of the final comparison. I dropped that entire round and the two runs from round five that started before the pause. I warmed the input again and repeated those rounds in their original order. The [six discarded timings](../experiments/report-benchmarks/interrupted-times.csv) are separate from the reported results.
