//! The interface's text, in every language the app ships.
//!
//! The strings live in `assets/locales/<tag>.json`, one flat object of key to
//! text per language, so a translator edits data rather than Rust and needs no
//! toolchain. The cost of that choice is that a missing key cannot be caught by
//! the compiler — so `a_catalogue_matches_english_key_for_key` carries the load
//! instead, and `Texts::t` falls back rather than showing a blank.
//!
//! Keys are dotted and grouped by where they appear: `menu.file.save`,
//! `palette.heading`, `part.led`. A key that names a part mirrors the kind's
//! own spelling (`PartKind::Led` → `part.led`) so the two can be checked against
//! the catalogue by a test rather than by eye.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A language the app ships.
///
/// The list is deliberately closed: a language is a file someone translated and
/// a name to offer in the menu, not a string the user may invent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Locale {
    English,
    Turkish,
}

impl Locale {
    /// In the order the language menu lists them.
    pub const ALL: [Locale; 2] = [Self::English, Self::Turkish];

    /// The tag its catalogue file is named with, and the one the config saves.
    pub fn tag(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Turkish => "tr",
        }
    }

    /// The language's name *in itself*, which is what a language menu should
    /// show: someone who cannot read English cannot find "Turkish" in a list.
    pub fn name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Turkish => "Türkçe",
        }
    }

    /// The locale a saved tag names, or `None` for a tag no language claims —
    /// which is what an older or hand-edited config can hold.
    pub fn from_tag(tag: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|locale| locale.tag() == tag)
    }
}

/// One language's strings.
pub type Catalogue = BTreeMap<String, String>;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Read one language's catalogue. A missing or unreadable file is an empty
/// catalogue, not a panic: the app still runs, `t` falls back, and the tests are
/// what catch the file going missing.
pub fn load(locale: Locale) -> Catalogue {
    let path = root().join("assets/locales").join(format!("{}.json", locale.tag()));
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Catalogue::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

/// Where the chosen language is remembered between runs.
///
/// Not in the diagram and not in eframe's own storage: the language belongs to
/// the reader, not to the circuit they happen to have open, so one file for the
/// app is right. XDG first, because that is where a Linux desktop expects a
/// config file, with `~/.config` as the fallback.
fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("croquis").join("locale"))
}

/// The language the last run chose, or English.
///
/// An unreadable file, a tag for a language we no longer ship, or a hand-edited
/// value all mean the same thing: start in English rather than refuse to start.
pub fn read_locale(path: &Path) -> Locale {
    let Ok(tag) = std::fs::read_to_string(path) else {
        return Locale::English;
    };
    Locale::from_tag(tag.trim()).unwrap_or(Locale::English)
}

/// Remember the chosen language. Failure is not worth a dialog: the app keeps
/// running in the language the reader picked, it just forgets it next time.
pub fn write_locale(path: &Path, locale: Locale) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, format!("{}\n", locale.tag()))
}

/// The language this run should start in.
pub fn saved_locale() -> Locale {
    config_path().map(|path| read_locale(&path)).unwrap_or(Locale::English)
}

/// Remember `locale` for the next run.
pub fn save_locale(locale: Locale) {
    if let Some(path) = config_path() {
        let _ = write_locale(&path, locale);
    }
}

/// The text the interface draws, in the chosen language.
///
/// English is kept alongside whatever is chosen because it is the contract: every
/// key is supposed to exist there, and a translation that has drifted shows the
/// English rather than nothing.
///
/// The catalogues sit behind `Arc` so a `Texts` is cheap to clone, which is what
/// lets a widget-heavy closure take its own copy: `t` borrows the `Texts` it was
/// called on, and holding that borrow across a closure that wants `&mut self`
/// would not compile.
#[derive(Clone)]
pub struct Texts {
    locale: Locale,
    chosen: Arc<Catalogue>,
    english: Arc<Catalogue>,
}

impl Texts {
    pub fn load(locale: Locale) -> Self {
        Self {
            locale,
            chosen: Arc::new(load(locale)),
            english: Arc::new(load(Locale::English)),
        }
    }

    pub fn locale(&self) -> Locale {
        self.locale
    }

    /// Switch language. Nothing is reloaded when the language is already chosen,
    /// so this is safe to call from a menu that redraws every frame.
    pub fn set_locale(&mut self, locale: Locale) {
        if locale != self.locale {
            self.locale = locale;
            self.chosen = Arc::new(load(locale));
        }
    }

