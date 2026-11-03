"""Shared ordered algebraic decision diagrams, one root per outcome probability."""

from fractions import Fraction
from math import gcd


class BudgetExceeded(ValueError):
    pass


class Diagrams:
    def __init__(self, descending=False, node_limit=50000, state_limit=100000):
        if type(descending) is not bool:
            raise ValueError('source order must be Boolean')
        for limit in (node_limit, state_limit):
            if type(limit) is not int or not 1 <= limit <= 100000:
                raise ValueError('diagram limits must be integers in 1..100000')
        self.descending = descending
        self.node_limit, self.state_limit = node_limit, state_limit
        self.nodes, self.unique, self.terminals, self.cache = [], {}, {}, {}
        self.visits = 0

    def terminal(self, value):
        if value not in self.terminals:
            if len(self.nodes) + len(self.terminals) >= self.node_limit:
                raise BudgetExceeded('decision diagram node/terminal limit exceeded')
            self.terminals[value] = -len(self.terminals) - 1
        return self.terminals[value]

    def node(self, variable, withdrawn, surviving):
        if withdrawn == surviving:
            return withdrawn
        key = variable, withdrawn, surviving
        if key not in self.unique:
            if len(self.nodes) + len(self.terminals) >= self.node_limit:
                raise BudgetExceeded('decision diagram node/terminal limit exceeded')
            self.nodes.append(key)
            self.unique[key] = len(self.nodes)
        return self.unique[key]

    def probability(self, numerator, total):
        self.visits += 1
        if self.visits > 8 * self.state_limit:
            raise BudgetExceeded('decision diagram visit limit exceeded')
        divisor = 0
        for count in (*numerator.values(), *total.values()):
            divisor = gcd(divisor, count)
        if not divisor:
            return self.terminal(None)
        key = tuple((mask, numerator.get(mask, 0) // divisor, count // divisor)
                    for mask, count in sorted(total.items()))
        if key in self.cache:
            return self.cache[key]
        if len(self.cache) >= self.state_limit:
            raise BudgetExceeded('decision diagram state limit exceeded')
        first_numerator, first_total = key[0][1:]
        if all(count * first_total == first_numerator * weight for _, count, weight in key):
            return self.support(tuple(mask for mask, _, _ in key), Fraction(first_numerator, first_total))
        variables = 0
        for mask, _, _ in key:
            variables |= mask
        if not variables:
            result = self.terminal(Fraction(key[0][1], key[0][2]))
        else:
            bit = (1 << (variables.bit_length() - 1)) if self.descending else variables & -variables
            children = []
            for survives in (False, True):
                child_numerator, child_total = {}, {}
                for mask, count, weight in key:
                    if mask & bit and not survives:
                        continue
                    reduced = mask & ~bit
                    if count:
                        child_numerator[reduced] = child_numerator.get(reduced, 0) + count
                    child_total[reduced] = child_total.get(reduced, 0) + weight
                children.append(self.probability(child_numerator, child_total))
            result = self.node(bit, *children)
        if len(self.cache) >= self.state_limit:
            raise BudgetExceeded('decision diagram state limit exceeded')
        self.cache[key] = result
        return result

    def support(self, masks, value):
        self.visits += 1
        if self.visits > 8 * self.state_limit:
            raise BudgetExceeded('decision diagram visit limit exceeded')
        if not masks:
            return self.terminal(None)
        if 0 in masks:
            return self.terminal(value)
        minimal = []
        for mask in sorted(set(masks), key=lambda item: (item.bit_count(), item)):
            if not any(prior & mask == prior for prior in minimal):
                minimal.append(mask)
        key = ('support', value, tuple(minimal))
        if key in self.cache:
            return self.cache[key]
        if len(self.cache) >= self.state_limit:
            raise BudgetExceeded('decision diagram state limit exceeded')
        variables = 0
        for mask in minimal:
            variables |= mask
        bit = (1 << (variables.bit_length() - 1)) if self.descending else variables & -variables
        absent = self.support(tuple(mask for mask in minimal if not mask & bit), value)
        present = self.support(tuple(mask & ~bit for mask in minimal), value)
        result = self.node(bit, absent, present)
        if len(self.cache) >= self.state_limit:
            raise BudgetExceeded('decision diagram state limit exceeded')
        self.cache[key] = result
        return result

    def profile(self, profile):
        numerators, total = profile.polynomials()
        return tuple(self.probability(numerator, total) for numerator in numerators)

    def read(self, roots, withdrawn):
        terminal_values = {node: value for value, node in self.terminals.items()}
        result = []
        for root in roots:
            while root > 0:
                variable, absent, present = self.nodes[root - 1]
                root = absent if variable & withdrawn else present
            result.append(terminal_values[root])
        if all(value is None for value in result):
            return None
        if any(value is None for value in result):
            raise AssertionError('probability components disagree on whether evidence exists')
        return tuple(result)

    def stats(self):
        return {'nodes': len(self.nodes), 'terminals': len(self.terminals),
                'states': len(self.cache), 'visits': self.visits}
