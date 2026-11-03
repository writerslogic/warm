"""Use pinned current-owner membership for the filtered pure-semantic tree."""
from semantic_scan_patch import relocate_control, replace_once


def patch(candidate):
    previous_capture = candidate[candidate.index('fn capture_query('):candidate.index('/// Captures the chunks')]
    previous_plan = candidate[candidate.index('fn capture_planned_query('):candidate.index('impl CapturedSearch {')]
    previous = (previous_capture + previous_plan).replace('fn capture_query(', 'fn previous_capture_query(')
    previous = previous.replace('capture_planned_query(', 'previous_capture_planned_query(')
    candidate = replace_once(candidate, 'struct EligibleOwners(Vec<usize>);',
        'enum EligibleOwners {\n    Sorted(Vec<usize>),\n    Hashed(FastSet<usize>),\n}')
    candidate = replace_once(candidate, '''        Self(owners)
    }

    fn contains(&self, owner: &usize) -> bool {
        self.0.binary_search(owner).is_ok()
    }''', '''        Self::Sorted(owners)
    }

    fn hashed(owners: impl ExactSizeIterator<Item = usize>) -> Option<Self> {
        (owners.len() <= DENSE_SLOT_LIMIT).then(|| Self::Hashed(owners.collect()))
    }

    fn contains(&self, owner: &usize) -> bool {
        match self {
            Self::Sorted(owners) => owners.binary_search(owner).is_ok(),
            Self::Hashed(owners) => owners.contains(owner),
        }
    }''')
    candidate = replace_once(candidate, '''        if prepared.pure_semantic() {
            scan = Some(''', '''        if prepared.pure_semantic() {
            if dense.is_some() {
                eligible = EligibleOwners::hashed(documents.iter()
                    .map(|document| Arc::as_ptr(document) as usize));
            }
            scan = Some(''')
    return candidate, relocate_control(previous)


def probe(text, previous):
    text = replace_once(text, '''                    baseline_capture_query(store, prepared)
                } else {
                    capture_query(store, prepared)''', '''                    baseline_capture_query(store, prepared)
                } else if method == 1 {
                    previous_capture_query(store, prepared)
                } else {
                    capture_query(store, prepared)''')
    text = replace_once(text, '''                } else if method == 1 {
                    materialized_execute_query(&captured)
                } else {''', '''                } else {''')
    start = text.index('fn materialized_execute_query(')
    end = text.index('#[test]', start)
    text = text[:start] + text[end:]
    text = text.replace('frozen_monotone_comparison', 'frozen_owner_membership_comparison')
    text = text.replace('frozen exposed-data computed-bound screen', 'frozen exposed-data captured-owner screen')
    text = replace_once(text, '''            let mut results = [Value::Null, Value::Null, Value::Null];''', '''            let mut owner_count = 0;
            let mut owner_capacity = 0;
            let mut results = [Value::Null, Value::Null, Value::Null];''')
    text = replace_once(text, '''                let rows: Vec<_> = ranked''', '''                if method == 2 {
                    if let Some(EligibleOwners::Hashed(owners)) = &captured.eligible {
                        owner_count = owners.len();
                        owner_capacity = owners.capacity();
                    }
                }
                let rows: Vec<_> = ranked''')
    text = replace_once(text, '"materializedSeconds": elapsed[1]', '"previousSeconds": elapsed[1]')
    text = replace_once(text, '"prunedDots": pruned_dots', '"prunedDots": pruned_dots, "ownerCount": owner_count, "ownerCapacity": owner_capacity')
    return previous + text
