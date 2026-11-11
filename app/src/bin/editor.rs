//! Spike B: a native egui code editor.
//!
//! Proves the editor risks without involving the production app: syntax
//! colouring through TextEdit's layouter, a completion list at the caret, and
//! diagnostic-to-line navigation. The pure mapping/filtering functions are
//! tested without a display.
//!
//! Run with:
//!
//! ```text
//! cargo run --manifest-path app/Cargo.toml --bin editor-spike
//! ```

use eframe::egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use eframe::egui::{self, Color32, FontId, Id};

const COMPLETIONS: &[&str] = &[
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

const KEYWORDS: &[&str] = &[
    "void", "int", "byte", "const", "if", "else", "for", "while", "return", "true", "false",
];

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 820.0])
            .with_min_inner_size([800.0, 500.0])
            .with_title("Croquis — egui editor spike"),
        ..Default::default()
    };
    eframe::run_native(
        "croquis-egui-editor-spike",
        options,
        Box::new(|_cc| Ok(Box::new(EditorSpike::default()))),
    )
}

struct EditorSpike {
    text: String,
    pending_cursor: Option<usize>,
    pending_jump: Option<(usize, usize)>,
}

impl Default for EditorSpike {
    fn default() -> Self {
        let mut text = String::from(
            "void setup() {\n  pinMode(13, OUTPUT);\n  Serial.begin(9600);\n}\n\nvoid loop() {\n  digitalWrite(13, HIGH);\n  delay(500);\n  digitalWrite(13, LOW);\n  delay(500);\n}\n",
        );
        for i in 0..40 {
            text.push_str(&format!("// filler line {i}\n"));
        }
        Self {
            text,
            pending_cursor: None,
            pending_jump: None,
        }
    }
}

impl eframe::App for EditorSpike {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();

        egui::Panel::right("editor-spike-info")
            .exact_size(280.0)
            .show(root, |ui| {
                ui.heading("egui editor spike");
                ui.label("TextEdit + syntax layouter + completion + diagnostic jump.");
                ui.separator();
                ui.label("Diagnostic");
                ui.monospace("sketch.ino:2:3: example error");
                if ui.button("Jump to line 2, column 3").clicked() {
                    self.pending_jump = Some((2, 3));
                }
                if ui.button("Go to end").clicked() {
                    self.pending_cursor = Some(self.text.chars().count());
                }
                ui.separator();
                ui.label("Try typing 'pin' or 'Serial.' and pick a completion.");
                ui.label("Ctrl/Cmd+Enter is left to the future app; this spike uses TextEdit's native undo/selection.");
            });

        egui::CentralPanel::default().show(root, |ui| {
            let editor_id = Id::unique("spike-code-editor");
            let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
                let mut job = highlight(text.as_str());
                job.wrap.max_width = wrap_width;
                ui.fonts_mut(|fonts| fonts.layout_job(job))
            };

            let mut output = None;
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    output = Some(
                        egui::TextEdit::multiline(&mut self.text)
                            .id(editor_id)
                            .code_editor()
                            .desired_width(f32::INFINITY)
                            .layouter(&mut layouter)
                            .show(ui),
                    );
                });

            let Some(output) = output else {
                return;
            };

            let index = if let Some((line, column)) = self.pending_jump.take() {
                Some(line_col_to_char_index(&self.text, line, column))
            } else {
                self.pending_cursor.take()
            };
            if let Some(index) = index {
                let mut state = output.state.clone();
                state
                    .cursor
                    .set_char_range(Some(CCursorRange::one(CCursor::new(index))));
                state.store(&ctx, output.response.id);
                output.response.request_focus();
            }

            if let Some(cursor) = output.cursor_range.and_then(|range| range.single()) {
                let index = usize::from(cursor.index);
                if let Some((start, prefix)) = word_prefix(&self.text, index) {
                    let items = filter_completions(&prefix);
                    if !items.is_empty() && prefix.chars().count() >= 2 {
                        let caret = output.galley.pos_from_cursor(cursor);
                        let pos = output.galley_pos + caret.left_bottom().to_vec2();
                        let mut chosen = None;
                        egui::Area::new(Id::unique("completion-list"))
                            .fixed_pos(pos)
                            .show(&ctx, |ui| {
                                egui::Frame::popup(ui.style()).show(ui, |ui| {
                                    ui.set_min_width(190.0);
                                    for item in items.iter().take(8) {
                                        if ui.button(*item).clicked() {
                                            chosen = Some(*item);
                                        }
                                    }
                                });
                            });
                        if let Some(item) = chosen {
                            let byte_start = char_to_byte(&self.text, start);
                            let byte_end = char_to_byte(&self.text, index);
                            self.text.replace_range(byte_start..byte_end, item);
                            self.pending_cursor = Some(start + item.chars().count());
                            ui.ctx().request_repaint();
                        }
                    }
                }
            }
        });
    }
}

