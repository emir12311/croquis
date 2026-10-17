// Register only the four component elements we need (each import defines its
// custom element). Importing the whole library would pull in every sensor and
// display, which works against the "stay lightweight" rule.
import '@wokwi/elements/dist/esm/arduino-uno-element'
import '@wokwi/elements/dist/esm/led-element'
import '@wokwi/elements/dist/esm/resistor-element'
import '@wokwi/elements/dist/esm/pushbutton-element'

import '@fontsource/ibm-plex-sans/400.css'
import '@fontsource/ibm-plex-sans/500.css'
import '@fontsource/ibm-plex-sans/600.css'
import '@fontsource/ibm-plex-mono/400.css'
import './app.css'
import { mount } from 'svelte'
import App from './App.svelte'

const app = mount(App, {
  target: document.getElementById('app')!,
})

export default app
