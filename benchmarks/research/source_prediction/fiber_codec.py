import json


class Reader:
    def __init__(self, data):
        if not isinstance(data, bytes) or len(data) > 256 * 1024 * 1024:
            raise ValueError('invalid packed byte count')
        self.data, self.offset = data, 0

    def take(self, count):
        if count < 0 or self.offset + count > len(self.data):
            raise ValueError('truncated packed model')
        result = self.data[self.offset:self.offset + count]
        self.offset += count
        return result

    def integer(self):
        value = 0
        for index in range(10):
            byte = self.take(1)[0]
            if index == 9 and byte > 1:
                raise ValueError('integer overflow')
            value |= (byte & 127) << (7 * index)
            if byte < 128:
                if index and byte == 0:
                    raise ValueError('noncanonical integer')
                return value
        raise ValueError('unterminated integer')

    def count(self, limit):
        value = self.integer()
        if value > limit:
            raise ValueError('packed allocation limit')
        return value

    def vector(self):
        return [self.integer() for _ in range(self.count(50000))]

    def rows(self, width):
        return [[self.integer() for _ in range(width)] for _ in range(self.count(200000))]

    def adjacency(self):
        result = []
        def visit(keys):
            if len(keys) == 4:
                if len(result) >= 200000:
                    raise ValueError('adjacency allocation limit')
                state, action, source, target = keys
                result.append([state, action, target, source, self.integer()])
                return
            previous = 0
            for ordinal in range(self.count(200000)):
                delta = self.integer()
                if ordinal and delta == 0:
                    raise ValueError('duplicate adjacency key')
                previous += delta
                if previous >= 1 << 64:
                    raise ValueError('adjacency integer overflow')
                visit(keys + [previous])
        visit([])
        return sorted(result)


def decode(data, adjacent=False):
    reader = Reader(data)
    if reader.take(4) != b'WPF1':
        raise ValueError('invalid packed header')
    states = [reader.take(32).hex() for _ in range(reader.count(50000))]
    actions = [json.loads(reader.take(reader.count(8 * 1024 * 1024))) for _ in range(reader.count(20000))]
    result = {'states': states, 'actions': actions}
    tag = reader.take(1)
    if tag in (b'F', b'Q'):
        if tag == b'Q':
            result['stateMap'] = reader.vector()
        result['records'] = reader.adjacency() if adjacent else reader.rows(5)
    elif tag == b'T':
        result['controlCount'] = reader.count(50000)
        result['stateMap'] = reader.vector()
        result['instances'] = [[control - 1, source - 1] for control, source in reader.rows(2)]
        result['rules'] = reader.rows(4)
        result['fallback'] = reader.adjacency()
    else:
        raise ValueError('unknown packed model type')
    if reader.offset != len(data):
        raise ValueError('trailing packed data')
    return result
