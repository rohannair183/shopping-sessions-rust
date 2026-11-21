#!/bin/sh
set -eu
root=$(cd "$(dirname "$0")/.." && pwd -P)
mkdir -p "$root/runs"
run_dir=$(mktemp -d "$root/runs/fixtures.XXXXXX")
chmod 0777 "$run_dir"
docker run --rm --platform linux/amd64 --read-only --network none --user 1000:1000 \
  --cpus "${NCPU:-6}" --memory "${MEM_GB:-8}g" --memory-swap "${MEM_GB:-8}g" --cap-drop ALL \
  --security-opt no-new-privileges --pids-limit 4096 --tmpfs /tmp:rw,size=64m,mode=1777 \
  -e "NCPU=${NCPU:-6}" -v "$root/fixtures:/fixtures:ro" -v "$run_dir:/results" \
  --entrypoint /bin/sh "$1" -ec '
    for f in /fixtures/*/input; do
      case_dir=${f%/input}
      DATA_DIR="$f" OUTPUT_DIR="/results/${case_dir##*/}" /usr/local/bin/shopping-sessions
    done'
docker run --rm --platform linux/amd64 --read-only --network none -e PYTHONDONTWRITEBYTECODE=1 \
  -v "$root/fixtures:/fixtures:ro" -v "$run_dir:/actual:ro" "$2" check_batch.py /fixtures /actual
