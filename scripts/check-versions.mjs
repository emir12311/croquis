#!/usr/bin/env node
/**
 * Every place that states the shipping app's version has to agree.
 *
 * The release that prompted this file: package.json, Cargo.toml and tauri.conf.json
 * said 0.1.1, package-lock.json still said 0.0.0, and packaging/arch/PKGBUILD said
 * 0.1.0 — so `makepkg` fetched the previous release while the README offered the
 * new one.
 *
 * What ships now is the egui app in `spike/egui`, so that is what this checks. The
 * Tauri app's files (`package.json`, `src-tauri/`) and its PKGBUILD keep their own
 * v0.2 version and are deliberately not compared to a v0.3 tag: they are no longer
 * released, and failing them against it would mean editing v0.2 to look like
 * something it is not.
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
  const m = /^version = "([^"]+)"/m.exec(readText('spike/egui/Cargo.toml'))
  return m?.[1]
}

/** Our own package in the app's lockfile — not a dependency that shares a name. */
function appLockVersion() {
  const block = /\[\[package\]\]\nname = "croquis"\nversion = "([^"]+)"/.exec(
    readText('spike/egui/Cargo.lock'),
  )
  return block?.[1]
}

const asked = (process.argv[2] ?? '').replace(/^v/, '')
const want = asked || appTomlVersion()
if (!want) {
  console.error('no version: pass a tag, or put one in spike/egui/Cargo.toml')
  process.exit(2)
}

const found = [
  ['spike/egui/Cargo.toml', appTomlVersion()],
  ['spike/egui/Cargo.lock', appLockVersion()],
]

const wrong = found.filter(([, got]) => got !== want)
if (wrong.length) {
  for (const [file, got] of wrong) {
    console.error(`${file}: ${got ?? 'missing'}, expected ${want}`)
  }
  process.exit(1)
}
console.log(`all ${found.length} version declarations say ${want}`)