/// Map a 1-based line and column to a character index, clamped to the text.
fn line_col_to_char_index(text: &str, line: usize, column: usize) -> usize {
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

fn char_to_byte(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(byte, _)| byte)
}

/// The word under the caret, including dots, and where it starts.
fn word_prefix(text: &str, char_index: usize) -> Option<(usize, String)> {
    let chars: Vec<char> = text.chars().collect();
    let end = char_index.min(chars.len());
    let mut start = end;
    while start > 0
        && (chars[start - 1].is_alphanumeric()
            || chars[start - 1] == '_'
            || chars[start - 1] == '.')
    {
        start -= 1;
    }
    if start == end {
        return None;
    }
    Some((start, chars[start..end].iter().collect()))
}

fn filter_completions(prefix: &str) -> Vec<&'static str> {
    let lowered = prefix.to_ascii_lowercase();
    COMPLETIONS
        .iter()
        .copied()
        .filter(|item| item.to_ascii_lowercase().starts_with(&lowered))
        .collect()
}

fn highlight(text: &str) -> LayoutJob {
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
    } else if word.chars().all(|c| c.is_ascii_digit()) {
        Color32::from_rgb(209, 154, 102)
    } else if COMPLETIONS.contains(&word) {
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
    fn line_and_column_map_to_a_character_index() {
        let text = "one\ntwo\nthree";
        assert_eq!(line_col_to_char_index(text, 1, 1), 0);
        assert_eq!(line_col_to_char_index(text, 2, 1), 4);
        assert_eq!(line_col_to_char_index(text, 2, 3), 6);
        assert_eq!(line_col_to_char_index(text, 99, 1), text.chars().count());
    }

    #[test]
    fn word_prefix_includes_dotted_serial_names() {
        assert_eq!(word_prefix("  pin", 5), Some((2, "pin".into())));
        assert_eq!(word_prefix("Serial.", 7), Some((0, "Serial.".into())));
        assert_eq!(word_prefix("x = ", 4), None);
    }

    #[test]
    fn completion_filter_is_case_insensitive_and_prefix_specific() {
        assert_eq!(filter_completions("pin"), vec!["pinMode"]);
        assert_eq!(
            filter_completions("serial."),
            vec!["serial.begin", "serial.println"]
        );
        assert!(filter_completions("zzz").is_empty());
    }

    #[test]
    fn highlighter_preserves_the_text() {
        let source = "void setup() {\n  // comment\n  pinMode(13, OUTPUT);\n}\n";
        let job = highlight(source);
        assert_eq!(job.text, source);
        assert!(!job.sections.is_empty());
    }

    #[test]
    fn char_to_byte_handles_multibyte_text() {
        let text = "aé\nb";
        assert_eq!(char_to_byte(text, 0), 0);
        assert_eq!(char_to_byte(text, 2), 3);
        assert_eq!(char_to_byte(text, 99), text.len());
    }

    #[test]
    fn diagnostic_index_is_safe_for_short_lines() {
        assert_eq!(line_col_to_char_index("a\n", 2, 99), 2);
        assert_eq!(line_col_to_char_index("", 1, 1), 0);
    }
}
