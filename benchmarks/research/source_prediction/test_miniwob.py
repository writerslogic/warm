import gzip
import itertools
import json
import unittest

from miniwob import MAX_DEPTH, MAX_STATES, canonical_dom, clean, decode, parse, summarize, support_key


class MiniwobTests(unittest.TestCase):
    def test_text_reference_canonicalization_preserves_actual_observations(self):
        node = {'tag': 't', 'ref': -1, 'text': 'important', 'left': 4}
        self.assertEqual(canonical_dom(node), canonical_dom({**node, 'ref': -100}))
        for difference in ({'text': 'different'}, {'left': 5}, {'ref': 1}, {'tag': 'INPUT_text'}):
            self.assertNotEqual(canonical_dom(node), canonical_dom({**node, **difference}))
        parent = {'tag': 'BODY', 'ref': 4, 'children': [node]}
        self.assertNotEqual(canonical_dom(parent), canonical_dom({**parent, 'ref': 5}))

    def test_pairing_never_bridges_an_unobserved_outcome(self):
        dom = {'tag': 'BODY', 'children': [{'tag': 'BUTTON', 'recordingTarget': True}]}
        def state(phase, x=2):
            return {'dom': dom, 'action': None if phase is None else {'type': 'click', 'timing': phase, 'x': x, 'y': 4}}
        document = {'utterance': 'test', 'states': [state(None), state(1), state(3), state(1), state(1, 3), state(3, 3), state(3)]}
        result = parse(document, 'task')
        self.assertEqual([pair['indices'] for pair in result['pairs']], [[1, 2], [4, 5]])
        self.assertEqual([item['index'] for item in result['unmatched']], [3, 6])
        self.assertEqual(result['pairs'][0]['target-path'], ['target-path', (0,), {'type': 'click'}])
        self.assertEqual(len(set(result['states'])), 1)
        copy = {**document, 'utterance': 'different goal'}
        self.assertNotEqual(parse(copy, 'task')['states'], result['states'])

    def test_support_characterization_exhaustively(self):
        supports = [frozenset(index for index in range(3) if mask & (1 << index)) for mask in range(8)]
        for left, right, budget in itertools.product(supports, supports, (0, 1, 2, None)):
            masks = [removed for removed in supports if budget is None or len(removed) <= budget]
            equivalent = all(bool(left - removed) == bool(right - removed) for removed in masks)
            self.assertEqual(support_key(left, budget) == support_key(right, budget), equivalent)

    def test_source_ids_and_missing_actions_survive_aggregation(self):
        def document(a, b, event):
            return {'states': [a, b], 'pairs': [{'state': a, 'target': b, 'raw': event}]}
        summary, packet = summarize([document('a', 'z', ['click']), document('b', 'z', ['click']), document('c', 'z', ['type'])], 'raw')
        self.assertEqual(summary['ceilings']['0']['necessaryGroups'], 2)
        self.assertEqual(summary['ceilings']['1']['necessaryGroups'], 3)
        self.assertEqual(summary['noOutgoingObservationStates'], 1)
        self.assertEqual({row[3] for row in packet['records']}, {0, 1, 2})

    def test_parser_bounds_and_metadata_removal(self):
        for depth in (MAX_DEPTH - 1, MAX_DEPTH):
            value = None
            for _ in range(depth):
                value = [value]
            clean(value)
        with self.assertRaises(ValueError):
            clean([value])
        state = {'dom': {'tag': 'BODY'}, 'action': None}
        empty = parse({'utterance': '', 'states': []}, 'task')
        self.assertEqual(empty['states'], [])
        self.assertEqual(empty['pairs'], [])
        for count in (MAX_STATES - 1, MAX_STATES):
            self.assertEqual(len(parse({'utterance': '', 'states': [state] * count}, 'task')['states']), count)
        with self.assertRaises(ValueError):
            parse({'utterance': '', 'states': [state] * (MAX_STATES + 1)}, 'task')
        with self.assertRaises(ValueError):
            decode(gzip.compress(b'{"value":NaN}'))
        self.assertEqual(clean({'recordingTarget': True, 'ref': 9, 'focused': True}), {'ref': 9, 'focused': True})


if __name__ == '__main__':
    unittest.main()
