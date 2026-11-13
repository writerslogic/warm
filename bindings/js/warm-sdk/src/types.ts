// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

export type JsonValue = null | boolean | number | string | JsonObject | readonly JsonValue[]
export interface JsonObject { readonly [key: string]: JsonValue }
export interface RequestOptions { signal?: AbortSignal }
export interface WarmClientOptions {
  url?: string
  apiKey?: string
  tenantId?: string
  timeoutMs?: number
  fetch?: typeof globalThis.fetch
}

export interface DocumentInput {
  id: string
  text?: string | null
  vector?: readonly number[] | null
  metadata?: JsonObject | null
  importIdentity?: string | null
  importAdoptLegacy?: boolean
}
export interface QueryInput {
  query?: string | null
  queryVector?: readonly number[] | null
  today?: string | null
  topK?: number | null
  filter?: JsonObject | null
  documentIds?: readonly string[] | null
  index?: 'auto' | 'exact' | 'graph' | null
  ef?: number | null
}
export interface Added { added: number }
export interface Deleted { deleted: boolean }
export interface DocumentMatch { id: string; text: string; metadata: JsonValue; score: number }
export interface QueryResponse { matches: DocumentMatch[] }
export interface DocumentInfo {
  id: string
  version: string
  metadata: JsonValue
  chunkCount: number
  sourceUri?: string | null
}
export interface DocumentChunkRecord {
  revision: string
  document: DocumentInfo
  index: number
  id: string
  startByte: number
  endByte: number
  text?: string | null
}
export interface VectorIndexStatus {
  rows: number
  graph: boolean
  current: boolean
  staleMutations: number
  rebuildRecommended: boolean
}
export interface Stats {
  tenant: string
  documents: number
  vectors: number
  documentIndexTrained: boolean
  vectorIndex: VectorIndexStatus
  usedBytes: number
  capacityBytes: number
}
export interface Maintained { documentIndexTrained: boolean; vectorIndex: VectorIndexStatus }

export type FactStatus = 'current' | 'superseded' | 'expired' | 'not-yet-valid'
export type HistoryMode = 'current' | 'all' | 'historical'
export interface MemoryCapabilities {
  contractVersion: number
  profile: string
  operations: string[]
  temporalQueries: boolean
  exactMetadataFilters: boolean
  subjectEntityFilters: boolean
  documentPagination: boolean
  committedErrorCode: string
}
export interface RememberOptions extends RequestOptions {
  id?: string | null
  eventTime?: number | null
  validFrom?: number | null
  validTo?: number | null
  subject?: string | null
  supersedes?: readonly string[] | null
  entities?: readonly string[] | null
  metadata?: JsonObject | null
  embedding?: readonly number[] | null
  sourceUri?: string | null
}
export type RememberInput = Omit<RememberOptions, 'signal'> & { text: string }
export interface RecallOptions extends RequestOptions {
  k?: number | null
  asOf?: number | null
  knownAt?: number | null
  history?: HistoryMode | null
  subject?: string | null
  entities?: readonly string[] | null
  filter?: JsonObject | null
  embedding?: readonly number[] | null
  lexicalWeight?: number | null
  semanticWeight?: number | null
  minSemanticScore?: number | null
  fusion?: 'blend' | 'rrf' | null
  candidateLimit?: number | null
  explain?: boolean
}
export type ExplainOptions = RecallOptions
export interface Remembered {
  id: string
  created: boolean
  supersedes: string[]
  supersededBy: string[]
  ingestTime: number
}
export interface ChainLink {
  id: string
  relation: 'predecessor' | 'successor'
  effectiveFrom: number
  ingestTime: number
}
export interface Explanation {
  retrieved: boolean
  rank?: number | null
  lexicalScore?: number | null
  semanticScore?: number | null
  retrievalScore?: number | null
  lexicalWeight: number
  semanticWeight: number
  fusion: string
  matchedTerms: string[]
  recencyFactor: number
  historicalFactor: number
  finalScore?: number | null
  status: FactStatus
  asOf: number
  knownAt?: number | null
  validFrom?: number | null
  validTo?: number | null
  withinValidity: boolean
  chain: ChainLink[]
}
export interface FactRecord {
  id: string
  text?: string | null
  status: FactStatus
  ingestTime?: number | null
  eventTime?: number | null
  validFrom?: number | null
  validTo?: number | null
  effectiveFrom: number
  subject?: string | null
  entities: string[]
  supersedes: string[]
  supersededBy: string[]
  metadata: JsonValue
}
export interface Recalled extends Omit<FactRecord, 'effectiveFrom'> {
  score: number
  sourceUri?: string | null
  explanation?: Explanation | null
}
export type ForgetScope =
  | { kind: 'ids'; ids: readonly string[] }
  | { kind: 'subject'; subject: string }
  | { kind: 'entity'; entity: string }
  | { kind: 'ingestedBefore' | 'eventBefore'; time: number }
  | { kind: 'matching'; filter: JsonObject }
export interface ForgetOptions extends RequestOptions {
  includeChain?: boolean
  dryRun?: boolean
  purge?: boolean
}
export interface ConsolidateOptions extends RequestOptions { dryRun?: boolean; allowDestructive?: boolean }
export interface ForgetReport { forgotten: string[]; dryRun: boolean; purged: boolean }
export interface ConsolidationReport { created: Remembered[]; forgotten: string[]; dryRun: boolean }
