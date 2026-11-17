// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
const { test } = require('node:test')
const { runScenario } = require('./helpers/run-scenario.cjs')

const scenario = (phases) => runScenario('vectra-scenario.cjs', 'warm-vectra-', phases)

test('vector-only migration retains f64 values, original metadata and indexed filter view across restart', () => scenario(['import', 'reopen']))
test('ordered transactions commit atomically, cancel cleanly and reject stale revisions', () => scenario(['transactions', 'transactions-reopen']))
test('concurrent reads respect queued write and transaction barriers', () => scenario(['read-concurrency']))
test('high-recall graph profile survives reopen and reports its construction settings', () => scenario(['high-recall', 'high-recall-reopen']))
test('complete pre-top-k eligibility covers later metadata pages, negative scores and clean-conflict retries', () => scenario(['paging']))
test('bounded filter cache retains alternating filters and rejects stale revisions', () => scenario(['filter-cache-lru']))
test('cached filters preserve Vectra selector quirks and operator ordering', () => scenario(['filter-cache-selector']))
test('cached filter reads remain revision-consistent across queued writes', () => scenario(['filter-cache-concurrency']))
test('filter cache respects its aggregate byte limit at the boundary', () => scenario(['filter-cache-budget']))
test('resource boundaries reject invalid input and preserve committed records', () => scenario(['bounds']))
test('actual Vectra export sidecars import without source writes; failed destinations remain quarantined', () => scenario(['source']))
