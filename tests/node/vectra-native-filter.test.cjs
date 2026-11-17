// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
const { test } = require('node:test')
const { runScenario } = require('./helpers/run-scenario.cjs')

const scenario = (phases) => runScenario('vectra-native-filter-scenario.cjs', 'warm-vectra-native-filter-', phases)

test('native equality filters preserve exact results and projection across mutation and restart without metadata paging', () => scenario(['parity', 'reopen']))
test('unsupported Vectra filters retain selector ordering and old-addon fallback', () => scenario(['fallback']))
test('native equality filters enforce bounds and reject unrelated migration records', () => scenario(['validation']))
test('native filtered reads respect queued writes and stale revisions', () => scenario(['concurrency']))
