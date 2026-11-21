# Shopping Sessions in Rust

This project reads the challenge CSVs and writes the two daily Parquet reports. The pipeline, generator, and answer checker run in Docker. You do not need Rust or Python installed on the host.

## Run it locally

Run these commands from this folder with Docker running. The local copy includes all 36 public fixtures. On a fresh clone, download the [official starter](https://www.joinbuilders.org/buildathon/tracks/shopping-sessions/files/starter-duckdb.zip) and copy the contents of its `fixtures/` folder into this project's `fixtures/` before running `make fixtures`.

```sh
make fixtures
make sample
make run DATA=data/sample RUN=runs/my-sample
```

The last command writes `user_daily/` and `campaign_daily/` under `runs/my-sample/output/`. Choose a new run directory each time so files from an older run cannot change the result.

To compare against the official million-event answer:

```sh
make dev
make run check-dev RUN=runs/my-dev-check
```

The public answer is already in `checks/dev-answer/` on this machine. On a fresh clone, download [dev-answer.zip](https://www.joinbuilders.org/buildathon/tracks/shopping-sessions/files/dev-answer.zip) and unpack it under `checks/` before running `check-dev`.

## What's in the folder

| Path | Contents |
| --- | --- |
| `src/` | The Rust pipeline. `main.rs` connects the input, shopper reports, campaign reports, and Parquet writers. |
| `tools/generator/` | The Rust generator supplied in your ZIP, plus a Dockerfile to build it locally. |
| `data/sample/` | Generated with seed 42: 10,000 requested events and 2,500 clicks. |
| `data/dev/` | Generated with seed 1: 1,000,000 requested events and 250,000 clicks. Raw CSV row counts can differ because the generator adds duplicates and malformed rows. |
| `runs/verified-dev/output/` | Verified Parquet results for the public seed-1 dataset. |
| `fixtures/` | All 36 official edge cases and their expected rows. |
| `docs/rules.md` | Exact input, cleaning, counting, and output rules. |
| `scripts/` | Local Docker runners, the official row readers and comparator, and a batch fixture check. |

Input data, fixture datasets, expected answers, and run outputs are excluded from Git. The submission builds from the root Dockerfile, Rust source, and Cargo files.

## Where to make changes

- **Input:** `csv.rs` reads and checks CSV rows. `time.rs` parses timestamps. `identity.rs` gives each shopper one number across all linked IDs. `input.rs` stores common event IDs as eight bytes and keeps every other ID as text. It keeps the earliest copy across all chunks, even when that copy is invalid.
- **Reports:** `report.rs` builds visits once per shopper. A forward scan gives credit to the latest eligible click. For assists, it counts only the visit starts a campaign has not already covered.
- **Output:** `output.rs` writes the original shopper name to Parquet. Linked shoppers use the smallest ID in their group. For short unlinked ASCII IDs, the writer recovers the name from the shopper key. The pipeline writes shopper rows by partition and merges campaign counts across partitions.

The local runner defaults to six pinned vCPUs and 8 GiB without swap. It runs the Linux amd64 image with networking off, read-only input, a read-only root filesystem, and uid 1000. A native amd64 machine with sufficient resources can run `make run NCPU=8 MEM_GB=24 RUN=runs/native-dev`. The binary keeps intermediate records in memory and prints phase timings to stderr. Runtime and peak memory at final size have not been measured.

## Verified in this workspace

Both tables matched all 36 official fixtures. The public million-event run matched all 117,038 shopper rows and 27,400 campaign rows, including schema and values. The comparator found no missing rows, extra rows, or duplicate keys.

On the local seed-7 practice dataset, the original program took 12.98 seconds, compact event IDs alone took 10.48 seconds, and both ID changes took 7.73 seconds. Each number is the median of three runs on six pinned Docker CPUs with an 8 GiB limit. The timer covers the program through writing both reports and excludes Docker startup. All 2,242,393 shopper rows and 28,014 campaign rows matched the original practice output.

The [follow-up profile](docs/performance-profile.md) breaks down the remaining costs after the compact ID changes and compares four further optimizations.

Using short unlinked shopper IDs directly as keys avoids a lock during event and click lookup. On the same seed-7 data, this cut the median from 6.03 seconds at `5cc01d1` to 5.52 seconds across six runs per build. Median peak container memory rose from 3.28 to 3.32 GiB. Both practice reports still matched.
