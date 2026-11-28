# Harness shakedown on origin/main 225c2f25 (not a ship candidate)

One run of each fuzz configuration on a known build, to show the harness runs end to end and finds nothing on
code that is expected to be exact. Build `main` (pre-#554 format), GCP n2-highmem-8, 8 threads.

| index | documents | generated | boundary and length cases | total | mismatches |
|---|---|---|---|---|---|
| English Wikipedia prefix, 600k | 548,246 with words | 20,000 (seed 1) | 720 | 20,720 | 0 |
| synthetic s10k (df 511/512/513, 4095/4096/4097, 311/313/314) | 10,000 | 3,000 (seed 7) | 720 | 3,720 | 0 |
| synthetic s20k (adds SLOT_SHARE 624/625/626) | 20,000 | 3,000 (seed 7) | 720 | 3,720 | 0 |
| synthetic s40k (adds 32767/32768/32769, SLOT_SHARE 1249/1250/1251) | 40,000 | 3,000 (seed 7) | 945 | 3,945 | 0 |

The 600k run took 2,003 s. Its df table was cross-checked first: 311 terms, from the 7 longest lists down through
sampled singletons, had a single-term COUNT equal to the table on the #554-format 600k index (0 mismatches).
The 5M control was not run. Boundary terms present in the real 600k index are in
`boundary_terms.json` of each run; the synthetic indexes hold them at exactly limit-1, limit and limit+1.
