#!/bin/bash
# usage: pre_s2.sh TAG CFGFILE ; everything (source refresh, build, runs) happens under the timing lock.
exec flock ~/hms-timing.lock bash ~/bigann/l22bench/body_s.sh "$@"
