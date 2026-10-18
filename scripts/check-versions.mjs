#!/usr/bin/env node
/**
 * Every place this project writes its version has to agree.
 *
 * The release that prompted this: package.json, Cargo.toml and tauri.conf.json said
 * 0.1.1, package-lock.json still said 0.0.0, and packaging/arch/PKGBUILD said 0.1.0 —
 * so `makepkg` fetched the previous release while the README offered the new one.
 *
 *   node scripts/check-versions.mjs          compare everything to package.json
 *   node scripts/check-versions.mjs v0.1.1   compare everything to a tag
 *
 * Exits non-zero and names the file that disagrees.
 */
import { readFileSync } from 'node:fs'

const readJson = (path) => JSON.parse(readFileSync(path, 'utf8'))
const readText = (path) => readFileSync(path, 'utf8')

const asked = (process.argv[2] ?? '').replace(/^v/, '')
const want = asked || readJson('package.json').version
if (!want) {
  console.error('no version: pass a tag, or put one in package.json')
  process.exit(2)
}

/** The `version` of our own package in Cargo.lock — not a dependency's. */
function cargoLockVersion() {
  const block = /\[\[package\]\]\nname = "croquis"\nversion = "([^"]+)"/.exec(readText('src-tauri/Cargo.lock'))
  return block?.[1]
}

function cargoTomlVersion() {
  const m = /^version = "([^"]+)"/m.exec(readText('src-tauri/Cargo.toml'))
  return m?.[1]
}

function pkgbuildVersion() {
  const m = /^pkgver=(\S+)/m.exec(readText('packaging/arch/PKGBUILD'))
  return m?.[1]
}

const pkg = readJson('package.json')
const conf = readJson('src-tauri/tauri.conf.json')
const lock = readJson('package-lock.json')

const found = [
  ['package.json', pkg.version],
  ['package-lock.json', lock.version],
  ['package-lock.json (root package)', lock.packages?.['']?.version],
  ['src-tauri/tauri.conf.json', conf.version],
  ['src-tauri/Cargo.toml', cargoTomlVersion()],
  ['src-tauri/Cargo.lock', cargoLockVersion()],
  ['packaging/arch/PKGBUILD', pkgbuildVersion()],
]

const wrong = found.filter(([, got]) => got !== want)
if (wrong.length) {
  for (const [file, got] of wrong) {
    console.error(`${file}: ${got ?? 'missing'}, expected ${want}`)
  }
  process.exit(1)
}
console.log(`all ${found.length} version declarations say ${want}`)
