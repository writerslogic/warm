#!/usr/bin/env bash
C=$HOME/bigann/s2; O=$C/out
echo "alive=$(pgrep -f 's2_chain.sh' | wc -l) py=$(pgrep -f 'run.py --nodocker' | wc -l) now=$(date -u +%FT%TZ) last=[$(tail -n 1 $HOME/bigann/logs/s2-status.txt)] ABORT=$(cat $C/ABORT 2>/dev/null) DONE=$(cat $C/DONE 2>/dev/null) steps=$(grep -c '^Step ' $O/harness-stdout.log 2>/dev/null) rss_mb=$(ps -C python -o rss= --sort=-rss 2>/dev/null | head -1 | awk '{printf "%d",$1/1024}')"
