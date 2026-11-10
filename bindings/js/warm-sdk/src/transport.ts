// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

import type { WarmClientOptions } from './types.js'
import { byteLength, integer, json, JsonSizeError, MAX_BYTES, record, string } from './validation.js'

export class WarmError extends Error {
  readonly code: string
  readonly status: number | undefined
  readonly statusCode: number | undefined
  readonly requestId: string | undefined

  constructor(message: string, code: string, status?: number, requestId?: string) {
    super(message)
    this.name = 'WarmError'
    this.code = code
    this.status = status
    this.statusCode = status
    this.requestId = requestId
  }
}

export class Transport {
  readonly url: string
  #fetch: typeof globalThis.fetch
  #headers: Headers
  #timeout: number
  #secrets: string[]

  constructor(options: WarmClientOptions) {
    const config = record(options, 'client options', ['url', 'apiKey', 'tenantId', 'timeoutMs', 'fetch'])
    const supplied = config.url === undefined ? 'http://localhost:8080' : string(config.url, 'url', 8192, true)
    let parsed: URL
    try { parsed = new URL(supplied) } catch { throw new TypeError('url must be an absolute HTTP(S) URL') }
    if (!/^https?:\/\//i.test(supplied) || !['http:', 'https:'].includes(parsed.protocol) || parsed.username || parsed.password ||
        supplied.slice(supplied.indexOf('//') + 2).split('/')[0]!.includes('@') ||
        supplied.includes('?') || supplied.includes('#') || /[\u0000-\u0020\u007f\\]/.test(supplied)) {
      throw new TypeError('url must be HTTP(S) without userinfo, a query, fragment, whitespace or backslash')
    }
    this.url = parsed.href.replace(/\/+$/, '')
    this.#headers = new Headers({ Accept: 'application/json', 'Content-Type': 'application/json' })
    this.#secrets = []
    if (config.apiKey !== undefined) {
      const key = string(config.apiKey, 'apiKey', 8192, true)
      if (!/^[\x21-\x7e]+$/.test(key)) throw new TypeError('apiKey must contain visible ASCII without whitespace')
      this.#headers.set('Authorization', `Bearer ${key}`)
      this.#secrets = [key, encodeURIComponent(key)]
    }
    if (config.tenantId !== undefined) {
      const tenant = string(config.tenantId, 'tenantId', 64, true)
      if (!/^[A-Za-z0-9_-]+$/.test(tenant)) throw new TypeError('tenantId must contain only letters, digits, underscores or hyphens')
      this.#headers.set('X-Tenant-ID', tenant)
    }
    this.#timeout = config.timeoutMs === undefined ? 30000 : integer(config.timeoutMs, 'timeoutMs', 1, 2_147_483_647)
    const implementation = config.fetch === undefined ? globalThis.fetch : config.fetch
    if (typeof implementation !== 'function') throw new TypeError('A Fetch API implementation is required')
    const fetcher = implementation as typeof globalThis.fetch
    this.#fetch = config.fetch === undefined ? fetcher.bind(globalThis) : fetcher
  }

  #safe(value: unknown, fallback: string, max: number): string {
    if (typeof value !== 'string' || /[<>]/.test(value)) return fallback
    let result = value
    for (const secret of this.#secrets) result = result.split(secret).join('[redacted]')
    return result.replace(/Bearer\s+[^\s,;]+/gi, 'Bearer [redacted]').replace(/[\u0000-\u001f\u007f-\u009f]/g, ' ').slice(0, max) || fallback
  }

