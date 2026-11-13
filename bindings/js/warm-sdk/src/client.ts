// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

import type * as T from './types.js'
import * as v from './validation.js'
import { envelope, response, schemas } from './responses.js'
import { Transport, WarmError } from './transport.js'

type Input = Record<string, unknown>
type Validator = (value: unknown) => unknown
const time: Validator = value => v.integer(value, 'time', Number.MIN_SAFE_INTEGER)
const text: Validator = value => v.string(value, 'text', 64 * 1024)
const optionalString: Validator = value => v.string(value, 'string')
const filter: Validator = value => v.json(v.record(value, 'filter'))
const bool: Validator = value => v.boolean(value, 'option')
const rememberKeys = ['id', 'eventTime', 'validFrom', 'validTo', 'subject', 'supersedes', 'entities', 'metadata', 'embedding', 'sourceUri']
const recallKeys = ['k', 'asOf', 'knownAt', 'history', 'subject', 'entities', 'filter', 'embedding', 'lexicalWeight', 'semanticWeight', 'minSemanticScore', 'fusion', 'candidateLimit', 'explain']

function copy(source: Input, target: Input, key: string, validate: Validator, wire = key, nullable = true): void {
  if (source[key] !== undefined) target[wire] = source[key] === null && nullable ? null : validate(source[key])
}

function label(value: unknown): string {
  const result = v.string(value, 'label')
  v.string(result.trim(), 'label', 1024, true)
  if (result.includes('\0')) throw new TypeError('Labels must not contain NUL')
  return result
}

function labels(value: unknown): string[] {
  const items = v.array(value, 'entities').map(label)
  if (new Set(items.map(item => item.trim())).size > 256) throw new RangeError('At most 256 distinct entities are supported')
  return items
}

function links(value: unknown): string[] {
  const items = v.array(value, 'supersedes').map(v.id)
  if (new Set(items).size > 256) throw new RangeError('At most 256 distinct supersedes IDs are supported')
  return items
}

function fact(source: Input, content: unknown): Input {
  const out: Input = { text: v.string(content, 'text', 64 * 1024, true) }
  copy(source, out, 'id', v.id)
  for (const key of ['eventTime', 'validFrom', 'validTo']) copy(source, out, key, time)
  copy(source, out, 'subject', label)
  copy(source, out, 'supersedes', links)
  copy(source, out, 'entities', labels)
  copy(source, out, 'metadata', value => v.metadata(value, true))
  copy(source, out, 'embedding', v.vector)
  copy(source, out, 'sourceUri', optionalString)
  if (typeof out.validFrom === 'number' && typeof out.validTo === 'number' && out.validFrom >= out.validTo) throw new RangeError('validFrom must be before validTo')
  return out
}

function recall(source: Input, query: unknown): Input {
  const out: Input = { query: v.string(query, 'query', 8192) }
  copy(source, out, 'k', value => v.integer(value, 'k', 1, 1000))
  copy(source, out, 'candidateLimit', value => v.integer(value, 'candidateLimit', 1, 4096))
  for (const key of ['asOf', 'knownAt']) copy(source, out, key, time)
  copy(source, out, 'history', value => v.choice(value, 'history', ['current', 'all', 'historical']))
  copy(source, out, 'subject', label)
  copy(source, out, 'entities', labels)
  copy(source, out, 'filter', filter)
  copy(source, out, 'embedding', v.vector)
  for (const key of ['lexicalWeight', 'semanticWeight']) copy(source, out, key, value => v.number(value, key, 0))
  copy(source, out, 'minSemanticScore', value => v.number(value, 'minSemanticScore', -1, 1))
  copy(source, out, 'fusion', value => v.choice(value, 'fusion', ['blend', 'rrf']))
  copy(source, out, 'explain', bool, 'explain', false)
  if (typeof out.k === 'number' && typeof out.candidateLimit === 'number' && out.candidateLimit < out.k) throw new RangeError('candidateLimit must be at least k')
  const weight = ((out.lexicalWeight as number | null | undefined) ?? 1) + ((out.semanticWeight as number | null | undefined) ?? 1)
  if (!Number.isFinite(weight) || weight <= 0) throw new RangeError('Search weights must have a finite positive sum')
  return out
}

function forgetOptions(options: Input, dryRun: boolean): Input {
  return {
    includeChain: options.includeChain === undefined ? false : v.boolean(options.includeChain, 'includeChain'),
    dryRun: options.dryRun === undefined ? dryRun : v.boolean(options.dryRun, 'dryRun'),
    purge: options.purge === undefined ? false : v.boolean(options.purge, 'purge'),
  }
}

function boundedBatch(values: unknown, convert: (value: unknown) => Input, overhead: number): Input[] {
  const items = v.array(values, 'batch', 4096)
  let bytes = overhead + Math.max(0, items.length - 1)
  return items.map(value => {
    const item = convert(value)
    bytes += v.byteLength(JSON.stringify(item))
    if (bytes > v.MAX_BYTES) throw new WarmError('Request exceeds 16 MiB', 'request_too_large')
    return item
  })
}

