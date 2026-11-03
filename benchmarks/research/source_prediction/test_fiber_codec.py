import unittest

from fiber_codec import Reader, decode
from run_fiber import adjacency, common, normalize, rows, varint, vector


class CodecTests(unittest.TestCase):
    def test_integer_boundaries_and_malformed_encodings(self):
        for value in (0, 127, 128, 129, 16383, 16384, 16385, (1 << 64) - 2, (1 << 64) - 1):
            self.assertEqual(Reader(varint(value)).integer(), value)
        for value in (-1, 1 << 64, True):
            with self.assertRaises(ValueError):
                varint(value)
        for data in (b'', b'\x80', b'\x80\x00', b'\xff' * 10, b'\xff' * 9 + b'\x02'):
            with self.assertRaises(ValueError):
                Reader(data).integer()

    def test_normalized_rows_and_adjacency_recover_all_fields(self):
        states, actions = ['00' * 32, 'ff' * 32], [{'type': 'click', 'x': 4}]
        records = normalize([[0, 0, 1, 0, 4], [0, 0, 1, 0, 2], [0, 0, 0, 1, 2], [1, 0, 1, 1, 1]])
        for tag, mapping in ((b'F', None), (b'Q', [1, 0])):
            prefix = common(states, actions) + tag + (vector(mapping) if mapping is not None else b'')
            expected = {'states': states, 'actions': actions, 'records': records}
            if mapping is not None:
                expected['stateMap'] = mapping
            for adjacent, content in ((False, rows(records)), (True, adjacency(records))):
                data = prefix + content
                self.assertEqual(decode(data, adjacent), expected)
                for broken in (data[:-1], data + b'\x00', b'BAD!' + data[4:]):
                    with self.assertRaises(ValueError):
                        decode(broken, adjacent)


if __name__ == '__main__':
    unittest.main()
