
### TOP_10 (mean / median us of per-query best of 10)

| category | n | warm-0.7.4-bp | warm-lu3-bp | infino-0.11.26 |
|---|---|---|---|---|
| ALL | 31 | 2383 / 431 | 2407 / 431 | 2237 / 430 |
| union | 18 | 3720 / 724 | 3761 / 747 | 3466 / 1035 |
| union:num_tokens_2 | 4 | 292 / 200 | 296 / 196 | 236 / 198 |
| union:num_tokens_3 | 4 | 410 / 378 | 424 / 396 | 709 / 694 |
| intersection | 11 | 618 / 251 | 618 / 251 | 616 / 310 |
| term | 2 | 54 / 54 | 54 / 54 | 80 / 80 |

### TOP_100 (mean / median us of per-query best of 10)

| category | n | warm-0.7.4-bp | warm-lu3-bp | infino-0.11.26 |
|---|---|---|---|---|
| ALL | 31 | 4111 / 629 | 3696 / 667 | 2968 / 817 |
| union | 18 | 6444 / 1126 | 5722 / 1052 | 4450 / 1778 |
| union:num_tokens_2 | 4 | 559 / 444 | 514 / 440 | 472 / 386 |
| union:num_tokens_3 | 4 | 627 / 607 | 635 / 614 | 1231 / 1232 |
| intersection | 11 | 1003 / 504 | 1014 / 500 | 1041 / 565 |
| term | 2 | 212 / 212 | 213 / 213 | 226 / 226 |

### TOP_1000 (mean / median us of per-query best of 10)

| category | n | warm-0.7.4-bp | warm-lu3-bp | infino-0.11.26 |
|---|---|---|---|---|
| ALL | 31 | 7392 / 1808 | 6357 / 1779 | 4585 / 2060 |
| union | 18 | 11356 / 5118 | 9567 / 3193 | 6656 / 3354 |
| union:num_tokens_2 | 4 | 3338 / 2470 | 2376 / 2403 | 1643 / 1612 |
| union:num_tokens_3 | 4 | 1424 / 1374 | 1437 / 1396 | 2836 / 2626 |
| intersection | 11 | 2015 / 1160 | 2025 / 1159 | 1884 / 1025 |
| term | 2 | 1290 / 1290 | 1288 / 1288 | 804 / 804 |

### COUNT (mean / median us of per-query best of 10)

| category | n | warm-0.7.4-bp | warm-lu3-bp | infino-0.11.26 |
|---|---|---|---|---|
| ALL | 31 | 667 / 449 | 690 / 459 | 1012 / 523 |
| union | 18 | 943 / 524 | 977 / 554 | 1387 / 752 |
| union:num_tokens_2 | 4 | 324 / 266 | 339 / 270 | 334 / 345 |
| union:num_tokens_3 | 4 | 486 / 477 | 511 / 506 | 428 / 420 |
| intersection | 11 | 333 / 254 | 342 / 262 | 576 / 430 |
| term | 2 | 18 / 18 | 23 / 23 | 28 / 28 |