export class WarmClient {
  readonly url: string
  #transport: Transport
  #memory: MemoryClient

  constructor(options: T.WarmClientOptions = {}) {
    this.#transport = new Transport(options)
    this.url = this.#transport.url
    this.#memory = new MemoryClient(this.#transport)
  }

  get memory(): MemoryClient { return this.#memory }

  async addDocuments(documents: readonly T.DocumentInput[], options: T.RequestOptions = {}): Promise<T.Added> {
    const signal = v.requestSignal(options)
    const payload = boundedBatch(documents, item => {
      const source = v.record(item, 'document', ['id', 'text', 'vector', 'metadata', 'importIdentity', 'importAdoptLegacy'])
      const out: Input = { id: v.id(source.id) }
      copy(source, out, 'text', text)
      copy(source, out, 'vector', v.vector)
      copy(source, out, 'metadata', value => v.metadata(value))
      copy(source, out, 'importIdentity', value => {
        const identity = v.string(value, 'importIdentity', 64)
        if (!/^[0-9a-f]{64}$/.test(identity)) throw new TypeError('importIdentity must be a lowercase SHA-256 digest')
        return identity
      }, 'import_identity')
      copy(source, out, 'importAdoptLegacy', bool, 'import_adopt_legacy', false)
      const stamp = out.metadata == null ? undefined : (out.metadata as Input).warm_import_identity
      if (out.import_identity != null || stamp !== undefined || out.import_adopt_legacy === true) {
        if (typeof out.import_identity !== 'string' || stamp !== out.import_identity) throw new TypeError('importIdentity must match metadata.warm_import_identity')
      }
      if (out.vector == null && !(typeof out.text === 'string' && out.text.trim())) throw new TypeError('A document requires a vector or nonempty text for a server-side embedder')
      return out
    }, 2)
    if (payload.length === 0) {
      if (signal?.aborted) throw new WarmError('Request was aborted', 'aborted')
      return { added: 0 }
    }
    return this.#transport.request('POST', '/api/v1/documents/batch', payload, signal, response<T.Added>(schemas.added))
  }

  async deleteDocument(id: string, options: T.RequestOptions = {}): Promise<T.Deleted> {
    return this.#transport.request('DELETE', `/api/v1/documents/${v.segment(id)}`, undefined, v.requestSignal(options), response<T.Deleted>(schemas.deleted))
  }

  async readDocumentChunk(id: string, index = 0, options: T.RequestOptions = {}): Promise<T.DocumentChunkRecord> {
    const path = `/api/v1/documents/${v.segment(id)}/chunks/${v.integer(index, 'chunk index')}`
    return this.#transport.request('GET', path, undefined, v.requestSignal(options), response<T.DocumentChunkRecord>(schemas.chunk))
  }

  async stats(options: T.RequestOptions = {}): Promise<T.Stats> {
    return this.#transport.request('GET', '/api/v1/stats', undefined, v.requestSignal(options), response<T.Stats>(schemas.stats))
  }

  async maintainDocumentIndex(options: T.RequestOptions = {}): Promise<T.Maintained> {
    return this.#transport.request('POST', '/api/v1/index/maintain', {}, v.requestSignal(options), response<T.Maintained>(schemas.maintained))
  }

  async query(input: T.QueryInput, options: T.RequestOptions = {}): Promise<T.QueryResponse> {
    const source = v.record(input, 'query input', ['query', 'queryVector', 'today', 'topK', 'filter', 'documentIds', 'index', 'ef'])
    const out: Input = { top_k: source.topK === undefined ? 3 : source.topK === null ? null : v.integer(source.topK, 'topK', 1) }
    copy(source, out, 'query', value => v.string(value, 'query', 8192, true))
    copy(source, out, 'queryVector', v.vector, 'query_vector')
    copy(source, out, 'today', optionalString)
    copy(source, out, 'filter', filter)
    copy(source, out, 'documentIds', value => {
      const ids = v.array(value, 'documentIds', 1000, 1).map(v.id)
      if (new Set(ids).size !== ids.length) throw new TypeError('documentIds must be unique')
      return ids
    }, 'document_ids')
    copy(source, out, 'index', value => v.choice(value, 'index', ['auto', 'exact', 'graph']))
    copy(source, out, 'ef', value => v.integer(value, 'ef', 1, 4096))
    if (out.query == null && out.query_vector == null) throw new TypeError('query requires query or queryVector')
    return this.#transport.request('POST', '/api/v1/query', out, v.requestSignal(options), response<T.QueryResponse>(schemas.query))
  }
}

export class MemoryClient {
  #transport: Transport

  constructor(transport: Transport) { this.#transport = transport }

  async capabilities(contractVersion = 1, options: T.RequestOptions = {}): Promise<T.MemoryCapabilities> {
    const payload = { contractVersion: v.integer(contractVersion, 'contractVersion', 1, 0xffffffff) }
    return this.#transport.request('POST', '/api/v1/memory/capabilities', payload, v.requestSignal(options), envelope<T.MemoryCapabilities>(schemas.capabilities, 'capabilities'))
  }

  async remember(text: string, options: T.RememberOptions = {}): Promise<T.Remembered> {
    const source = v.record(options, 'remember options', [...rememberKeys, 'signal'])
    const payload = fact(source, text)
    return this.#transport.request('POST', '/api/v1/memory/remember', payload, v.signal(source.signal), value => {
      const items = envelope<T.Remembered[]>(schemas.remembered, 'remembered')(value)
      if (items.length !== 1) throw new TypeError('Expected one remembered fact')
      return items[0]!
    })
  }

