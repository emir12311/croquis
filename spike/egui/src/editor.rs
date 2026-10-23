//! Native syntax highlighting for the sketch panel.

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{Color32, FontId};

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
}
