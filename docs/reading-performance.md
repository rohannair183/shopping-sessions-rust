# How much of the input time can we remove?

Before the compact ID changes, reading the file bytes took 0.40 seconds locally. Turning those bytes into usable event and click records took 3.91 seconds. Changing how we fetch bytes alone cannot remove most of that time. See the [event and click readers](../src/input.rs) and the [reader for each file chunk](../src/csv.rs).

The [challenge rules](https://www.joinbuilders.org/buildathon/tracks/shopping-sessions/files/rules.md) require reading every input CSV. Every comma separates fields and only a newline ends a row, even when fields have quotes. A reader can use those guarantees to scan many bytes at once. It still has to detect malformed quotes and field counts, and the event timestamp can take three formats.

Our reader reuses a string, but it still copies each line into that string. [Rust 1.90's implementation](https://github.com/rust-lang/rust/blob/1.90.0/library/std/src/io/mod.rs#L2207-L2233) finds the newline and appends the bytes. [It then checks those bytes for valid UTF-8](https://github.com/rust-lang/rust/blob/1.90.0/library/std/src/io/mod.rs#L383-L398). Processing rows straight from a shared read buffer can avoid the copy. It cannot remove the work of interpreting the fields.

Linux keeps recently used file pages in memory. [Normal reads and memory mapping both use this cache](https://docs.kernel.org/mm/page_cache.html). A second read can therefore avoid physical disk access. A quick warm read measures how fast the machine can deliver cached bytes. It does not predict a first read on the scorer.

Two teams published useful comparisons. Neither measured our program:

| What the team measured | What it tells us |
| --- | --- |
| [Rust team, seed 7, 20 million events and 5 million clicks](https://github.com/shervan0714/shopping-sessions/blob/c95e1887fe85d7a1771f2c5c3b8dcfa7ba6e020f/PERFORMANCE.md): 2.6 seconds parsing in its best run on an Intel laptop under WSL2. | A faster reader can still spend seconds converting CSV into usable records. |
| [Rust team, 20 million events on an eight-CPU Xeon VM](https://github.com/kleongf/buildathon-solution/blob/940e3a0ec24919a291938c6fcb83894a958fa6c5/README.md): 1.28 seconds for the whole container. | Four seconds is not a fixed cost at this scale. The team changed date parsing, ID storage, duplicate handling, and more. Its result does not isolate a reader speedup. |

## Local measurements

I measured the two input folders on seed 7: 1.90 GB across 21,593,666 event rows and 5,137,591 click rows. I used the repo's Linux amd64 release build on an Arm Mac, with six pinned Docker CPUs and an 8 GiB memory limit. Each test ran three times. I varied the order and left the file caches alone. These are local measurements, not timings from the hidden scorer.

| Work included in each test | Median elapsed time |
| --- | ---: |
| Read every file byte into memory without interpreting it | 0.40 s |
| Use the current reader to find and copy every row | 0.52 s |
| Also split the rows into fields and check quoting and field counts | 1.22 s |
| Also convert event, ingestion, and click timestamps | 1.53 s |
| Run the current event and click readers through validation, shopper lookup, ID storage, and grouping into batches | 3.91 s |

Each row measures a separate pass over the same files. The numbers include the preceding work; do not add them. The raw byte pass uses a 256 KiB buffer. The other rows use the actual reader. The final row stops before duplicate removal, session calculations, and writing output. Creating lookup tables and freeing the completed batches are outside this timer.

The full reader took much longer than the raw byte pass. The largest gap in these tests came after splitting fields and converting timestamps: applying the remaining rules, looking up shopper and campaign IDs, hashing IDs, allocating records, and storing them. The tests do not isolate the cost of each of those operations.

I tested two changes in separate copies of the program. The block reader loads a 4 MiB chunk and gives each row to the parser as a slice of that chunk. The date test keeps the current line reader, decodes fixed-width date fields directly, and remembers the start of each recently seen date. It still checks the full date before reusing a result.

| Full input preparation | Run 1 | Run 2 | Run 3 | Median |
| --- | ---: | ---: | ---: | ---: |
| Current code | 4.48 s | 3.91 s | 3.62 s | 3.91 s |
| Block reader | 3.30 s | 3.25 s | 4.46 s | 3.30 s |
| Date test | 3.13 s | 3.25 s | 3.97 s | 3.25 s |
| Block reader plus date test | 3.93 s | 3.10 s | 3.89 s | 3.89 s |

All four tests produced the same candidate count. The trial times overlap. Neither single change got input preparation near zero, and their savings did not add together.

## Whole-program check

I alternated three runs per version on the same unchanged practice input, in fresh containers. Times measure the release program from setup through writing both reports. They exclude image build and container startup.

| Version | Run 1 | Run 2 | Run 3 | Median |
| --- | ---: | ---: | ---: | ---: |
| Current program | 14.15 s | 13.61 s | 13.63 s | 13.63 s |
| Block reader | 13.64 s | 13.78 s | 13.44 s | 13.64 s |
| Date test | 13.93 s | 13.33 s | 13.74 s | 13.74 s |

Both prototypes matched the saved practice output row for row and passed all 36 public fixtures. Neither prototype reduced the median time for the whole job. I would keep the current reader and prioritize compact event IDs. In the earlier local experiment, that change alone reduced the median time for the whole job from 13.00 to 10.53 seconds. Measuring how much the reader spends looking up and storing shopper IDs would be the next input experiment.

The input timings, trial logs, and benchmark code stayed in the local run directory. Neither reader prototype shipped.

The main program now stores compact event IDs and uses numbers for shopper IDs. See the [current verification and timings](../README.md#verified-in-this-workspace).
