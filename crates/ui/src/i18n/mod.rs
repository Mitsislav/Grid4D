//! The interface language: every text the interface shows goes through
//! [`tr`] (or [`trf`] with placeholders), whose key is the English text, so
//! code stays readable and English is the fallback. Spec:
//! `docs/reference/workspace-language.md`.

mod el;

use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

/// The languages the interface speaks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[default]
    English,
    Greek,
}

impl Language {
    pub const ALL: [Self; 2] = [Self::English, Self::Greek];

    /// Its name in itself (as a language menu lists it).
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Greek => "Ελληνικά",
        }
    }

    const fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Greek,
            _ => Self::English,
        }
    }

    const fn to_u8(self) -> u8 {
        match self {
            Self::English => 0,
            Self::Greek => 1,
        }
    }
}

/// The language in use (a plain atomic: reading it costs nothing).
static CURRENT: AtomicU8 = AtomicU8::new(0);

/// The language the interface speaks now.
#[must_use]
pub fn language() -> Language {
    Language::from_u8(CURRENT.load(Ordering::Relaxed))
}

/// Switches the interface to `language` (takes effect on the next frame).
pub fn set_language(language: Language) {
    CURRENT.store(language.to_u8(), Ordering::Relaxed);
}

/// Marks a text for translation where [`tr`] cannot run (constants, `const
/// fn`s); translate it with [`tr`] where it is shown.
#[must_use]
pub const fn key(english: &'static str) -> &'static str {
    english
}

/// `english` in the language in use (itself in English, or when no
/// translation exists).
#[must_use]
pub fn tr(english: &str) -> &str {
    match language() {
        Language::English => english,
        Language::Greek => el::lookup(english).unwrap_or(english),
    }
}

/// [`tr`] with numbered placeholders: `{0}`, `{1}`, … in the text are
/// replaced by `args` in order (a translation may move them).
#[must_use]
pub fn trf(english: &str, args: &[&dyn std::fmt::Display]) -> String {
    let mut text = tr(english).to_owned();
    for (index, arg) in args.iter().enumerate() {
        text = text.replace(&format!("{{{index}}}"), &arg.to_string());
    }
    text
}

#[cfg(test)]
mod tests;
