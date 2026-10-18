/**
 * The sketch being edited, and whether it differs from what was last saved.
 *
 * Separate from the circuit document on purpose: typing is watched by the editor,
 * and circuit edits by `Project`, so "unsaved changes" can mean either.
 */

export const DEFAULT_SKETCH = `void setup() {
  // the built-in LED is on pin 13
  pinMode(LED_BUILTIN, OUTPUT);
  Serial.begin(9600);
  Serial.println("croquis ready");
}

void loop() {
  digitalWrite(LED_BUILTIN, HIGH);
  delay(500);
  digitalWrite(LED_BUILTIN, LOW);
  delay(500);
  Serial.println("blink");
}
`

export class Sketch {
  text = $state(DEFAULT_SKETCH)
  /** The text as it was when this project was loaded or last saved. */
  synced = $state(DEFAULT_SKETCH)

  dirty = $derived(this.text !== this.synced)

  reset() {
    this.text = DEFAULT_SKETCH
    this.synced = DEFAULT_SKETCH
  }

  /** Adopt a loaded sketch. A file without one leaves the current text in place. */
  load(text: string | null) {
    if (text !== null) this.text = text
    this.synced = this.text
  }

  markSaved() {
    this.synced = this.text
  }
}