  #error(response: Response, body: unknown): WarmError {
    const value = body !== null && typeof body === 'object' && !Array.isArray(body) ? body as Record<string, unknown> : {}
    const legacy = value.error !== null && typeof value.error === 'object' && !Array.isArray(value.error) ? value.error as Record<string, unknown> : {}
    const rawCode = this.#safe(value.code ?? legacy.code, 'http_error', 64)
    const code = /^[A-Za-z0-9_.:-]+$/.test(rawCode) ? rawCode : 'http_error'
    const message = this.#safe(value.detail ?? legacy.message, `WARM request failed (HTTP ${response.status})`, 1024)
    const identifier = this.#safe(response.headers.get('x-request-id') ?? value.request_id, '', 128)
    return new WarmError(message, code, response.status, /^[A-Za-z0-9_.:-]+$/.test(identifier) ? identifier : undefined)
  }

  async request<T>(method: string, path: string, payload: unknown, signal: AbortSignal | undefined, decode: (value: unknown) => T): Promise<T> {
    let body: string | undefined
    if (payload !== undefined) {
      try { body = JSON.stringify(json(payload)) } catch (error) {
        if (error instanceof JsonSizeError) throw new WarmError('Request exceeds 16 MiB', 'request_too_large')
        throw error
      }
      if (byteLength(body) > MAX_BYTES) throw new WarmError('Request exceeds 16 MiB', 'request_too_large')
    }
    if (signal?.aborted) throw new WarmError('Request was aborted', 'aborted')
    const controller = new AbortController()
    let timedOut = false
    let rejectAbort!: (error: WarmError) => void
    const aborted = new Promise<never>((_, reject) => { rejectAbort = reject })
    const abort = (): void => {
      controller.abort()
      rejectAbort(new WarmError(timedOut ? 'Request timed out' : 'Request was aborted', timedOut ? 'timeout' : 'aborted'))
    }
    signal?.addEventListener('abort', abort, { once: true })
    const timer = setTimeout(() => { timedOut = true; abort() }, this.#timeout)
    let response: Response | undefined
    let reader: ReadableStreamDefaultReader<Uint8Array> | undefined
    let finished = false
    try {
      if (signal?.aborted) abort()
      const fetched = Promise.resolve().then(() => {
        if (controller.signal.aborted) throw new WarmError('Request was aborted', 'aborted')
        return this.#fetch(`${this.url}${path}`, {
          method, headers: new Headers(this.#headers), body, signal: controller.signal,
          redirect: 'manual', credentials: 'omit', referrerPolicy: 'no-referrer',
        })
      }).then(value => {
        if (controller.signal.aborted) void value.body?.cancel().catch(() => {})
        return value
      })
      response = await Promise.race([fetched, aborted])
      if (response.type === 'opaqueredirect' || response.redirected || (response.status >= 300 && response.status < 400)) {
        throw new WarmError('WARM redirects are not followed', 'redirect_error', response.status || undefined)
      }
      const declared = response.headers.get('content-length')
      if (declared && /^\d+$/.test(declared) && Number(declared) > MAX_BYTES) throw new WarmError('Response exceeds 16 MiB', 'response_too_large', response.status)
      const chunks: Uint8Array[] = []
      let length = 0
      if (response.body) {
        reader = response.body.getReader()
        for (;;) {
          const chunk = await Promise.race([reader.read(), aborted])
          if (chunk.done) break
          length += chunk.value.byteLength
          if (length > MAX_BYTES) throw new WarmError('Response exceeds 16 MiB', 'response_too_large', response.status)
          chunks.push(chunk.value)
        }
      }
      finished = true
      const bytes = new Uint8Array(length)
      let offset = 0
      for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.byteLength }
      let parsed: unknown
      try { parsed = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes)) } catch {
        if (!response.ok) throw this.#error(response, undefined)
        throw new WarmError('WARM returned an invalid JSON response', 'invalid_response', response.status)
      }
      if (!response.ok) throw this.#error(response, parsed)
      try { json(parsed); return decode(parsed) } catch {
        throw new WarmError('WARM returned a response outside the documented contract', 'invalid_response', response.status)
      }
    } catch (error) {
      if (error instanceof WarmError) throw error
      if (controller.signal.aborted) throw new WarmError(timedOut ? 'Request timed out' : 'Request was aborted', timedOut ? 'timeout' : 'aborted')
      throw new WarmError('Unable to complete the WARM request', 'network_error')
    } finally {
      clearTimeout(timer)
      signal?.removeEventListener('abort', abort)
      if (!finished) {
        if (reader) void reader.cancel().catch(() => {})
        else if (response?.body) void response.body.cancel().catch(() => {})
      }
      reader?.releaseLock()
    }
  }
}
