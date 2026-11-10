# AZ-1: frozen streaming and filter confirmation

Full official Big ANN harness runs on Azure Standard_E8bds_v5 in East US,
Intel Xeon Platinum 8370C, 4 physical cores / 8 threads. This is **not** the
organisers' D8lds_v5: host RAM is 64 GiB and storage limits differ. Streaming
containers were limited to 8 GiB and filter containers to 16 GiB, without swap;
the host page cache was dropped before each run. `hardware.txt` and
`harness-limits.json` record the measured configuration.

Order: streaming control, E2a, E2a, control, filter, filter. Source pins and all
parameters were frozen before Azure measurement (`summary.json`). All 1,280
streaming operations and 640 searches completed in every streaming run. Each
filter run used all 100,000 public queries and retained all six submitted
configurations. The headline remains brute_max=5000, target=1750; no point was
selected using these Azure results. The unchanged official exporter produced
`export.csv` in each run directory.

The E2a complete-workload time gate was 3,240 seconds. Both repeats pass at
2,865.57 / 2,871.94 seconds and recall .9965711875. Control recall is .994244125
at 2,598.39 / 2,591.44 seconds. Filter headline QPS is 54,721.70 / 53,972.75 at
recall .906415. Peak RSS is 6.52–6.59 GiB for streaming and 14.385 GiB for filter;
GNU time measured process RSS, not cgroup memory.peak.

`input-manifest.json` records input identities. Runner patches add resource/time
accounting and the stated Docker limits without changing search or scoring.
The E2a/filter image build recipes fetch their exact frozen commits explicitly
because they may not be advertised by a fresh clone. Raw HDF5, complete logs,
image metadata and recipes are retained in the full AZ-1 archive; its verified
SHA256 is in `archive.sha256`. The compact exports and receipts here are copied
without changing results.

These runs confirm behavior on this Azure host. They do not establish an
organiser-published rank or constitute measurement on D8lds_v5. Private queries
remain unmeasured.
