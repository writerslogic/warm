// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

// Usage: node search-init.mjs <site-root>
import assert from 'node:assert/strict'
import { chromium } from 'playwright'
import { serve } from './serve.mjs'

const [root] = process.argv.slice(2)
assert.ok(root, 'Usage: node search-init.mjs <site-root>')
const port = Number(process.env.PORT ?? 8123)
const prefix = process.env.PREFIX ?? ''
const version = process.env.VERSION ?? 'dev'
const timeout = 10000
const indexPath = `${prefix}/${version}/search/search_index.json`
const indexRequest = request => new URL(request.url()).pathname === indexPath
const server = await serve(root, port, prefix)
const home = `http://127.0.0.1:${server.port}${prefix}/${version}/`
let browser
let checked = 0

async function scenario(name, options, check) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, ...options })
  let release
  const gate = new Promise(resolve => { release = resolve })
  let held = false
  let requests = 0
  let responses = 0
  const errors = []
  try {
    const page = await context.newPage()
    page.setDefaultTimeout(timeout)
    page.setDefaultNavigationTimeout(15000)
    page.on('pageerror', error => errors.push(error.message))
    page.on('request', request => { if (indexRequest(request)) requests += 1 })
    page.on('response', response => { if (indexRequest(response)) responses += 1 })
    await context.route(url => url.pathname === indexPath, async route => {
      if (held) await gate
      await route.continue()
    })
    await check({
      page,
      hold: () => { held = true },
      release,
      assertRequests: expected => assert.equal(requests, expected, `${name}: index request count`),
      assertHeld: () => assert.equal(responses, 0, `${name}: index responded before gate release`),
    })
    assert.deepEqual(errors, [], `${name}: browser page errors`)
    checked += 1
  } catch (error) {
    throw new Error(`${name}: ${error.message}`, { cause: error })
  } finally {
    release()
    try {
      await context.unrouteAll({ behavior: 'wait' })
    } finally {
      await context.close()
    }
  }
}

async function cold(state) {
  await state.page.goto(home, { waitUntil: 'networkidle' })
  state.assertRequests(0)
}

async function result(page, expected) {
  await page.waitForFunction(path => {
    const hits = document.querySelectorAll('.md-search-result__item > a, .md-search-result__item a.md-search-result__link')
    const pages = [...new Set([...hits].map(a => new URL(a.href).pathname))].slice(0, 5)
    return pages.some(value => value.endsWith(path))
  }, expected, { timeout })
}

async function releaseIndex(state) {
  state.assertHeld()
  const [response] = await Promise.all([
    state.page.waitForResponse(indexRequest, { timeout }),
    Promise.resolve().then(state.release),
  ])
  assert.equal(response.status(), 200, 'Search index response status')
}

async function activateAndType(state, activate) {
  await cold(state)
  state.hold()
  await Promise.all([
    state.page.waitForRequest(indexRequest, { timeout }),
    activate(state.page),
  ])
  await state.page.waitForFunction(() => document.activeElement?.matches('input.md-search__input'), null, { timeout })
  const box = state.page.locator('input.md-search__input')
  await box.fill('holographic')
  await box.press('End')
  await box.fill('quickstart')
  await box.press('End')
  state.assertHeld()
  assert.equal(await box.inputValue(), 'quickstart')
  await releaseIndex(state)
  await result(state.page, '/quickstart/')
  assert.equal(await box.inputValue(), 'quickstart', 'Query changed while index loaded')
  state.assertRequests(1)
}

async function query(page, text, expected) {
  const box = page.locator('input.md-search__input')
  await box.fill('')
  await box.press('End')
  await page.waitForFunction(() => /Type to start searching/i.test(document.querySelector('.md-search-result__meta')?.textContent ?? ''), null, { timeout })
  await box.fill(text)
  await box.press('End')
  await result(page, expected)
}

async function highlighted(state) {
  await state.page.waitForFunction(() => [...document.querySelectorAll('article.md-content__inner mark')]
    .some(mark => /holograph/i.test(mark.textContent)), null, { timeout })
  assert.equal(await state.page.locator('#__search').isChecked(), false, 'Highlighting opened search')
  state.assertRequests(1)
}

async function reopenAndNavigate(state) {
  const { page } = state
  await page.keyboard.press('Escape')
  await page.waitForFunction(() => !document.querySelector('#__search').checked, null, { timeout })
  await page.keyboard.press('/')
  await page.waitForFunction(() => document.activeElement?.matches('input.md-search__input'), null, { timeout })
  await query(page, 'quickstart', '/quickstart/')
  state.assertRequests(1)

  await page.keyboard.press('Escape')
  await page.waitForFunction(() => !document.querySelector('#__search').checked, null, { timeout })
  await page.evaluate(() => { window.__warmSearchInitDocument = document })
  const conceptsPath = `${prefix}/${version}/concepts/holographic/`
  await page.locator('a.warm-btn[href$="concepts/holographic/"]').click()
  await page.waitForFunction(path => location.pathname === path && !document.querySelector('.warm-home') && document.querySelector('article.md-content__inner h1'), conceptsPath, { timeout })
  assert.equal(await page.evaluate(() => window.__warmSearchInitDocument === document), true, 'Instant navigation replaced the document')
  await page.keyboard.press('s')
  await page.waitForFunction(() => document.activeElement?.matches('input.md-search__input'), null, { timeout })
  await query(page, 'holographic', '/concepts/holographic/')
  state.assertRequests(1)
}

try {
  browser = await chromium.launch()
  await scenario('desktop click, reopen and instant navigation', {}, async state => {
    await activateAndType(state, page => page.locator('input.md-search__input').click())
    await reopenAndNavigate(state)
  })
  for (const key of ['/', 's', 'f']) {
    await scenario(`desktop shortcut ${key}`, {}, state => activateAndType(state, page => page.keyboard.press(key)))
  }
  await scenario('phone header search', { viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true }, state => activateAndType(state, page => page.locator('header label.md-header__button[for="__search"]').click()))
  await scenario('query deep link', {}, async state => {
    state.hold()
    await Promise.all([
      state.page.waitForRequest(indexRequest, { timeout }),
      state.page.goto(`${home}?q=quickstart`, { waitUntil: 'domcontentloaded' }),
    ])
    assert.equal(await state.page.locator('input.md-search__input').inputValue(), 'quickstart')
    await releaseIndex(state)
    await result(state.page, '/quickstart/')
    state.assertRequests(1)
  })
  await scenario('highlight deep link without search', {}, async state => {
    state.hold()
    await Promise.all([
      state.page.waitForRequest(indexRequest, { timeout }),
      state.page.goto(`${home}concepts/holographic/?h=holographic`, { waitUntil: 'domcontentloaded' }),
    ])
    await releaseIndex(state)
    await highlighted(state)
  })
  await scenario('instant navigation to highlight before search', {}, async state => {
    await cold(state)
    await state.page.evaluate(() => { window.__warmSearchInitDocument = document })
    const link = state.page.locator('a.warm-btn[href*="concepts/holographic/"]')
    await link.evaluate(anchor => { anchor.search = '?h=holographic' })
    state.hold()
    await Promise.all([
      state.page.waitForRequest(indexRequest, { timeout }),
      link.click(),
    ])
    await releaseIndex(state)
    await highlighted(state)
    assert.equal(await state.page.evaluate(() => window.__warmSearchInitDocument === document), true, 'Highlighted navigation replaced the document')
  })
} finally {
  try {
    if (browser) await browser.close()
  } finally {
    await server.close()
  }
}
console.log(`search initialization: ${checked} scenarios passed`)