  async rememberMany(facts: readonly T.RememberInput[], options: T.RequestOptions = {}): Promise<T.Remembered[]> {
    const values = boundedBatch(facts, item => {
      const source = v.record(item, 'fact', ['text', ...rememberKeys])
      return fact(source, source.text)
    }, 12)
    if (!values.length) throw new RangeError('rememberMany requires at least one fact')
    return this.#transport.request('POST', '/api/v1/memory/remember', { facts: values }, v.requestSignal(options), value => {
      const items = envelope<T.Remembered[]>(schemas.remembered, 'remembered')(value)
      if (items.length !== values.length) throw new TypeError('Remembered count does not match submitted facts')
      return items
    })
  }

  async recall(query: string, options: T.RecallOptions = {}): Promise<T.Recalled[]> {
    const source = v.record(options, 'recall options', [...recallKeys, 'signal'])
    return this.#transport.request('POST', '/api/v1/memory/recall', recall(source, query), v.signal(source.signal), envelope<T.Recalled[]>(schemas.recalled, 'results'))
  }

  async explain(query: string, id: string, options: T.ExplainOptions = {}): Promise<T.Explanation> {
    const source = v.record(options, 'explain options', [...recallKeys, 'signal'])
    const payload = { ...recall(source, query), id: v.id(id) }
    return this.#transport.request('POST', '/api/v1/memory/explain', payload, v.signal(source.signal), envelope<T.Explanation>(schemas.explained, 'explanation'))
  }

  async forget(scope: T.ForgetScope, options: T.ForgetOptions = {}): Promise<T.ForgetReport> {
    const source = v.record(scope, 'forget scope')
    const kind = v.choice(source.kind, 'scope kind', ['ids', 'subject', 'entity', 'ingestedBefore', 'eventBefore', 'matching'])
    let chosen: Input
    if (kind === 'ids') {
      v.record(source, 'forget scope', ['kind', 'ids'])
      chosen = { kind, ids: v.array(source.ids, 'ids').map(v.id) }
    } else if (kind === 'subject' || kind === 'entity') {
      v.record(source, 'forget scope', ['kind', kind])
      chosen = { kind, [kind]: label(source[kind]) }
    } else if (kind === 'matching') {
      v.record(source, 'forget scope', ['kind', 'filter'])
      const matching = v.record(source.filter, 'filter')
      if (!Object.keys(matching).length || Object.hasOwn(matching, '_mem')) throw new TypeError('Forget filter must be nonempty and must not contain reserved metadata._mem')
      chosen = { kind, filter: filter(matching) }
    } else {
      v.record(source, 'forget scope', ['kind', 'time'])
      chosen = { kind, time: time(source.time) }
    }
    const settings = v.record(options, 'forget options', ['includeChain', 'dryRun', 'purge', 'signal'])
    return this.#transport.request('POST', '/api/v1/memory/forget', { scope: chosen, ...forgetOptions(settings, false) }, v.signal(settings.signal), response<T.ForgetReport>(schemas.forgotten))
  }

  async get(id: string, options: T.RequestOptions = {}): Promise<T.FactRecord | null> {
    return this.#transport.request('POST', '/api/v1/memory/get', { id: v.id(id) }, v.requestSignal(options), envelope<T.FactRecord | null>(schemas.get, 'fact'))
  }

  async history(id: string, options: T.RequestOptions = {}): Promise<T.FactRecord[]> {
    return this.#transport.request('POST', '/api/v1/memory/history', { id: v.id(id) }, v.requestSignal(options), envelope<T.FactRecord[]>(schemas.history, 'history'))
  }

  async applyForgetting(options: T.ForgetOptions = {}): Promise<T.ForgetReport> {
    const source = v.record(options, 'forgetting options', ['includeChain', 'dryRun', 'purge', 'signal'])
    return this.#transport.request('POST', '/api/v1/memory/applyForgetting', forgetOptions(source, true), v.signal(source.signal), response<T.ForgetReport>(schemas.forgotten))
  }

  async consolidate(options: T.ConsolidateOptions = {}): Promise<T.ConsolidationReport> {
    const source = v.record(options, 'consolidation options', ['dryRun', 'allowDestructive', 'signal'])
    const payload = {
      dryRun: source.dryRun === undefined ? true : v.boolean(source.dryRun, 'dryRun'),
      allowDestructive: source.allowDestructive === undefined ? false : v.boolean(source.allowDestructive, 'allowDestructive'),
    }
    return this.#transport.request('POST', '/api/v1/memory/consolidate', payload, v.signal(source.signal), response<T.ConsolidationReport>(schemas.consolidated))
  }
}
