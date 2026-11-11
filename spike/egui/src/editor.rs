//! Native syntax highlighting and compiler diagnostics for the sketch panel.

use eframe::egui::Color32;

/// How bad a compiler message is. `avr-gcc` reports both in the same stream,
/// and they read differently: a warning still leaves a working sketch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// One `file:line:column: severity: message` line from the compiler.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub severity: Severity,
    pub message: String,
}

impl Diagnostic {
    /// Whether the caret can go here: only the sketch's own lines are in the
    /// editor, and a diagnostic from a bundled library has no place to land.
    pub fn in_sketch(&self) -> bool {
        self.file == SKETCH_FILE
    }
}

/// The name `engine::compile` gives the sketch, through `#line`, so the
/// compiler's own messages point at it by this name and not at a temp file.
const SKETCH_FILE: &str = "sketch.ino";

/// Every diagnostic in a compiler transcript, in the order the compiler wrote
/// them. Context lines (`In function 'void setup()':`) and linker complaints are
/// not diagnostics and are dropped; the raw text is still shown beside these.
pub fn parse_diagnostics(output: &str) -> Vec<Diagnostic> {
    output.lines().filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<Diagnostic> {
    let (file, rest) = line.split_once(':')?;
    let file = file.trim();
    // Only source files carry `line:column`. The `.ino` is the sketch; a `.cpp`
    // or `.h` is a bundled library included by it.
    if !(file.ends_with(".ino") || file.ends_with(".cpp") || file.ends_with(".h")) {
        return None;
    }
    let (number, rest) = rest.split_once(':')?;
    let line_number: usize = number.trim().parse().ok()?;
    let (number, rest) = rest.split_once(':')?;
    let column: usize = number.trim().parse().ok()?;
    let (severity, message) = rest.split_once(':')?;
    let severity = match severity.trim() {
        "error" | "fatal error" => Severity::Error,
        "warning" => Severity::Warning,
        _ => return None,
    };
    let message = message.trim();
    if message.is_empty() {
        return None;
    }
    Some(Diagnostic {
        file: file.to_string(),
        line: line_number,
        column,
        severity,
        message: message.to_string(),
    })
}

const KEYWORDS: &[&str] = &[
    "void", "int", "byte", "const", "if", "else", "for", "while", "return", "true", "false",
];

const FUNCTIONS: &[&str] = &[
    "analogRead",
    "analogWrite",
    "delay",
    "digitalRead",
    "digitalWrite",
    "pinMode",
    "serial.begin",
    "serial.println",
    "HIGH",
    "INPUT",
    "LOW",
    "OUTPUT",
];

/// The syntax `egui_code_editor` needs, assembled from the word lists this file
/// already uses to colour a sketch.
///
/// The crate ships Rust and Python presets and no C/C++, so an Arduino one is
/// built here — in the same place that decides what the highlighter treats as a
/// keyword, so the two cannot disagree about what a sketch looks like.
pub fn arduino_syntax() -> egui_code_editor::Syntax {
    use std::collections::BTreeSet;

    let keywords: BTreeSet<&'static str> = KEYWORDS
        .iter()
        .copied()
        .chain([
            "do", "switch", "case", "break", "continue", "static", "volatile", "HIGH", "LOW",
            "INPUT", "OUTPUT", "INPUT_PULLUP", "LED_BUILTIN",
        ])
        .collect();
    let types: BTreeSet<&'static str> = [
        "void", "int", "byte", "long", "float", "double", "char", "boolean", "bool", "unsigned",
        "word", "String", "size_t",
    ]
    .into_iter()
    .collect();
    let functions: BTreeSet<&'static str> = FUNCTIONS.iter().copied().collect();

    egui_code_editor::Syntax::new("C++")
        .with_case_sensitive(true)
        .with_comment("//")
        .with_comment_multiline(["/*", "*/"])
        .with_quotes(['"', '\''].into_iter().collect::<BTreeSet<char>>())
        .with_keywords(keywords)
        .with_types(types)
        .with_special(functions)
}

