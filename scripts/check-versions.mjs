#!/usr/bin/env node
/**
 * Every place that states the shipping app's version has to agree.
 *
 * The release that prompted this file: three manifests said 0.1.1, the lock file still
 * said 0.0.0, and the Arch recipe said 0.1.0 — so a source build fetched the previous
 * release while the README offered the new one.
 *
 * The app is `app`, so its `Cargo.toml` and the `croquis` entry in its
 * `Cargo.lock` are what this checks.
 *
 *   node scripts/check-versions.mjs          compare to the app's Cargo.toml
 *   node scripts/check-versions.mjs v0.3.0   compare to a tag
 *
 * Exits non-zero and names the file that disagrees.
 */
import { readFileSync } from 'node:fs'

const readText = (path) => readFileSync(path, 'utf8')

/** `version` in the app's manifest — the first one, which is the package's own. */
function appTomlVersion() {
  const m = /^version = "([^"]+)"/m.exec(readText('app/Cargo.toml'))
  return m?.[1]
}

/** Our own package in the app's lockfile — not a dependency that shares a name. */
function appLockVersion() {
  const block = /\[\[package\]\]\nname = "croquis"\nversion = "([^"]+)"/.exec(
    readText('app/Cargo.lock'),
  )
  return block?.[1]
}

const asked = (process.argv[2] ?? '').replace(/^v/, '')
const want = asked || appTomlVersion()
if (!want) {
  console.error('no version: pass a tag, or put one in app/Cargo.toml')
  process.exit(2)
}

const found = [
  ['app/Cargo.toml', appTomlVersion()],
  ['app/Cargo.lock', appLockVersion()],
]

const wrong = found.filter(([, got]) => got !== want)
if (wrong.length) {
  for (const [file, got] of wrong) {
    console.error(`${file}: ${got ?? 'missing'}, expected ${want}`)
  }
  process.exit(1)
}
console.log(`all ${found.length} version declarations say ${want}`)
