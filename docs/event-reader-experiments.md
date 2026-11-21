# Grouped shopper lookup and direct CSV reading

We tested both proposals on the same seed-7 practice input. The control was commit `9217386`. Each variant runs in a separate copy of the program.

Grouping cut the median whole job from 4.85 to 3.95 seconds, an 18.6% reduction, and finished first against the control in all six rounds. The direct scanner alone cut the median by 0.06 seconds and won five rounds. Neither matched the isolated click reader at 0.392 seconds.

| Median of six runs | Control | Direct scanner | Grouped lookup |
| --- | ---: | ---: | ---: |
| Event reader alone | 1.691 s | 1.607 s | 1.180 s |
| Read both inputs, remove duplicates and route events in the full job | 2.767 s | 2.631 s | 2.049 s |
| Whole job through exit | 4.852 s | 4.791 s | 3.950 s |
| Peak container memory | 3.18 GiB | 3.23 GiB | 2.40 GiB |

The control looks up the shopper for all 21,118,908 valid event copies. In a separate grouped run, the 32 partitions resolved 1,285,232 distinct raw shopper IDs across the valid winners, 93.9% fewer calls. Record packing and duplicate handling also changed, so the total time saved cannot be attributed to lookup alone.

In a separate head-to-head test, adding the direct scanner to grouping cut the median event read from **1.180 to 1.010 seconds**. Both changes together beat grouping alone in all six isolated pairs and in five of six whole-job pairs. The whole-job medians within this comparison were **4.317 and 4.039 seconds**. The median saving within a pair was 0.155 seconds. Grouping alone took longer in this set than in the first, so comparing the combined result with the earlier 3.950-second figure would give the wrong impression. The combined reader still took about 2.6 times the control click-reader time.

All three variants passed Rust tests, formatting and Clippy, both tables on all 36 public fixtures, the official dev answer, and the saved seed-7 practice output. The practice checker compared 2,242,393 shopper rows and 28,014 campaign rows for each variant. It found no missing rows, extra rows or duplicate keys. The default reader now uses the combined implementation.

## What changed

The [direct scanner](../experiments/event-reader-pair/scan.patch) reads events in chunks. It finds commas, quotes and newlines together, then reads the seven fields from the chunk. Short quoted rows use the fast path. Long rows use the existing field parser. This changes only how events are read. Clicks, shopper lookup, duplicate removal and reports use the control code.

The [grouped lookup](../experiments/event-reader-pair/grouped.patch) keeps a 16-byte version record for a common event ID and a 16-byte payload for a common valid event. The control uses a 64-byte candidate. Unusual IDs, timestamp ranges and categories use separate storage to preserve their exact values. Duplicate removal marks losing payloads in place. An invalid earliest copy still suppresses later valid copies. Workers then route surviving events by raw shopper ID. An exact map in each partition resolves that raw ID once and reuses the result.

This prototype keeps link building before event reading, as in the control. It still emits the control's shopper records for reporting. It does not implement the public Polaris program's overlap with link building or its in-place report writer.

The [combined patch](../experiments/event-reader-pair/combined.patch) uses the direct scanner in the grouped implementation. The direct scanner, grouped lookup and combined patches each apply independently to `9217386`.

## What the timers measure

The [full runs](../experiments/event-reader-pair/full-times.csv) include process launch, building the shopper links, reading the files, duplicate removal, both reports and process exit. The timer is inside the container and excludes Docker startup. Six measured rounds cover all six orderings of the three versions. Each version also ran once to warm the cache before the comparison.

The [isolated reader runs](../experiments/event-reader-pair/isolated-times.csv) give all six workers to one reader. The timer starts after links and catalogs are available and stops when the reader has stored its records, before duplicate removal, regrouping or freeing the records. Each event run stored 21,582,714 copies. Each control click run stored 5,082,806 clicks. Neither change modifies the click reader. In the full-job CSV, the event and click wall time columns can overlap and can include waits for other workers. Do not add them together.

The [combined comparison](../experiments/event-reader-pair/combined-times.csv) compares grouping with and without direct scanning in a later set of six pairs. In three pairs each version ran first. Compare versions within the same set of runs.

All measurements used the Linux amd64 release build on the same Arm Mac, with six pinned CPUs, 8 GiB of memory, no swap, and the same read-only practice files. Docker event logs identified overlapping container work. The first isolated round overlapped a separate build. Its [four original timings](../experiments/event-reader-pair/excluded-isolated-times.csv) are excluded from the reported medians. The replacement used the same order. The [timer code](../experiments/event-reader-pair/instrument.py) and [trial order](../experiments/event-reader-pair/bench.py) are saved alongside the patches.

A report checker also overlapped the second isolated pair in the combined comparison. We [excluded those two readings](../experiments/event-reader-pair/excluded-combined-times.csv) and repeated the pair in the same order. No measured whole-job pair overlapped another recorded container job.
