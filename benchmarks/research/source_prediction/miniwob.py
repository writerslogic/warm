"""An optimistic source-support ceiling for recorded empirical transitions."""

from collections import Counter, defaultdict
import gzip
import hashlib
import io
import json


MAX_COMPRESSED = 1048576
MAX_DECODED = 8388608
MAX_STATES = 20000
MAX_DEPTH = 128
MAX_ITEMS = 500000


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=True, allow_nan=False).encode()


def key(value):
    return hashlib.sha256(encoded(value)).hexdigest()


def clean(value, depth=0, budget=None):
    if budget is None:
        budget = [MAX_ITEMS]
    budget[0] -= 1
    if depth > MAX_DEPTH or budget[0] < 0:
        raise ValueError('JSON structural limit exceeded')
    if isinstance(value, dict):
        return {name: clean(item, depth + 1, budget) for name, item in value.items() if name != 'recordingTarget'}
    if isinstance(value, list):
        return [clean(item, depth + 1, budget) for item in value]
    if value is None or type(value) in (str, int, float, bool):
        return value
    raise ValueError('unsupported JSON value')


def decode(data):
    if len(data) > MAX_COMPRESSED:
        raise ValueError('compressed file limit exceeded')
    with gzip.GzipFile(fileobj=io.BytesIO(data)) as stream:
        raw = stream.read(MAX_DECODED + 1)
    if len(raw) > MAX_DECODED:
        raise ValueError('decoded file limit exceeded')

    def invalid(value):
        raise ValueError('non-finite JSON number')

    result = json.loads(raw, parse_constant=invalid)
    clean(result)
    return result


def targets(dom):
    found, stack = [], [(dom, ())]
    while stack:
        node, path = stack.pop()
        if not isinstance(node, dict):
            raise ValueError('DOM node must be an object')
        if node.get('recordingTarget') is True:
            found.append(path)
        children = node.get('children', [])
        if not isinstance(children, list):
            raise ValueError('DOM children must be a list')
        stack.extend((child, path + (index,)) for index, child in enumerate(children))
    return found


def action(value):
    if not isinstance(value, dict) or not isinstance(value.get('type'), str) or not value['type']:
        raise ValueError('event action needs a type')
    return {name: item for name, item in value.items() if name != 'timing'}


def canonical_dom(dom):
    result = clean(dom)
    stack = [result]
    while stack:
        node = stack.pop()
        if node.get('tag') == 't' and type(node.get('ref')) is int and node['ref'] < 0:
            node['ref'] = -1
        stack.extend(node.get('children', []))
    return result


def parse(document, task):
    if not isinstance(document, dict) or not isinstance(document.get('utterance'), str):
        raise ValueError('demonstration needs an utterance')
    states = document.get('states')
    if not isinstance(states, list) or not 0 <= len(states) <= MAX_STATES:
        raise ValueError('demonstration state count outside limit')
    clean(document)
    identities, dom_targets = [], []
    for state in states:
        if not isinstance(state, dict) or not isinstance(state.get('dom'), dict):
            raise ValueError('recorded state needs a DOM')
        dom_targets.append(targets(state['dom']))
        identities.append(key([task, document['utterance'], canonical_dom(state['dom'])]))
    pairs, unmatched, no_action = [], [], []
    index = 0
    while index < len(states):
        before = states[index].get('action')
        if before is None:
            no_action.append(index)
            index += 1
            continue
        raw = action(before)
        after = states[index + 1].get('action') if index + 1 < len(states) else None
        if (type(before.get('timing')) is int and before['timing'] == 1 and
                isinstance(after, dict) and type(after.get('timing')) is int and after['timing'] == 3 and
                action(after) == raw):
            locations = dom_targets[index]
            if len(locations) == 1:
                projected = ['target-path', locations[0], {name: value for name, value in raw.items() if name not in ('x', 'y')}]
            else:
                projected = ['raw-fallback', raw]
            pairs.append({'indices': [index, index + 1], 'state': identities[index],
                          'target': identities[index + 1], 'raw': ['raw', raw],
                          'target-path': projected, 'targetCount': len(locations)})
            index += 2
        else:
            unmatched.append({'index': index, 'timing': before.get('timing'), 'action': raw})
            index += 1
    return {'states': identities, 'pairs': pairs, 'unmatched': unmatched,
            'noActionIndices': no_action, 'reward': document.get('reward'),
            'rawReward': document.get('rawReward'), 'goal': key(document['utterance'])}


def support_key(sources, budget):
    sources = tuple(sorted(sources))
    if budget is not None and len(sources) > budget:
        return ('always-nonempty',)
    return ('support', sources)


def summarize(parsed, projection):
    supports = defaultdict(lambda: defaultdict(set))
    states, records, actions = set(), Counter(), {}
    for source, document in enumerate(parsed):
        states.update(document['states'])
        for pair in document['pairs']:
            action_id = key(pair[projection])
            actions[action_id] = pair[projection]
            supports[pair['state']][action_id].add(source)
            records[(pair['state'], action_id, pair['target'], source)] += 1
    ceilings, assignments = {}, {}
    for budget in (0, 1, 2, None):
        groups, membership = {}, {}
        for state in sorted(supports):
            signature = tuple((action_id, support_key(sources, budget))
                              for action_id, sources in sorted(supports[state].items()))
            membership[state] = groups.setdefault(signature, len(groups))
        tag = 'unlimited' if budget is None else str(budget)
        ceilings[tag] = {'activeStates': len(supports), 'necessaryGroups': len(groups),
                         'maximumMergeableStates': len(supports) - len(groups)}
        assignments[tag] = membership
    coverage = Counter(len(sources) for row in supports.values() for sources in row.values())
    return {'observedStates': len(states), 'activeStates': len(supports),
            'noOutgoingObservationStates': len(states - supports.keys()), 'actions': len(actions),
            'transitions': sum(records.values()), 'distinctTransitionRecords': len(records),
            'sourceSupportSizeHistogram': dict(sorted(coverage.items())), 'ceilings': ceilings}, {
                'states': sorted(states), 'actions': actions,
                'records': [list(row) + [count] for row, count in sorted(records.items())],
                'supportGroups': assignments,
            }
