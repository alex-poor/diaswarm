#!/usr/bin/env bash
# Memory sampler v2. The v1 script recorded App-Summary "Native Heap: <Pss>" only,
# which EXCLUDES swapped-out pages -- on a zram phone a leak can flatten purely by
# being compressed away. Record the allocator's own outstanding total too.
#   alloc = MEMINFO table "Native Heap" column 7 (Heap Alloc)  <- the honest leak number
#   size  = column 6 (Heap Size), swap = column 4 (SwapPss), rss = column 5
D="$1"; PKG="$2"; LABEL="$3"; IVL="${4:-120}"
START=$(date +%s)
while true; do
  out=$(adb -s "$D" shell "pid=\$(pidof $PKG); [ -z \"\$pid\" ] && { echo GONE; exit; }
    dumpsys meminfo \$pid 2>/dev/null | awk '/^ +Native Heap /{printf \"pss=%s swap=%s rss=%s size=%s alloc=%s free=%s\", \$3,\$6,\$7,\$8,\$9,\$10; exit}'
    echo -n \" uptime=\$(ps -o etime= -p \$pid | tr -d ' ')\"" 2>/dev/null | tr -d '\r')
  echo "$LABEL t+$((($(date +%s)-START)/60))m $(date +%H:%M:%S) $out"
  sleep "$IVL"
done
