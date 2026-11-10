// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

import * as v from './validation.js'

type Check = (value: unknown) => void
const text: Check = value => { v.string(value, 'response string') }
const numeric: Check = value => { v.number(value, 'response number') }
const count: Check = value => { v.integer(value, 'response count') }
const millis: Check = value => { v.integer(value, 'response time', Number.MIN_SAFE_INTEGER) }
const bool: Check = value => { v.boolean(value, 'response boolean') }
const anything: Check = value => { if (value === undefined) throw new TypeError('Missing response value') }
const optional = (check: Check): Check => value => { if (value !== undefined) check(value) }
const nullable = (check: Check): Check => value => { if (value !== null) check(value) }
const maybe = (check: Check): Check => optional(nullable(check))
const list = (check: Check): Check => value => { v.array(value, 'response array').forEach(check) }
const strings = list(text)
const shape = (fields: Record<string, Check>): Check => value => {
  const object = v.record(value, 'response')
  for (const [key, check] of Object.entries(fields)) check(object[key])
}
const status: Check = value => { v.choice(value, 'fact status', ['current', 'superseded', 'expired', 'not-yet-valid']) }
const remembered = shape({ id: text, created: bool, supersedes: strings, supersededBy: strings, ingestTime: millis })
const chain = shape({ id: text, relation: value => { v.choice(value, 'chain relation', ['predecessor', 'successor']) }, effectiveFrom: millis, ingestTime: millis })
const explanation = shape({
  retrieved: bool, rank: maybe(count), lexicalScore: maybe(numeric), semanticScore: maybe(numeric),
  retrievalScore: maybe(numeric), lexicalWeight: numeric, semanticWeight: numeric, fusion: text,
  matchedTerms: strings, recencyFactor: numeric, historicalFactor: numeric, finalScore: maybe(numeric),
  status, asOf: millis, knownAt: maybe(millis), validFrom: maybe(millis), validTo: maybe(millis),
  withinValidity: bool, chain: list(chain),
})
const factFields = {
  id: text, text: maybe(text), status, ingestTime: maybe(millis), eventTime: maybe(millis),
  validFrom: maybe(millis), validTo: maybe(millis), subject: maybe(text), entities: strings,
  supersedes: strings, supersededBy: strings, metadata: anything,
}
const fact = shape({ ...factFields, effectiveFrom: millis })
const recalled = shape({ ...factFields, score: numeric, sourceUri: maybe(text), explanation: maybe(explanation) })
const vectorIndex = shape({ rows: count, graph: bool, current: bool, staleMutations: count, rebuildRecommended: bool })

export const schemas = {
  added: shape({ added: count }),
  deleted: shape({ deleted: bool }),
  chunk: shape({ revision: text, document: shape({ id: text, version: text, metadata: anything, chunkCount: count, sourceUri: maybe(text) }), index: count, id: text, startByte: count, endByte: count, text: maybe(text) }),
  query: shape({ matches: list(shape({ id: text, text, metadata: anything, score: numeric })) }),
  stats: shape({ tenant: text, documents: count, vectors: count, documentIndexTrained: bool, vectorIndex, usedBytes: numeric, capacityBytes: numeric }),
  maintained: shape({ documentIndexTrained: bool, vectorIndex }),
  capabilities: shape({ capabilities: shape({ contractVersion: count, profile: text, operations: strings, temporalQueries: bool, exactMetadataFilters: bool, subjectEntityFilters: bool, documentPagination: bool, committedErrorCode: text }) }),
  remembered: shape({ remembered: list(remembered) }),
  recalled: shape({ results: list(recalled) }),
  explained: shape({ explanation }),
  get: shape({ fact: nullable(fact) }),
  history: shape({ history: list(fact) }),
  forgotten: shape({ forgotten: strings, dryRun: bool, purged: bool }),
  consolidated: shape({ created: list(remembered), forgotten: strings, dryRun: bool }),
}

export function response<T>(schema: Check): (value: unknown) => T {
  return value => { schema(value); return value as T }
}

export function envelope<T>(schema: Check, key: string): (value: unknown) => T {
  return value => { schema(value); return (value as Record<string, unknown>)[key] as T }
}
