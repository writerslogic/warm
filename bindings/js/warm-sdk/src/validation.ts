// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

import type { JsonValue } from './types.js'

export const MAX_BYTES = 16 * 1024 * 1024
export class JsonSizeError extends RangeError {}
const encoder = new TextEncoder()
export const byteLength = (value: string): number => encoder.encode(value).byteLength

export function string(value: unknown, name: string, max = MAX_BYTES, nonempty = false): string {
  if (typeof value !== 'string') throw new TypeError(`${name} must be a string`)
  for (let i = 0; i < value.length; i += 1) {
    const unit = value.charCodeAt(i)
    if (unit >= 0xd800 && unit <= 0xdbff) {
      const next = value.charCodeAt(++i)
      if (!(next >= 0xdc00 && next <= 0xdfff)) throw new TypeError(`${name} must be valid Unicode`)
    } else if (unit >= 0xdc00 && unit <= 0xdfff) throw new TypeError(`${name} must be valid Unicode`)
  }
  if (value.length > max || byteLength(value) > max || (nonempty && !value.trim())) {
    throw new RangeError(`${name} has an invalid byte length`)
  }
  return value
}

export function integer(value: unknown, name: string, min = 0, max = Number.MAX_SAFE_INTEGER): number {
  if (typeof value !== 'number' || !Number.isSafeInteger(value)) throw new TypeError(`${name} must be a safe integer`)
  if (value < min || value > max) throw new RangeError(`${name} is outside its supported range`)
  return value
}

export function number(value: unknown, name: string, min = -Infinity, max = Infinity): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) throw new TypeError(`${name} must be finite`)
  if (value < min || value > max) throw new RangeError(`${name} is outside its supported range`)
  return value
}

export function boolean(value: unknown, name: string): boolean {
  if (typeof value !== 'boolean') throw new TypeError(`${name} must be a boolean`)
  return value
}

export function record(value: unknown, name: string, keys?: readonly string[]): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) throw new TypeError(`${name} must be an object`)
  const prototype = Object.getPrototypeOf(value)
  if (prototype !== null && prototype !== Object.prototype) throw new TypeError(`${name} must be a plain object`)
  for (const key of Reflect.ownKeys(value)) {
    const descriptor = Object.getOwnPropertyDescriptor(value, key)!
    if (typeof key !== 'string' || !('value' in descriptor) || !descriptor.enumerable || (keys && !keys.includes(key))) {
      throw new TypeError(`${name} contains an unsupported property`)
    }
  }
  return value as Record<string, unknown>
}

export function array(value: unknown, name: string, max = 1_048_576, min = 0): unknown[] {
  if (!Array.isArray(value)) throw new TypeError(`${name} must be an array`)
  if (value.length < min || value.length > max) throw new RangeError(`${name} has an invalid length`)
  for (let i = 0; i < value.length; i += 1) {
    const descriptor = Object.getOwnPropertyDescriptor(value, String(i))
    if (!descriptor || !('value' in descriptor)) throw new TypeError(`${name} must not contain holes or accessors`)
  }
  if (Reflect.ownKeys(value).length !== value.length + 1) throw new TypeError(`${name} contains extra properties`)
  return value
}

export function id(value: unknown): string {
  const result = string(value, 'id', 256)
  if (!result.length) throw new RangeError('id must not be empty')
  if (/[\u0000-\u001f\u007f-\u009f]/.test(result) || result.startsWith('hms:')) throw new TypeError('id contains a reserved prefix or control character')
  return result
}

export function segment(value: unknown): string {
  const result = id(value)
  // WHATWG URL parsing normalizes even percent-encoded dot-only path segments.
  if (result === '.' || result === '..') throw new TypeError('Dot-only document IDs cannot be addressed through a URL path')
  return encodeURIComponent(result).replace(/[!'()*]/g, char => `%${char.charCodeAt(0).toString(16).toUpperCase()}`)
}

export function vector(value: unknown): number[] {
  const values = array(value, 'embedding', 1_048_576, 1).map(item => number(item, 'embedding value'))
  if (values.every(item => item === 0)) throw new RangeError('Embedding must not be all zeros')
  return values
}

export function choice(value: unknown, name: string, values: readonly string[]): string {
  if (typeof value !== 'string' || !values.includes(value)) throw new TypeError(`${name} has an unsupported value`)
  return value
}

export function json(value: unknown, limit = MAX_BYTES): JsonValue {
  const ancestors = new Set<object>()
  let bytes = 0
  const add = (size: number): void => {
    bytes += size
    if (bytes > limit) throw new JsonSizeError('JSON exceeds its byte limit')
  }
  const visit = (item: unknown, depth: number): JsonValue => {
    if (depth > 64) throw new RangeError('JSON nesting exceeds 64 levels')
    if (item === null || typeof item === 'boolean') { add(item === null ? 4 : item ? 4 : 5); return item }
    if (typeof item === 'number') { number(item, 'JSON number'); add(String(item).length); return item }
    if (typeof item === 'string') {
      string(item, 'JSON string', limit)
      add(byteLength(JSON.stringify(item)))
      return item
    }
    if (typeof item !== 'object') throw new TypeError('JSON contains an unsupported value')
    if (ancestors.has(item)) throw new TypeError('JSON must not contain cycles')
    ancestors.add(item)
    let result: JsonValue
    if (Array.isArray(item)) {
      const items = array(item, 'JSON array')
      add(2 + Math.max(0, items.length - 1))
      result = items.map(entry => visit(entry, depth + 1))
    } else {
      const source = record(item, 'JSON object')
      const entries = Object.entries(source)
      add(2 + Math.max(0, entries.length - 1))
      const target: Record<string, JsonValue> = Object.create(null)
      for (const [key, entry] of entries) {
        string(key, 'JSON key', limit)
        add(byteLength(JSON.stringify(key)) + 1)
        target[key] = visit(entry, depth + 1)
      }
      result = target
    }
    ancestors.delete(item)
    return result
  }
  return visit(value, 0)
}

export function metadata(value: unknown, reserved = false): JsonValue {
  const object = record(value, 'metadata')
  if (reserved && Object.hasOwn(object, '_mem')) throw new TypeError('metadata._mem is reserved')
  return json(object, 64 * 1024)
}

export function signal(value: unknown): AbortSignal | undefined {
  if (value === undefined) return undefined
  if (value === null || typeof value !== 'object' || typeof (value as AbortSignal).aborted !== 'boolean' ||
      typeof (value as AbortSignal).addEventListener !== 'function' || typeof (value as AbortSignal).removeEventListener !== 'function') {
    throw new TypeError('signal must be an AbortSignal')
  }
  return value as AbortSignal
}

export function requestSignal(options: unknown): AbortSignal | undefined {
  return signal(record(options, 'request options', ['signal']).signal)
}
