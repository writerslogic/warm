// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import { mkdtemp, mkdir, readFile, realpath, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, isAbsolute, join, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { promisify } from 'node:util'
import { cleanEnvironment } from './support.mjs'

const execute = promisify(execFile)
export const packageRoot = fileURLToPath(new URL('..', import.meta.url))

function noRuntimeDependencies(manifest) {
  for (const key of ['dependencies', 'optionalDependencies', 'peerDependencies']) {
    assert.deepEqual(Object.keys(manifest[key] ?? {}), [], `portable package has ${key}`)
  }
}

async function npm(args, options) {
  const cli = process.env.npm_execpath
  assert.ok(cli || process.platform !== 'win32', 'Run this test through npm test or npm run test:package on Windows')
  return execute(cli ? process.execPath : 'npm', cli ? [cli, ...args] : args, {
    timeout: 120000, maxBuffer: 1024 * 1024, ...options,
  })
}

export async function installConsumer(t, registrySpec) {
  const localManifest = JSON.parse(await readFile(join(packageRoot, 'package.json'), 'utf8'))
  assert.equal(localManifest.name, 'warm-sdk')
  noRuntimeDependencies(localManifest)
  if (registrySpec !== undefined) assert.equal(registrySpec, `warm-sdk@${localManifest.version}`, 'Registry verification must use the exact candidate version')
  const temporary = await mkdtemp(join(tmpdir(), 'warm-js-consumer-'))
  t.after(() => rm(temporary, { recursive: true, force: true }))
  const consumer = join(temporary, 'consumer')
  await mkdir(consumer)
  await writeFile(join(consumer, 'package.json'), JSON.stringify({ name: 'warm-sdk-external-consumer', version: '1.0.0', private: true }))
  const userConfig = join(temporary, 'user.npmrc')
  const globalConfig = join(temporary, 'global.npmrc')
  await writeFile(userConfig, '')
  await writeFile(globalConfig, '')
  const npmEnvironment = { ...cleanEnvironment(), npm_config_cache: join(temporary, 'npm-cache'),
    npm_config_userconfig: userConfig, npm_config_globalconfig: globalConfig,
    npm_config_registry: 'https://registry.npmjs.org/' }
  let source = registrySpec
  if (!source) {
    const packed = await npm(['pack', '--ignore-scripts', '--json', '--pack-destination', temporary], { cwd: packageRoot, env: npmEnvironment })
    const receipts = JSON.parse(packed.stdout)
    assert.equal(receipts.length, 1)
    const filename = receipts[0].filename
    assert.equal(typeof filename, 'string')
    source = resolve(temporary, filename)
    assert.equal(dirname(source), resolve(temporary), 'npm pack returned a path outside its destination')
    assert.ok(source.endsWith('.tgz'))
  }
  await npm(['install', '--ignore-scripts', '--save-exact', '--no-audit', '--no-fund', source], { cwd: consumer, env: npmEnvironment })
  if (registrySpec) await npm(['audit', 'signatures'], { cwd: consumer, env: npmEnvironment })
  const installedDirectory = await realpath(join(consumer, 'node_modules', 'warm-sdk'))
  const sourceRelative = relative(await realpath(packageRoot), installedDirectory)
  assert.ok(sourceRelative === '..' || sourceRelative.startsWith(`..${process.platform === 'win32' ? '\\' : '/'}`) || isAbsolute(sourceRelative), 'Consumer resolved the source package')
  const installedManifest = JSON.parse(await readFile(join(installedDirectory, 'package.json'), 'utf8'))
  assert.equal(installedManifest.name, 'warm-sdk')
  assert.equal(installedManifest.version, localManifest.version)
  noRuntimeDependencies(installedManifest)
  const lock = JSON.parse(await readFile(join(consumer, 'package-lock.json'), 'utf8'))
  assert.deepEqual(Object.keys(lock.packages).filter(path => path !== ''), ['node_modules/warm-sdk'])
  if (registrySpec) {
    const metadata = await npm(['view', registrySpec, 'dist.integrity', '--json'], { cwd: consumer, env: npmEnvironment })
    const integrity = JSON.parse(metadata.stdout)
    assert.match(integrity, /^sha512-[A-Za-z0-9+/]+={0,2}$/)
    assert.equal(lock.packages['node_modules/warm-sdk'].integrity, integrity)
    t.diagnostic(JSON.stringify({ package: registrySpec, integrity, signatures: 'verified' }))
  }
  return { consumer, installedDirectory }
}
