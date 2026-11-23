# Decision criteria for the cross-observation correspondence direction

Written before any label was assigned, and before any counterfactual was run. The failure questions of the development split had been listed (question text and gold answer) when this was written, so the thresholds below are not blind to the population; they were fixed before they were applied.

Population: the 228 LongMemEval-V2 development questions (all questions minus `../held_ids.json`), and the 97 of them that the round-6 system failed (`final-arch/forensics_r6_dev.json`).

Definitions:

- Entity correspondence: answering needs the same real-world record, control or row to be matched across two or more observations where the match is not given by an exact identifier. Comparing two different pages, or reading one control before and after an action, is not correspondence.
- Cross-observation aggregation: the answer needs several observations or trajectories, with no identity ambiguity.

Rules, fixed in advance:

1. The identity direction is pursued as the main intervention only if at least 20% of the failures (20 of 97) have correspondence as the primary or a contributing cause by inspection, and an identity oracle can be built from independently grounded links.
2. If correspondence is below that, it is rejected for this workload. The provenance repair stays; no matcher or retrospective inference is added.
3. An alternative intervention is implemented only if some candidate shows a measured, judge-free gain on development data over its control. A candidate already killed by a recorded measurement is not rebuilt without a new idea.
4. No score is predicted from a sample of failures.
