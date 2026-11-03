"""Adapt the declared scan cohort to three controls and analytic pruning cases."""
from semantic_scan_patch import relocate_control, replace_once


def old_bounds(text):
    for name in ('search', 'search_where', 'search_where_limited'):
        text = text.replace('index.' + name + '(', 'index.' + name + '_baseline(')
    return relocate_control(text)


def build(probe, bounded_probe, candidate, baseline):
    baseline = old_bounds(baseline)
    materialized = candidate[candidate.index('fn execute_query('):candidate.index('fn execute_semantic_scan(')]
    materialized = old_bounds(materialized.replace('fn execute_query(', 'fn materialized_execute_query('))
    probe = probe.replace('semantic_scan_measure', 'monotone_measure').replace('frozen_semantic_scan_comparison', 'frozen_monotone_comparison')
    probe = probe.replace('frozen exposed-data semantic scan screen', 'frozen exposed-data computed-bound screen')
    probe = replace_once(probe, '[Value::Null, Value::Null]', '[Value::Null, Value::Null, Value::Null]')
    probe = replace_once(probe, '[0.0; 2]', '[0.0; 3]')
    probe = replace_once(probe, 'for offset in 0..2', 'for offset in 0..3')
    probe = replace_once(probe, '(ordinal + repetition + offset) % 2', '(ordinal + repetition + offset) % 3')
    probe = replace_once(probe, '''                    baseline_execute_query(&captured)
                } else {
                    execute_query(&captured)''', '''                    baseline_execute_query(&captured)
                } else if method == 1 {
                    materialized_execute_query(&captured)
                } else {
                    execute_query(&captured)''')
    probe = replace_once(probe, '''            writeln!(
                stream,''', '''            assert_eq!(results[0], results[2], "{name} {state} {filter_name} {ordinal}");
            let pruned_dots = if name.starts_with("separated-") {
                let captured = capture_query(store, prepare_search("", &options, Some(query.len())).unwrap());
                let index = captured.dense.as_ref().unwrap();
                let (_, dots) = index.search_where_limited(
                    captured.prepared.embedding.as_ref().unwrap(), captured.prepared.limit,
                    captured.prepared.minimum, |document| captured.prepared.matches(document),
                    captured.count * (query.len() + 1) / 16,
                ).expect("useful analytic pruning exhausted its budget");
                assert!(dots < captured.count);
                Some(dots)
            } else { None };
            writeln!(
                stream,''')
    probe = replace_once(probe, '"candidateSeconds": elapsed[1], "results": results[1]',
                         '"materializedSeconds": elapsed[1], "candidateSeconds": elapsed[2], "prunedDots": pruned_dots, "results": results[2]')
    analytic = bounded_probe[bounded_probe.index('    for count in [8192, 32768] {'):]
    analytic = analytic.replace('bounded_measure(', 'monotone_measure(')
    assert probe.endswith('}\n')
    return baseline + materialized + probe[:-2] + analytic
