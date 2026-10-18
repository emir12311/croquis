import { describe, expect, it } from 'vitest'
import { FileSession, type FileBackend } from './file-session.svelte'

class FakeBackend implements FileBackend {
  files = new Map<string, string>()
  openPath: string | null = null
  savePath: string | null = null
  failRead = ''
  failWrite = ''
  savePicks: string[] = []
  defaultsAsked = 0

  async readText(path: string) {
    if (this.failRead) throw new Error(this.failRead)
    const text = this.files.get(path)
    if (text === undefined) throw new Error('missing')
    return text
  }

  async writeText(path: string, contents: string) {
    if (this.failWrite) throw new Error(this.failWrite)
    this.files.set(path, contents)
  }

  async pickOpen() {
    return this.openPath
  }

  async pickSave(defaultPath: string) {
    this.savePicks.push(defaultPath)
    return this.savePath
  }

  async defaultPath() {
    this.defaultsAsked++
    return '/home/me/diagram.json'
  }
}

describe('FileSession.save', () => {
  it('asks where a first save goes, offers documents, and adopts the path', async () => {
    const backend = new FakeBackend()
    backend.savePath = '/home/me/circuits/blink.json'
    const files = new FileSession(backend)

    expect(await files.save('{"version":1}')).toBe(true)
    expect(backend.defaultsAsked).toBe(1)
    expect(backend.savePicks).toEqual(['/home/me/diagram.json'])
    expect(backend.files.get('/home/me/circuits/blink.json')).toBe('{"version":1}')
    expect(files.savePath).toBe('/home/me/circuits/blink.json')
    expect(files.error).toBe('')
    expect(files.savedNote).toBe(true)
  })

  it('writes straight home on later saves, without asking again', async () => {
    const backend = new FakeBackend()
    backend.savePath = '/home/me/first.json'
    const files = new FileSession(backend)
    await files.save('one')
    backend.savePath = '/home/me/other.json'

    expect(await files.save('two')).toBe(true)
    expect(backend.savePicks).toHaveLength(1)
    expect(backend.files.get('/home/me/first.json')).toBe('two')
    expect(files.savePath).toBe('/home/me/first.json')
  })

  it('reports a failed write and keeps the old home', async () => {
    const backend = new FakeBackend()
    backend.savePath = '/home/me/first.json'
    const files = new FileSession(backend)
    await files.save('one')

    backend.failWrite = 'read-only file system'
    expect(await files.save('two')).toBe(false)
    expect(files.error).toContain('read-only file system')
    expect(files.savePath).toBe('/home/me/first.json')
    expect(backend.files.get('/home/me/first.json')).toBe('one')
  })

  it('does nothing when the picker is cancelled', async () => {
    const backend = new FakeBackend()
    const files = new FileSession(backend)
    expect(await files.save('x')).toBe(false)
    expect(files.savePath).toBeNull()
    expect(files.error).toBe('')
  })
})

describe('FileSession.open', () => {
  it('adopts the path only after the file was read and applied', async () => {
    const backend = new FakeBackend()
    backend.openPath = '/home/me/preset.json'
    backend.files.set('/home/me/preset.json', '{"parts":[]}')
    const files = new FileSession(backend)

    let loaded = ''
    expect(await files.open((text) => (loaded = text))).toBe(true)
    expect(loaded).toBe('{"parts":[]}')
    expect(files.savePath).toBe('/home/me/preset.json')
  })

  it('does not adopt a file the loader rejects', async () => {
    const backend = new FakeBackend()
    backend.openPath = '/home/me/broken.json'
    backend.files.set('/home/me/broken.json', 'not json')
    const files = new FileSession(backend)

    expect(
      await files.open(() => {
        throw new Error('not a circuit')
      }),
    ).toBe(false)
    expect(files.error).toContain('not a circuit')
    expect(files.savePath).toBeNull()
  })

  it('leaves the previous home alone when the read fails', async () => {
    const backend = new FakeBackend()
    backend.savePath = '/home/me/home.json'
    const files = new FileSession(backend)
    await files.save('home')

    backend.openPath = '/home/me/broken.json'
    backend.failRead = 'permission denied'
    expect(await files.open(() => {})).toBe(false)
    expect(files.error).toContain('permission denied')
    expect(files.savePath).toBe('/home/me/home.json')
  })

  it('does nothing when the picker is cancelled', async () => {
    const backend = new FakeBackend()
    const files = new FileSession(backend)
    expect(await files.open(() => {})).toBe(false)
    expect(files.savePath).toBeNull()
  })
})

describe('FileSession.reset', () => {
  it('forgets the home and any error, so the next save asks', async () => {
    const backend = new FakeBackend()
    backend.savePath = '/home/me/home.json'
    const files = new FileSession(backend)
    await files.save('one')
    backend.failWrite = 'nope'
    await files.save('two')
    expect(files.error).not.toBe('')

    files.reset()
    expect(files.savePath).toBeNull()
    expect(files.error).toBe('')
  })
})
