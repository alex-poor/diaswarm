#!/usr/bin/env bash
# Capture a 2-minute native heap profile of the running Ayni follower while it
# climbs, WITHOUT reinstalling (no identity/grant wipe). Same method that found
# pending_open_paths. Analyse for the reader read path:
#   open_segment / segments_covering / AnyHeader::decode / decrypt_data / verify
#
# Usage:  ./ayni-heapprofd.sh [serial]     (default: the Ayni phone)
set -eu
SERIAL="${1:-2C011FDH200MYL}"
PKG=nz.diaswarm.ayni
ADB="adb -s $SERIAL"

echo "# Ayni pid on $SERIAL:"; $ADB shell pidof "$PKG" || { echo "Ayni not running"; exit 1; }

# heapprofd needs the process profileable; on a rooted phone this is fine.
cat > /tmp/ayni-heap.cfg <<EOF
buffers { size_kb: 131072 }
data_sources {
  config {
    name: "android.heapprofd"
    heapprofd_config {
      sampling_interval_bytes: 4096
      process_cmdline: "$PKG"
      shmem_size_bytes: 16777216
      dump_at_max: true
    }
  }
}
duration_ms: 120000
EOF

$ADB push /tmp/ayni-heap.cfg /data/local/tmp/ayni-heap.cfg >/dev/null
echo "# capturing 120s (let the heap climb now)…"
$ADB shell "su root sh -c 'cat /data/local/tmp/ayni-heap.cfg | perfetto --txt -c - -o /data/misc/perfetto-traces/ayni-heap.pftrace'"
$ADB pull /data/misc/perfetto-traces/ayni-heap.pftrace /tmp/ayni-heap.pftrace
echo
echo "# Open /tmp/ayni-heap.pftrace at https://ui.perfetto.dev  ->  'Native heap profile' flamegraph."
echo "# EXPECT, if the diagnosis holds: the hot frames are the reader read path"
echo "#   (open_segment / decrypt_data / segments_covering / AnyHeader::decode / verify),"
echo "#   NOT p2panda-net sync sessions or iroh. Retained bytes grow with it."
