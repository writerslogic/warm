"""Replace only the isolated owner's scorer; keep both full-query controls."""
from semantic_scan_patch import relocate_control, replace_once


def patch(owner_source, kernel, tests):
    start = owner_source.index('fn execute_query(')
    end = owner_source.index('fn execute_chunks(', start)
    previous = owner_source[start:end].replace('fn execute_query(', 'fn previous_execute_query(')
    previous = previous.replace('execute_semantic_scan(', 'previous_execute_semantic_scan(')
    start = owner_source.index('fn execute_semantic_scan(')
    end = owner_source.index('fn execute_chunks(', start)
    source = owner_source[:start] + kernel + '\n\n' + owner_source[end:]
    start = source.index('fn previous_capture_query(')
    end = source.index('fn baseline_capture_query(', start)
    source = source[:start] + relocate_control(previous) + source[end:]
    source = replace_once(source, 'mod owner_probe {', 'mod certified_probe {')
    source = replace_once(source, 'frozen_owner_membership_comparison', 'frozen_certified_scan_comparison')
    source = replace_once(source, 'frozen exposed-data captured-owner screen', 'frozen exposed-data certified arithmetic screen')
    source = replace_once(source, '''                } else if method == 1 {
                    previous_capture_query(store, prepared)
                } else {''', '''                } else {''')
    source = replace_once(source, '''                    baseline_execute_query(&captured)
                } else {
                    execute_query(&captured)''', '''                    baseline_execute_query(&captured)
                } else if method == 1 {
                    previous_execute_query(&captured)
                } else {
                    execute_query(&captured)''')
    source = replace_once(source, '            let mut owner_count = 0;',
        '            let mut screened_rows = 0;\n            let mut canonical_rows = 0;\n            let mut owner_count = 0;')
    source = replace_once(source, '                let start = Instant::now();',
        '                CERTIFIED_SCAN_COUNTS.set(None);\n                let start = Instant::now();')
    source = replace_once(source, '''                if method == 2 {
                    if let Some(EligibleOwners::Hashed(owners))''', '''                if method == 2 {
                    (screened_rows, canonical_rows) = CERTIFIED_SCAN_COUNTS.get().unwrap_or((0, 0));
                    if let Some(EligibleOwners::Hashed(owners))''')
    source = replace_once(source, '"ownerCapacity": owner_capacity, "results": results[2]',
        '"ownerCapacity": owner_capacity, "screenedRows": screened_rows, "canonicalRows": canonical_rows, "results": results[2]')
    if not source.rstrip().endswith('}'):
        raise ValueError('expected a closed test module')
    end = source.rfind('}')
    return source[:end] + tests + '\n' + source[end:]
