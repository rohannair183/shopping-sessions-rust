"""Compare both Rust output tables for each fixture using the official schema and row readers."""
from pathlib import Path
import sys

from run_fixtures import SCHEMAS, load_actual, load_expected, rows


def main():
    fixtures, actual = map(Path, sys.argv[1:])
    cases = sorted(p for p in fixtures.iterdir() if p.is_dir())
    failures = []
    for case in cases:
        for table, schema in SCHEMAS.items():
            expected_rows = rows(load_expected(case / "expected" / f"{table}.csv", schema))
            actual_rows = rows(load_actual(actual / case.name / table, schema))
            if actual_rows != expected_rows:
                failures.append(f"{case.name}/{table}: expected {expected_rows}, got {actual_rows}")
    for failure in failures:
        print(failure, file=sys.stderr)
    if not cases or failures:
        return 1
    print(f"Matched both tables on all {len(cases)} fixtures.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
