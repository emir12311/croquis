/**
 * The file lifecycle: where the circuit lives, save/save-as, open, and the
 * feedback that goes with them.
 *
 * The Tauri calls and the native dialogs arrive through `FileBackend`, so the
 * rules (a first save offers a default, a failed write is reported, an open only
 * adopts a path it actually read) are testable without the desktop shell.
 */

export interface FileBackend {
  readText(path: string): Promise<string>
  writeText(path: string, contents: string): Promise<void>
  /** The native open picker; null when the user cancelled. */
  pickOpen(): Promise<string | null>
  /** The native save picker; null when the user cancelled. */
  pickSave(defaultPath: string): Promise<string | null>
  /** Where a first Save should offer to put the file. */
  defaultPath(): Promise<string>
}

const NOTE_MS = 1400

export class FileSession {
  /** Where this circuit was last written — the home Save writes to. */
  savePath = $state<string | null>(null)
  /** A failed save or open, shown in the circuit panel. */
  error = $state('')
  /** A short-lived acknowledgement that the circuit reached the disk. */
  savedNote = $state(false)

  private noteTimer: ReturnType<typeof setTimeout> | null = null
  private backend: FileBackend

  constructor(backend: FileBackend) {
    this.backend = backend
  }

  /** A new circuit has no home, so its first Save asks. */
  reset() {
    this.savePath = null
    this.error = ''
  }

  /** Opening or loading something adopts its path; a failed open must not. */
  adopt(path: string) {
    this.savePath = path
    this.error = ''
  }

  isDirtyNoteVisible(): boolean {
    return this.savedNote
  }

  /**
   * Save straight to wherever the circuit lives. With no home yet, this is a
   * Save as — the picker is for choosing a location, not confirming one.
   */
  async save(contents: string): Promise<boolean> {
    if (!this.savePath) return this.saveAs(contents)
    return this.writeTo(this.savePath, contents)
  }

  /** Always asks, even when there is a home — this is how the home moves. */
  async saveAs(contents: string): Promise<boolean> {
    try {
      const fallback = this.savePath ?? (await this.backend.defaultPath())
      const path = await this.backend.pickSave(fallback)
      if (!path) return false
      return this.writeTo(path, contents)
    } catch (err) {
      // The picker can fail before a write is attempted.
      this.error = `Could not save the circuit: ${String(err)}`
      return false
    }
  }

  private async writeTo(path: string, contents: string): Promise<boolean> {
    try {
      await this.backend.writeText(path, contents)
    } catch (err) {
      // A failed write has to be visible: the alternative is the app looking like
      // it saved while the file on disk is untouched.
      this.error = `Could not save ${path}: ${String(err)}`
      return false
    }
    this.error = ''
    this.savePath = path
    this.noteSaved()
    return true
  }

  /**
   * Pick, read and apply a circuit. `load` parses and applies it, and may throw:
   * a file that cannot be loaded must not become the path the next Save writes over,
   * so the path is adopted only once `load` has returned.
   */
  async open(load: (text: string) => void): Promise<boolean> {
    try {
      const path = await this.backend.pickOpen()
      if (!path) return false
      const text = await this.backend.readText(path)
      load(text)
      this.error = ''
      this.savePath = path
      return true
    } catch (err) {
      this.error = `Could not open the circuit: ${String(err)}`
      return false
    }
  }

  private noteSaved() {
    this.savedNote = true
    if (this.noteTimer) clearTimeout(this.noteTimer)
    this.noteTimer = setTimeout(() => {
      this.savedNote = false
      this.noteTimer = null
    }, NOTE_MS)
  }
}