/// The widget's theme.
///
/// `CodeEditor::default()` draws in Gruvbox, which clashes with the shell's
/// palette. The crate ships eight themes; Github Dark is the closest to v0.2's
/// CodeMirror look. Picked by name rather than by index so a reordering upstream
/// cannot quietly change it, and built once because it parses eight hex colours.
pub fn code_theme() -> &'static egui_code_editor::Color32Theme {
    use std::sync::OnceLock;
    /// The colour the editor draws its background in.
    ///
    /// The theme's own background is near-black (`#0d1117`), which read as a
    /// different surface from the rest of the shell: a black slab inside a grey
    /// panel. This is the shell's own SURFACE, so the panel is one colour — and it
    /// is still dark enough for the theme's syntax colours to carry.
    const EDITOR_BG: Color32 = Color32::from_rgb(0x21, 0x21, 0x24);

    static THEME: OnceLock<egui_code_editor::Color32Theme> = OnceLock::new();
    THEME.get_or_init(|| {
        let chosen = egui_code_editor::DEFAULT_THEMES
            .iter()
            .find(|theme| theme.name() == "Github Dark")
            .copied()
            .unwrap_or(egui_code_editor::DEFAULT_THEMES[0]);
        let mut theme = egui_code_editor::Color32Theme::from(chosen);
        theme.bg = EDITOR_BG;
        theme
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arduino_syntax_knows_the_words_a_sketch_uses() {
        let syntax = arduino_syntax();
        assert_eq!(syntax.language(), "C++");
        assert_eq!(syntax.comment(), "//", "a sketch comments with //");
        for word in ["void", "int", "byte"] {
            assert!(syntax.is_type(word), "{word} should read as a type");
        }
        for word in ["if", "else", "while", "return"] {
            assert!(syntax.is_keyword(word), "{word} should be a keyword");
        }
        for word in ["pinMode", "digitalWrite", "delay"] {
            assert!(syntax.is_special(word), "{word} is part of the Arduino API");
        }
        assert!(
            !syntax.is_keyword("notAThing"),
            "and a word nobody listed is not a keyword"
        );
    }

    #[test]
    fn parses_the_compilers_errors_and_warnings() {
        // Real `avr-gcc` output: a context line, an error, a warning, and a
        // linker complaint that has a line number's shape but no column.
        let output = "\
sketch.ino: In function 'void setup()':
sketch.ino:3:3: error: 'pinModee' was not declared in this scope
sketch.ino:6:1: warning: unused variable 'x' [-Wunused-variable]
sketch.ino:(.text+0x1a): undefined reference to `foo()'
";
        let diagnostics = parse_diagnostics(output);
        assert_eq!(diagnostics.len(), 2, "got {diagnostics:?}");
        assert_eq!(diagnostics[0].line, 3);
        assert_eq!(diagnostics[0].column, 3);
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert_eq!(
            diagnostics[0].message,
            "'pinModee' was not declared in this scope"
        );
        assert!(diagnostics[0].in_sketch());
        assert_eq!(diagnostics[1].line, 6);
        assert_eq!(diagnostics[1].severity, Severity::Warning);
        assert_eq!(diagnostics[1].message, "unused variable 'x' [-Wunused-variable]");
    }

    #[test]
    fn a_library_diagnostic_has_no_caret_to_land_on() {
        let output = "/tmp/x/libraries/Servo/Servo.cpp:42:5: error: 'x' was not declared\n";
        let diagnostics = parse_diagnostics(output);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].line, 42);
        assert!(!diagnostics[0].in_sketch());
    }

    #[test]
    fn a_clean_compile_has_no_diagnostics() {
        assert!(parse_diagnostics("").is_empty());
        assert!(parse_diagnostics("sketch.ino: In function 'void loop()':").is_empty());
        assert!(
            parse_diagnostics("sketch.ino:3: note: some note").is_empty(),
            "a note is not a problem to show"
        );
    }

    #[test]
    fn a_fatal_error_is_an_error() {
        let output = "sketch.ino:1:10: fatal error: Servo.hh: No such file or directory\n";
        let diagnostics = parse_diagnostics(output);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Severity::Error);
    }

}
