"""Factor closed single-source behavior into shared control and a source binding."""

from collections import Counter, defaultdict, deque

from sparse_state import minimize, normalized, validate


MAX_COUNT = 2147483647


def compile_model(states, actions, sources, records):
    validate(states, actions, sources, records)
    grouped, incoming = [[] for _ in range(states)], [set() for _ in range(states)]
    for state, action, target, source, count in records:
        grouped[state].append((action, target, source, count))
        incoming[target].add(state)
    colors = [{row[2] for row in outgoing} for outgoing in grouped]
    bad = {state for state in range(states) if len(colors[state]) > 1}
    for state, outgoing in enumerate(grouped):
        if len(colors[state]) == 1 and any(colors[target] and colors[target] != colors[state]
                                           for _, target, _, _ in outgoing):
            bad.add(state)
    pending = deque(sorted(bad))
    while pending:
        for predecessor in incoming[pending.popleft()]:
            if predecessor not in bad:
                bad.add(predecessor)
                pending.append(predecessor)
    good = set(range(states)) - bad
    erased = [(state, action, target, 0, count) for state, action, target, _, count in records if state in good]
    partition = minimize(states, actions, sources, erased, current=True)['partition']
    controls = {block: index for index, block in enumerate(sorted({partition[state] for state in good}))}
    instances, state_map, keys = [], [], {}
    for state in range(states):
        source = next(iter(colors[state])) if colors[state] else -1
        key = ('bad', state) if state in bad else (controls[partition[state]], source)
        if key not in keys:
            keys[key] = len(instances)
            instances.append([-1, -1] if state in bad else list(key))
        state_map.append(keys[key])
    representatives = {}
    for state in sorted(good):
        representatives.setdefault(controls[partition[state]], state)
    rules = []
    for control, state in sorted(representatives.items()):
        by_action = defaultdict(Counter)
        for action, target, _, count in grouped[state]:
            by_action[action][controls[partition[target]]] += count
        for action, counts in sorted(by_action.items()):
            for target, count in normalized(counts):
                while count:
                    part = min(count, MAX_COUNT)
                    rules.append([control, action, target, part])
                    count -= part
    fallback = [[state_map[state], action, state_map[target], source, count]
                for state, action, target, source, count in records if state in bad]
    return {'controlCount': len(controls), 'stateMap': state_map, 'instances': instances,
            'rules': rules, 'fallback': fallback}


def expand(packet, actions, sources):
    validate(1, actions, sources, [])
    if not isinstance(packet, dict) or set(packet) != {'controlCount', 'stateMap', 'instances', 'rules', 'fallback'}:
        raise ValueError('invalid factored model fields')
    controls, mapping, instances, rules, fallback = (packet[name] for name in
                                                    ('controlCount', 'stateMap', 'instances', 'rules', 'fallback'))
    if type(controls) is not int or not 0 <= controls <= 50000:
        raise ValueError('invalid control count')
    for values, limit in ((mapping, 50000), (instances, 50000), (rules, 200000), (fallback, 200000)):
        if not isinstance(values, list) or len(values) > limit:
            raise ValueError('factored model allocation limit')
    if not mapping or not instances or any(type(value) is not int or not 0 <= value < len(instances) for value in mapping):
        raise ValueError('invalid original-state mapping')
    lookup = {}
    for index, instance in enumerate(instances):
        if (not isinstance(instance, list) or len(instance) != 2
                or any(type(value) is not int for value in instance)):
            raise ValueError('invalid instance')
        control, source = instance
        if control == -1 and source == -1:
            continue
        if not 0 <= control < controls or not -1 <= source < sources or tuple(instance) in lookup:
            raise ValueError('invalid or duplicate control binding')
        lookup[tuple(instance)] = index
    if any(not isinstance(row, list) or len(row) != 4 for row in rules):
        raise ValueError('invalid rule shape')
    validate(max(controls, 1), actions, sources,
             [(control, action, target, 0, count) for control, action, target, count in rules])
    if controls == 0 and rules:
        raise ValueError('rules without controls')
    validate(len(instances), actions, sources, fallback)
    if any(instances[row[0]] != [-1, -1] for row in fallback):
        raise ValueError('fallback overlaps a source-bound state')
    grouped = [[] for _ in range(controls)]
    for control, action, target, count in rules:
        grouped[control].append((action, target, count))
    total = len(fallback) + sum(len(grouped[control]) for control, source in instances if control >= 0 and source >= 0)
    if total > 200000:
        raise ValueError('expanded model allocation limit')
    result = [list(row) for row in fallback]
    for state, (control, source) in enumerate(instances):
        if control == -1:
            continue
        if source == -1:
            if grouped[control]:
                raise ValueError('an unbound state has outgoing rules')
            continue
        for action, target, count in grouped[control]:
            key = (target, source) if (target, source) in lookup else (target, -1)
            if key not in lookup:
                raise ValueError('missing target binding')
            result.append([state, action, lookup[key], source, count])
    validate(len(instances), actions, sources, result)
    return result
