//! Native syntax highlighting and compiler diagnostics for the sketch panel.

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{Color32, FontId};

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

/// Map a 1-based line and column from a diagnostic to a character index in the
/// sketch. A column past the end of its line stops at that line's end, and a
/// line past the end of the sketch at the end of the sketch. Character, not
/// byte: the sketch is UTF-8, and a comment in Turkish is enough to make the
/// two differ.
pub fn line_col_to_char_index(text: &str, line: usize, column: usize) -> usize {
    let line = line.max(1);
    let column = column.max(1);
    let mut current_line = 1;
    let mut line_byte = 0;
    for (byte, character) in text.char_indices() {
        if current_line == line {
            break;
        }
        if character == '\n' {
            current_line += 1;
            line_byte = byte + character.len_utf8();
        }
    }
    if current_line < line {
        return text.chars().count();
    }
    let line_end = text[line_byte..]
        .find('\n')
        .map_or(text.len(), |offset| line_byte + offset);
    let line_chars = text[line_byte..line_end].chars().count();
    text[..line_byte].chars().count() + column.saturating_sub(1).min(line_chars)
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

pub fn highlight(text: &str) -> LayoutJob {
    let mono = FontId::monospace(13.0);
    let mut job = LayoutJob::default();
    for line in text.split_inclusive('\n') {
        let comment = line.find("//");
        let (code, tail) = match comment {
            Some(index) => (&line[..index], Some(&line[index..])),
            None => (line, None),
        };
        append_tokens(&mut job, code, &mono);
        if let Some(comment) = tail {
            job.append(
                comment,
                0.0,
                TextFormat {
                    font_id: mono.clone(),
                    color: Color32::from_rgb(112, 128, 112),
                    ..Default::default()
                },
            );
        }
    }
    job
}

fn append_tokens(job: &mut LayoutJob, text: &str, mono: &FontId) {
    let mut word = String::new();
    for character in text.chars() {
        if character.is_alphanumeric() || character == '_' {
            word.push(character);
        } else {
            flush_word(job, &word, mono);
            word.clear();
            job.append(
                &character.to_string(),
                0.0,
                TextFormat {
                    font_id: mono.clone(),
                    color: Color32::from_rgb(204, 207, 204),
                    ..Default::default()
                },
            );
        }
    }
    flush_word(job, &word, mono);
}

fn flush_word(job: &mut LayoutJob, word: &str, mono: &FontId) {
    if word.is_empty() {
        return;
    }
    let color = if KEYWORDS.contains(&word) {
        Color32::from_rgb(198, 120, 221)
    } else if word.chars().all(|character| character.is_ascii_digit()) {
        Color32::from_rgb(209, 154, 102)
    } else if FUNCTIONS.contains(&word) {
        Color32::from_rgb(97, 175, 239)
    } else {
        Color32::from_rgb(204, 207, 204)
    };
    job.append(
        word,
        0.0,
        TextFormat {
            font_id: mono.clone(),
            color,
            ..Default::default()
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlighting_preserves_the_sketch() {
        let source = "void setup() {\n  // note\n  pinMode(13, OUTPUT);\n}\n";
        let job = highlight(source);
        assert_eq!(job.text, source);
        assert!(!job.sections.is_empty());
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

    #[test]
    fn diagnostics_map_onto_the_sketch_by_character() {
        // A multibyte comment is enough to make a byte index and a character
        // index disagree, and the caret is placed by character.
        let sketch = "// bir yorum: ş\nvoid setup() {\n  pinModee(13, OUTPUT);\n}\n";
        let diagnostic =
            &parse_diagnostics("sketch.ino:3:3: error: 'pinModee' was not declared in this scope\n")
                [0];
        let index = line_col_to_char_index(sketch, diagnostic.line, diagnostic.column);
        let expected = "// bir yorum: ş\nvoid setup() {\n".chars().count() + 2;
        assert_eq!(index, expected);
        assert!(sketch.chars().skip(index).collect::<String>().starts_with("pinModee"));
    }

    #[test]
    fn a_jump_past_the_end_lands_at_the_end() {
        let text = "one\ntwo\nthree";
        assert_eq!(line_col_to_char_index(text, 1, 1), 0);
        assert_eq!(line_col_to_char_index(text, 2, 1), 4);
        assert_eq!(line_col_to_char_index(text, 2, 3), 6);
        assert_eq!(line_col_to_char_index(text, 99, 1), text.chars().count());
        // A column past the end of a line stops at that line's end, not at the
        // end of the sketch: that is where the compiler meant.
        assert_eq!(line_col_to_char_index(text, 2, 999), 7);
        assert_eq!(line_col_to_char_index("a\n", 2, 99), 2);
        assert_eq!(line_col_to_char_index("", 1, 1), 0);
    }
}
