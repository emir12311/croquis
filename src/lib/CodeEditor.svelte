<script lang="ts">
  import { onMount } from 'svelte'
  import { EditorView, keymap } from '@codemirror/view'
  import { indentWithTab } from '@codemirror/commands'
  import { acceptCompletion } from '@codemirror/autocomplete'
  import { EditorState } from '@codemirror/state'
  import { basicSetup } from 'codemirror'
  import { cpp } from '@codemirror/lang-cpp'
  import { arduinoCompletions } from './arduino-api'
  import { syntaxHighlighting, HighlightStyle } from '@codemirror/language'
  import { tags as t } from '@lezer/highlight'

  let { value = $bindable('') }: { value?: string } = $props()

  let host: HTMLDivElement
  let view: EditorView | null = null

  // Colours come from the app's own tokens, so the editor simply follows the
  // theme (dark/light, and any user palette override).
  const theme = EditorView.theme({
    '&': {
      height: '100%',
      backgroundColor: 'var(--bg)',
      color: 'var(--text)',
      fontSize: 'var(--fs-code)',
    },
    '.cm-scroller': {
      fontFamily: 'var(--font-mono)',
      lineHeight: 'var(--lh-code)',
    },
    '.cm-content': { padding: 'var(--space-2) 0', caretColor: 'var(--accent)' },
    '.cm-gutters': {
      backgroundColor: 'var(--bg)',
      color: 'var(--text-muted)',
      border: 'none',
      borderRight: '1px solid var(--border)',
    },
    '.cm-activeLine': { backgroundColor: 'color-mix(in srgb, var(--text) 7%, transparent)' },
    '.cm-activeLineGutter': { backgroundColor: 'transparent', color: 'var(--text)' },
    '&.cm-focused': { outline: 'none' },
    // The completion popup: our tokens, and a border rather than a shadow —
    // the app has no shadows, and an unstyled list would undo the rest.
    '.cm-tooltip': {
      backgroundColor: 'var(--surface-raised)',
      border: '1px solid var(--border)',
      borderRadius: 'var(--radius-panel)',
      color: 'var(--text)',
    },
    '.cm-tooltip.cm-tooltip-autocomplete > ul': {
      fontFamily: 'var(--font-mono)',
      fontSize: 'var(--fs-ui-dense)',
      maxHeight: '16em',
    },
    '.cm-tooltip.cm-tooltip-autocomplete > ul > li': { padding: '2px var(--space-2)' },
    '.cm-tooltip-autocomplete ul li[aria-selected]': {
      backgroundColor: 'color-mix(in srgb, var(--accent) 22%, transparent)',
      color: 'var(--text)',
    },
    '.cm-completionDetail': {
      color: 'var(--text-muted)',
      fontStyle: 'normal',
      marginLeft: 'var(--space-3)',
    },
    '.cm-completionMatchedText': { color: 'var(--accent)', textDecoration: 'none' },
    '.cm-completionInfo': {
      backgroundColor: 'var(--surface-raised)',
      border: '1px solid var(--border)',
      color: 'var(--text)',
      fontFamily: 'var(--font-ui)',
      padding: 'var(--space-2) var(--space-3)',
      maxWidth: '34ch',
    },
    '.cm-cursor, .cm-dropCursor': { borderLeftColor: 'var(--accent)' },
    '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection': {
      backgroundColor: 'color-mix(in srgb, var(--accent) 30%, transparent)',
    },
  })

  const highlight = HighlightStyle.define([
    { tag: t.keyword, color: 'var(--accent)' },
    { tag: [t.string, t.special(t.string)], color: 'var(--success)' },
    { tag: [t.number, t.bool, t.null], color: 'var(--info)' },
    {
      tag: [t.comment, t.lineComment, t.blockComment],
      color: 'var(--text-muted)',
      fontStyle: 'italic',
    },
    { tag: [t.typeName, t.className], color: 'var(--accent)' },
    { tag: [t.operator, t.punctuation], color: 'var(--text-muted)' },
    {
      tag: [t.definition(t.variableName), t.function(t.variableName), t.function(t.propertyName)],
      color: 'var(--text)',
    },
  ])

  // One LanguageSupport, so the Arduino completions can hang off its language —
  // which leaves the editor's own word completion in place, so the user's own
  // variable and function names complete too.
  const arduino = cpp()

  onMount(() => {
    view = new EditorView({
      parent: host,
      state: EditorState.create({
        doc: value,
        extensions: [
          basicSetup,
          arduino,
          // Tab between a snippet's placeholders comes free with autocompletion
          // (the package wires its own snippetKeymap), so there is nothing to add.
          arduino.language.data.of({ autocomplete: arduinoCompletions }),
          theme,
          syntaxHighlighting(highlight),
          // Tab has to be claimed explicitly: CodeMirror leaves it to the browser
          // so focus can escape the editor, which is why it was skipping to the
          // next element instead of indenting. Shift-Tab dedents, and Escape gives
          // focus back — the counterweight to trapping Tab.
          //   Tab accepts a completion when the list is open (acceptCompletion
          //   declines otherwise), and indents the rest of the time.
          //   Escape only reaches this binding when no completion is open, because
          //   basicSetup's keymaps have higher precedence.
          keymap.of([
            { key: 'Tab', run: acceptCompletion },
            indentWithTab,
            {
              key: 'Escape',
              run: (v) => {
                v.contentDOM.blur()
                return true
              },
            },
          ]),
          EditorView.updateListener.of((u) => {
            if (u.docChanged) value = u.state.doc.toString()
          }),
        ],
      }),
    })
    return () => {
      view?.destroy()
      view = null
    }
  })

  // Keep the editor in sync if `value` is set from outside.
  $effect(() => {
    if (!view) return
    const current = view.state.doc.toString()
    if (value !== current) {
      view.dispatch({ changes: { from: 0, to: current.length, insert: value } })
    }
  })
</script>

<div class="editor" bind:this={host}></div>

<style>
  .editor {
    height: 100%;
  }
</style>
