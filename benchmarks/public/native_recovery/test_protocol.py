import json
import unittest

from run import ACKS, accept_ack


class ProtocolTests(unittest.TestCase):
    def test_acknowledgements_are_ordered_and_exact(self):
        for stage, message in enumerate(ACKS):
            self.assertEqual(accept_ack(json.dumps(message), stage), message)
            for other in range(len(ACKS)):
                if other != stage:
                    with self.assertRaises(ValueError):
                        accept_ack(json.dumps(ACKS[other]), stage)
            with self.assertRaises(ValueError):
                accept_ack(json.dumps({**message, 'extra': True}), stage)
        for stage in (-1, 3, 4):
            with self.assertRaises(ValueError):
                accept_ack(json.dumps(ACKS[-1]), stage)

    def test_protocol_length_boundary(self):
        encoded = json.dumps(ACKS[0])
        for size in (4095, 4096):
            self.assertEqual(accept_ack(encoded.ljust(size), 0), ACKS[0])
        with self.assertRaises(ValueError):
            accept_ack(encoded.ljust(4097), 0)


if __name__ == '__main__':
    unittest.main()