    /// The text for `key`.
    ///
    /// A key missing from the chosen language reads in English; one missing from
    /// both reads as the key itself. Showing `menu.file.save` is ugly on purpose:
    /// it is a bug the next screenshot finds, where a blank line hides.
    pub fn t<'a>(&'a self, key: &'a str) -> &'a str {
        self.chosen
            .get(key)
            .or_else(|| self.english.get(key))
            .map(String::as_str)
            .unwrap_or(key)
    }

    /// The keys this language is missing, in key order. For tests and for a
    /// translator's "what is left" list.
    pub fn missing(&self) -> Vec<&str> {
        self.english
            .keys()
            .filter(|key| !self.chosen.contains_key(*key))
            .map(String::as_str)
            .collect()
    }

    /// Whether English — the contract — carries any text for `key`.
    ///
    /// A key that only ever resolves by falling back to itself reads as its own
    /// name in the UI, which is a bug; this is how the catalogue coverage test
    /// tells a real entry from that.
    pub fn has(&self, key: &str) -> bool {
        self.english.contains_key(key)
    }

    /// The text for `key` with `{name}`-style placeholders filled in.
    ///
    /// A translation has to be able to put the value anywhere in the sentence —
    /// Turkish and English do not agree on where "Could not load {name}" puts the
    /// name — so the sentence comes from the file whole and the value is
    /// substituted into it, never glued on the end.
    pub fn tf(&self, key: &str, pairs: &[(&str, &str)]) -> String {
        let mut out = self.t(key).to_string();
        for (name, value) in pairs {
            out = out.replace(&format!("{{{name}}}"), value);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(locale: Locale) -> Vec<String> {
        load(locale).keys().cloned().collect()
    }

    #[test]
    fn every_shipped_language_loads_its_catalogue() {
        for locale in Locale::ALL {
            let catalogue = load(locale);
            assert!(
                !catalogue.is_empty(),
                "{} ({}) has no strings: assets/locales/{}.json is missing or empty",
                locale.name(),
                locale.tag(),
                locale.tag()
            );
        }
    }

    #[test]
    fn a_catalogue_matches_english_key_for_key() {
        // The one guarantee the compiler cannot give: a language that is missing
        // a key, or carrying one English dropped, is a translation that has
        // drifted. Both directions matter — an extra key is text nobody shows.
        let english = keys(Locale::English);
        for locale in Locale::ALL {
            if locale == Locale::English {
                continue;
            }
            let translated = keys(locale);
            let mut missing: Vec<&String> =
                english.iter().filter(|key| !translated.contains(key)).collect();
            missing.sort();
            let mut extra: Vec<&String> =
                translated.iter().filter(|key| !english.contains(key)).collect();
            extra.sort();
            assert!(
                missing.is_empty(),
                "{} is missing {} key(s): {missing:?}",
                locale.name(),
                missing.len()
            );
            assert!(
                extra.is_empty(),
                "{} has {} key(s) English does not: {extra:?}",
                locale.name(),
                extra.len()
            );
        }
    }

    #[test]
    fn no_catalogue_ships_an_empty_string() {
        // An empty value is indistinguishable from a deliberate blank in the UI
        // and is almost always a half-finished translation.
        for locale in Locale::ALL {
            for (key, text) in load(locale) {
                assert!(
                    !text.trim().is_empty(),
                    "{} has an empty value for `{key}`",
                    locale.name()
                );
            }
        }
    }

    #[test]
    fn a_translation_is_used_where_it_exists() {
        let texts = Texts::load(Locale::Turkish);
        assert_eq!(texts.locale(), Locale::Turkish);
        // Read through the real catalogue, not a fixture: if Turkish says
        // something other than English for this key, the lookup is doing its job.
        let turkish = load(Locale::Turkish);
        let english = load(Locale::English);
        let a_key = english
            .iter()
            .find(|(key, text)| turkish.get(*key).is_some_and(|other| other != *text))
            .map(|(key, _)| key.clone())
            .expect("Turkish differs from English somewhere, or nothing is translated");
        assert_eq!(texts.t(&a_key), turkish[&a_key]);
    }

    #[test]
    fn a_missing_key_falls_back_to_english_then_to_the_key() {
        let mut texts = Texts::load(Locale::Turkish);
        // Pretend a Turkish translation is incomplete.
        let english_key = texts
            .english
            .keys()
            .find(|key| texts.chosen.contains_key(*key))
            .cloned()
            .expect("the catalogues share keys");
        let mut chosen = (*texts.chosen).clone();
        chosen.remove(&english_key);
        texts.chosen = Arc::new(chosen);
        assert_eq!(
            texts.t(&english_key),
            texts.english[&english_key],
            "a key the chosen language lost reads in English"
        );
        assert_eq!(
            texts.t("no.such.key"),
            "no.such.key",
            "a key nobody has reads as itself, so the gap is visible"
        );
    }

    #[test]
    fn switching_language_swaps_the_catalogue() {
        let mut texts = Texts::load(Locale::English);
        assert_eq!(texts.missing(), Vec::<&str>::new(), "English is the contract");
        texts.set_locale(Locale::Turkish);
        assert_eq!(texts.locale(), Locale::Turkish);
        assert!(
            texts.missing().is_empty(),
            "Turkish should be complete: {:?}",
            texts.missing()
        );
    }

    #[test]
    fn a_locale_round_trips_through_its_tag() {
        for locale in Locale::ALL {
            assert_eq!(Locale::from_tag(locale.tag()), Some(locale));
        }
        assert_eq!(Locale::from_tag("de"), None, "a language we do not ship");
    }

    #[test]
    fn every_key_the_catalogue_declares_has_a_translation() {
        // The core hands out keys, the locale files hold the words, and nothing
        // in Rust ties the two together except this test. A new part is one
        // `PartKind` variant away from shipping an untranslated name — this is
        // what stops it.
        use croquis_core::catalogue::{field_key, parts, FieldControl};

        let texts = Texts::load(Locale::English);
        let mut missing = Vec::new();
        for part in parts() {
            if !texts.has(&part.label_key) {
                missing.push(part.label_key.clone());
            }
            for field in &part.fields {
                if !texts.has(field.label_key) {
                    missing.push(field.label_key.to_string());
                }
                if let FieldControl::Select(options) = &field.control {
                    for option in options {
                        if !texts.has(&option.label_key) {
                            missing.push(option.label_key.clone());
                        }
                    }
                }
            }
        }
        for key in field_key::ALL {
            if !texts.has(key) {
                missing.push((*key).to_string());
            }
        }
        missing.sort();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "{} catalogue key(s) have no text in en.json: {missing:?}",
            missing.len()
        );
    }

    #[test]
    fn the_chosen_language_survives_a_round_trip() {
        // A nested path, so writing also has to create the directory it lives in.
        let dir = std::env::temp_dir().join(format!("croquis-locale-{}", std::process::id()));
        let path = dir.join("croquis").join("locale");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            read_locale(&path),
            Locale::English,
            "a file nobody wrote yet is English"
        );
        write_locale(&path, Locale::Turkish).expect("write");
        assert_eq!(read_locale(&path), Locale::Turkish);
        write_locale(&path, Locale::English).expect("write");
        assert_eq!(read_locale(&path), Locale::English, "and it can go back");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_saved_tag_no_language_claims_starts_in_english() {
        // What a hand-edited or downgraded config can hold. Refusing to start in
        // a language we do not have would be worse than starting in English.
        let path = std::env::temp_dir().join(format!(
            "croquis-locale-bad-{}.json",
            std::process::id()
        ));
        std::fs::write(&path, "klingon\n").expect("write");
        assert_eq!(read_locale(&path), Locale::English);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn every_key_the_shell_asks_for_has_a_translation() {
        // The shell names its keys as string literals, so a typo there cannot be
        // a compile error — it would just show a key on a button. Reading them
        // back out of the source turns that into a failing test.
        let source = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs"),
        )
        .expect("the shell's own source");
        let texts = Texts::load(Locale::English);
        let mut missing = Vec::new();
        for marker in [".t(\"", ".tf(\""] {
            let mut rest = source.as_str();
            while let Some(at) = rest.find(marker) {
                rest = &rest[at + marker.len()..];
                let Some(end) = rest.find('"') else { break };
                let key = &rest[..end];
                if !texts.has(key) {
                    missing.push(key.to_string());
                }
            }
        }
        missing.sort();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "the shell asks for keys no catalogue carries: {missing:?}"
        );
    }

    #[test]
    fn a_part_and_field_read_as_words_not_as_keys() {
        // End to end: the catalogue's own keys, resolved in both languages,
        // come out as things a person would read.
        use croquis_core::catalogue::{part_by_kind, PartKind};

        let english = Texts::load(Locale::English);
        let turkish = Texts::load(Locale::Turkish);
        let led = part_by_kind(PartKind::Led);
        assert_eq!(english.t(&led.label_key), "LED");
        assert_eq!(turkish.t(&led.label_key), "LED");
        let resistor = part_by_kind(PartKind::Resistor);
        assert_eq!(english.t(&resistor.label_key), "Resistor");
        assert_eq!(turkish.t(&resistor.label_key), "Direnç");
        let value = &resistor.fields[0];
        assert_eq!(english.t(value.label_key), "Resistance");
        assert_eq!(turkish.t(value.label_key), "Direnç");
        assert!(
            !english.t(&led.label_key).starts_with("part."),
            "a key must never reach the screen"
        );
    }
}
