# Query-reinforced edge repair (issue #165)

Ship rule, fixed before any run: the repair (`full` arm) ships as a default-off option only if,
on **both** glove-100-angular and sift-128-euclidean, on held-out evaluation queries, it beats
**both** `base` and `control_random` in recall@10 at matched or better QPS across the ef sweep,
with its build seconds and index bytes reported. Anything less parks it.

Results: pending.
