#!/bin/sh
set -eu

if [ "$#" -ne 2 ]; then
  echo "Usage: IMAGE=release-image sh time-release.sh INPUT_DIRECTORY NEW_RUN_DIRECTORY" >&2
  exit 2
fi

cpus=${NCPU:-6}
memory_gib=${MEM_GB:-8}
image=${IMAGE:-shopping-sessions-rust:local}
for value in "$cpus" "$memory_gib"; do
  case "$value" in
    ''|0*|*[!0-9]*) echo "NCPU and MEM_GB must be positive integers." >&2; exit 2 ;;
  esac
done

input_dir=$(cd "$1" && pwd -P)
test -f "$input_dir/manifest.json" || { echo "Missing manifest.json in $input_dir" >&2; exit 2; }
mkdir "$2"
run_dir=$(cd "$2" && pwd -P)
mkdir "$run_dir/output" "$run_dir/scratch"
# The container writes as uid 1000, which may differ from the host user.
chmod 0777 "$run_dir/output" "$run_dir/scratch"

exec docker run --rm --platform linux/amd64 \
  --entrypoint /bin/sh \
  --cpuset-cpus "0-$((cpus - 1))" --memory "${memory_gib}g" --memory-swap "${memory_gib}g" \
  --network none --read-only --user 1000:1000 --tmpfs /tmp:rw,size=64m,mode=1777 \
  --pids-limit 4096 --cap-drop ALL --security-opt no-new-privileges \
  -e "NCPU=$cpus" -e "MEM_GB=$memory_gib" \
  -e DATA_DIR=/data -e OUTPUT_DIR=/output -e SCRATCH_DIR=/scratch -e HOME=/scratch \
  --mount "type=bind,src=$input_dir,dst=/data,readonly" \
  --mount "type=bind,src=$run_dir/output,dst=/output" \
  --mount "type=bind,src=$run_dir/scratch,dst=/scratch" "$image" -ec '
    start=$(date +%s%N)
    /usr/local/bin/shopping-sessions
    stop=$(date +%s%N)
    echo "elapsed_ns=$((stop - start))"
    if test -r /sys/fs/cgroup/memory.peak; then
      echo "cgroup_peak_bytes=$(cat /sys/fs/cgroup/memory.peak)"
    fi'
