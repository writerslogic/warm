# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Ordered Boolean decision diagrams as a query-specialized exact control."""

from probe import Index, validate


class Diagram:
    def __init__(self, max_nodes=250000):
        if type(max_nodes) is not int or not 2 <= max_nodes <= 250000:
            raise ValueError('invalid diagram allocation bound')
        self.max_nodes = max_nodes
        self.nodes = [(65536, 0, 0), (65536, 1, 1)]
        self.minimum = [float('inf'), 0]
        self.unique, self.applications = {}, {}
        self.complements = {0: 1, 1: 0}

    def node(self, variable, low, high):
        if low == high:
            return low
        if not 1 <= variable <= 65535 or min(self.nodes[low][0], self.nodes[high][0]) <= variable:
            raise ValueError('unordered diagram node')
        key = (variable, low, high)
        if key not in self.unique:
            if len(self.nodes) >= self.max_nodes:
                raise ValueError('diagram node bound exceeded')
            self.unique[key] = len(self.nodes)
            self.nodes.append(key)
            self.minimum.append(min(self.minimum[low], 1 + self.minimum[high]))
        return self.unique[key]

    def variable(self, source):
        return self.node(source, 0, 1)

    def negate(self, root):
        if root not in self.complements:
            variable, low, high = self.nodes[root]
            result = self.node(variable, self.negate(low), self.negate(high))
            self.complements[root] = result
            self.complements[result] = root
        return self.complements[root]

    def apply(self, operator, left, right):
        if operator not in ('and', 'or'):
            raise ValueError('invalid Boolean operator')
        if left > right:
            left, right = right, left
        if left == right:
            return left
        if operator == 'and':
            if left == 0:
                return 0
            if left == 1:
                return right
        else:
            if left == 0:
                return right
            if left == 1:
                return 1
        key = (operator, left, right)
        if key not in self.applications:
            if len(self.applications) >= 1000000:
                raise ValueError('diagram operation-cache bound exceeded')
            variable = min(self.nodes[left][0], self.nodes[right][0])
            low_left, high_left = self.nodes[left][1:] if self.nodes[left][0] == variable else (left, left)
            low_right, high_right = self.nodes[right][1:] if self.nodes[right][0] == variable else (right, right)
            result = self.node(variable, self.apply(operator, low_left, low_right),
                               self.apply(operator, high_left, high_right))
            self.applications[key] = result
        return self.applications[key]

    def evaluate(self, root, deleted):
        while root > 1:
            variable, low, high = self.nodes[root]
            root = high if variable in deleted else low
        return bool(root)


def symbolic_certificate(records, programs, snapshots, budget):
    validate(records, programs, snapshots, budget)
    diagram, index = Diagram(), Index(records)
    heads_cache, values, visited, retained = {}, {}, set(), {}
    rows_examined = 0

    def heads(slot, snapshot):
        nonlocal rows_examined
        key = (snapshot, slot)
        if key not in heads_cache:
            valid, known = snapshot
            newer, result = 0, []
            for row in index.slots.get(slot, ()):
                rows_examined += 1
                if row.valid_from > valid or row.recorded_at > known or (row.valid_to is not None and valid >= row.valid_to):
                    continue
                available = 1
                for source in row.sources:
                    available = diagram.apply('and', available, diagram.negate(diagram.variable(source)))
                condition = diagram.apply('and', available, diagram.negate(newer))
                if diagram.minimum[condition] <= budget:
                    result.append((row, condition))
                newer = diagram.apply('or', newer, available)
                if diagram.minimum[diagram.negate(newer)] > budget:
                    break
            heads_cache[key] = result
        return heads_cache[key]

    def value(expression, snapshot):
        key = (snapshot, expression)
        if key not in values:
            operator = expression[0]
            if operator == 'eq':
                result = 0
                for row, condition in heads(expression[1], snapshot):
                    if row.value == expression[2]:
                        result = diagram.apply('or', result, condition)
            elif operator == 'not':
                result = diagram.negate(value(expression[1], snapshot))
            elif operator in ('and', 'or'):
                result = diagram.apply(operator, value(expression[1], snapshot), value(expression[2], snapshot))
            else:
                condition = value(expression[1], snapshot)
                yes = diagram.apply('and', condition, value(expression[2], snapshot))
                no = diagram.apply('and', diagram.negate(condition), value(expression[3], snapshot))
                result = diagram.apply('or', yes, no)
            values[key] = result
        return values[key]

    def walk(expression, snapshot, reach):
        key = (snapshot, expression, reach)
        if diagram.minimum[reach] > budget or key in visited:
            return
        if len(visited) >= 1000000:
            raise ValueError('symbolic expression-state bound exceeded')
        visited.add(key)
        operator = expression[0]
        if operator == 'eq':
            for row, condition in heads(expression[1], snapshot):
                if diagram.minimum[diagram.apply('and', reach, condition)] <= budget:
                    retained[row.ordinal] = row
            return
        walk(expression[1], snapshot, reach)
        if operator == 'not':
            return
        condition = value(expression[1], snapshot)
        if operator in ('and', 'if'):
            walk(expression[2], snapshot, diagram.apply('and', reach, condition))
        if operator in ('or', 'if'):
            walk(expression[2 if operator == 'or' else 3], snapshot,
                 diagram.apply('and', reach, diagram.negate(condition)))

    for snapshot in snapshots:
        for program in programs:
            walk(program, snapshot, 1)
    return sorted(retained.values(), key=lambda row: row.ordinal), {
        'diagramNodes': len(diagram.nodes), 'operationCacheEntries': len(diagram.applications),
        'snapshotSlotCompilations': len(heads_cache), 'evidenceRowsExamined': rows_examined,
        'visitedExpressions': len(visited), 'variableOrder': 'ascending source ID'}
