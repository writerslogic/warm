// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

//! The lexical analyzer of a store: how text becomes the terms its BM25 index holds and its
//! queries match. Every ingest and query path goes through `Analyzer::words` and
//! `Analyzer::term`, so index and query analysis cannot drift apart.
//!
//! The Persian and Hindi rules reimplement the algorithms of Apache Lucene 10.6
//! (`PersianNormalizer`, `PersianStemmer`, `ArabicNormalizer`, `HindiNormalizer`,
//! `HindiStemmer`; Apache-2.0), which follow Larkey et al., "Hindi CLIR in Thirty Days".

use std::borrow::Cow;

use anyhow::{bail, Result};

/// A store's lexical analysis, fixed when the store is created and recorded in its
/// `store.json` (absent there for [`Analyzer::Legacy`], so stores written before analyzers
/// existed and default stores keep their bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Analyzer {
    /// Maximal alphanumeric runs, `str::to_lowercase`, English Snowball stem: the analysis of
    /// every store written before analyzers existed, and the default.
    #[default]
    Legacy,
    /// [`Self::Legacy`] after Unicode NFC, with Turkish dotted and dotless I folded together.
    English,
    Arabic,
    Danish,
    Dutch,
    Finnish,
    French,
    German,
    Greek,
    Hungarian,
    Italian,
    Norwegian,
    Portuguese,
    Romanian,
    Russian,
    Spanish,
    Swedish,
    Tamil,
    /// Snowball Turkish after Turkish lowercasing (`I` to dotless `ı`, `İ` to `i`).
    Turkish,
    /// Arabic and Persian letter normalization and Lucene's Persian light stemmer. ZWNJ ends a
    /// word, as Lucene's `PersianCharFilter` makes it.
    Persian,
    /// Devanagari words keep their virama, nukta and joiners; Lucene's Hindi normalization and
    /// light stemmer.
    Hindi,
    /// Overlapping character-cluster bigrams over Thai, Lao, Khmer, Myanmar and CJK text,
    /// whole words elsewhere: dictionary-free, as Lucene's `CJKBigramFilter`.
    Bigram,
}

const NAMES: [(Analyzer, &str); 22] = [
    (Analyzer::Legacy, "legacy"),
    (Analyzer::English, "english"),
    (Analyzer::Arabic, "arabic"),
    (Analyzer::Danish, "danish"),
    (Analyzer::Dutch, "dutch"),
    (Analyzer::Finnish, "finnish"),
    (Analyzer::French, "french"),
    (Analyzer::German, "german"),
    (Analyzer::Greek, "greek"),
    (Analyzer::Hungarian, "hungarian"),
    (Analyzer::Italian, "italian"),
    (Analyzer::Norwegian, "norwegian"),
    (Analyzer::Portuguese, "portuguese"),
    (Analyzer::Romanian, "romanian"),
    (Analyzer::Russian, "russian"),
    (Analyzer::Spanish, "spanish"),
    (Analyzer::Swedish, "swedish"),
    (Analyzer::Tamil, "tamil"),
    (Analyzer::Turkish, "turkish"),
    (Analyzer::Persian, "persian"),
    (Analyzer::Hindi, "hindi"),
    (Analyzer::Bigram, "bigram"),
];

impl Analyzer {
    /// The name `store.json` records and [`str::parse`] accepts. A name never changes
    /// meaning: a changed rule set gets a new name.
    pub fn name(self) -> &'static str {
        NAMES.iter().find(|(a, _)| *a == self).expect("named").1
    }

    pub fn is_legacy(self) -> bool {
        self == Self::Legacy
    }

    fn snowball(self) -> Option<rust_stemmers::Algorithm> {
        use rust_stemmers::Algorithm as A;
        Some(match self {
            Self::Legacy | Self::English => A::English,
            Self::Arabic => A::Arabic,
            Self::Danish => A::Danish,
            Self::Dutch => A::Dutch,
            Self::Finnish => A::Finnish,
            Self::French => A::French,
            Self::German => A::German,
            Self::Greek => A::Greek,
            Self::Hungarian => A::Hungarian,
            Self::Italian => A::Italian,
            Self::Norwegian => A::Norwegian,
            Self::Portuguese => A::Portuguese,
            Self::Romanian => A::Romanian,
            Self::Russian => A::Russian,
            Self::Spanish => A::Spanish,
            Self::Swedish => A::Swedish,
            Self::Tamil => A::Tamil,
            Self::Turkish => A::Turkish,
            Self::Persian | Self::Hindi | Self::Bigram => return None,
        })
    }

    /// Whether words are the maximal alphanumeric runs of the text, as the ingest fast path
    /// scans them.
    pub(crate) fn splits_alphanumeric(self) -> bool {
        !matches!(self, Self::Hindi | Self::Bigram)
    }

    /// The text the words are cut from: NFC for every analyzer but [`Self::Legacy`]. Applied
    /// to the whole text, since a decomposed accent would otherwise end a word.
    pub(crate) fn prepare(self, text: &str) -> Cow<'_, str> {
        if self.is_legacy() {
            Cow::Borrowed(text)
        } else {
            super::nfc(text)
        }
    }

    /// The words of `text` (already [`Self::prepare`]d), in order.
    pub(crate) fn words(self, text: &str) -> Vec<&str> {
        match self {
            Self::Hindi => runs(text, |c| c.is_alphanumeric() || joins_indic(c)).collect(),
            Self::Bigram => {
                let mut out = Vec::new();
                for run in runs(text, |c| {
                    c.is_alphanumeric() || unicode_normalization::char::is_combining_mark(c)
                }) {
                    bigrams(run, &mut out);
                }
                out
            }
            _ => runs(text, char::is_alphanumeric).collect(),
        }
    }

    /// The term of one word in the unstemmed (`stem` false) or stemmed form, built in `lower`.
    pub(crate) fn term<'a>(self, word: &str, stem: bool, lower: &'a mut String) -> Cow<'a, str> {
        lower.clear();
        match self {
            Self::Legacy => {
                if word.is_ascii() {
                    lower.push_str(word);
                    lower.make_ascii_lowercase();
                } else {
                    // `str::to_lowercase`, not per-char mapping: it applies the final-sigma rule.
                    lower.push_str(&word.to_lowercase());
                }
            }
            Self::Turkish => {
                let turkish: String = word
                    .chars()
                    .map(|c| match c {
                        'I' => 'ı',
                        'İ' => 'i',
                        c => c,
                    })
                    .collect();
                lower.push_str(&turkish.to_lowercase());
            }
            _ => super::lowercase_word(word, lower),
        }
        if !self.is_legacy() && !lower.is_ascii() {
            fold_digits(lower);
            match self {
                Self::Persian => persian_normalize(lower),
                Self::Hindi => hindi_normalize(lower),
                _ => {}
            }
        }
        if !stem {
            return Cow::Borrowed(lower.as_str());
        }
        match self {
            Self::Persian => {
                persian_stem(lower);
                Cow::Borrowed(lower.as_str())
            }
            Self::Hindi => {
                hindi_stem(lower);
                Cow::Borrowed(lower.as_str())
            }
            Self::Bigram => Cow::Borrowed(lower.as_str()),
            _ => match self.snowball() {
                Some(rust_stemmers::Algorithm::English) => super::stemmer().stem(lower),
                Some(algorithm) => stemmer(algorithm).stem(lower),
                None => Cow::Borrowed(lower.as_str()),
            },
        }
    }

    /// Every term of `text` in one form, in text order.
    pub(crate) fn tokens(self, text: &str, stem: bool) -> Vec<String> {
        let text = self.prepare(text);
        let mut lower = String::new();
        self.words(&text)
            .into_iter()
            .map(|word| self.term(word, stem, &mut lower).into_owned())
            .filter(|term| !term.is_empty())
            .collect()
    }
}

impl std::fmt::Display for Analyzer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl std::str::FromStr for Analyzer {
    type Err = anyhow::Error;

    fn from_str(name: &str) -> Result<Self> {
        match NAMES.iter().find(|(_, n)| *n == name) {
            Some(&(analyzer, _)) => Ok(analyzer),
            None => bail!(
                "unknown lexical analyzer {name:?}; one of {}",
                NAMES.map(|(_, n)| n).join(", ")
            ),
        }
    }
}

fn stemmer(algorithm: rust_stemmers::Algorithm) -> &'static rust_stemmers::Stemmer {
    use std::sync::OnceLock;
    static STEMMERS: OnceLock<std::sync::Mutex<Vec<(u8, &'static rust_stemmers::Stemmer)>>> =
        OnceLock::new();
    // One leaked stemmer per algorithm (at most 18), so the hot path holds no lock across a stem.
    let key = algorithm as u8;
    let mut all = STEMMERS
        .get_or_init(Default::default)
        .lock()
        .expect("stemmers");
    if let Some(&(_, s)) = all.iter().find(|(k, _)| *k == key) {
        return s;
    }
    let s: &'static rust_stemmers::Stemmer =
        Box::leak(Box::new(rust_stemmers::Stemmer::create(algorithm)));
    all.push((key, s));
    s
}

fn runs(text: &str, keep: impl Fn(char) -> bool) -> impl Iterator<Item = &str> {
    text.split(move |c: char| !keep(c))
        .filter(|s| !s.is_empty())
}

/// Devanagari virama and nukta and the zero-width joiners, which a Hindi word keeps (Unicode
/// word segmentation does) so [`hindi_normalize`] can remove them.
fn joins_indic(c: char) -> bool {
    matches!(c, '\u{094D}' | '\u{093C}' | '\u{200C}' | '\u{200D}')
        || (unicode_normalization::char::is_combining_mark(c)
            && ('\u{0900}'..='\u{0DFF}').contains(&c))
}

fn bigram_script(c: char) -> bool {
    matches!(c,
        '\u{0E00}'..='\u{0EFF}' // Thai, Lao
        | '\u{1000}'..='\u{109F}' // Myanmar
        | '\u{1780}'..='\u{17FF}' // Khmer
        | '\u{1100}'..='\u{11FF}' | '\u{3040}'..='\u{30FF}' | '\u{3130}'..='\u{318F}'
        | '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{AC00}'..='\u{D7AF}'
        | '\u{F900}'..='\u{FAFF}' | '\u{20000}'..='\u{2FFFF}')
}

/// Pushes the terms of one run: each maximal stretch of bigram-script text as its overlapping
/// pairs of clusters (a base character and the marks after it), or the cluster alone when it
/// is the only one; every other stretch whole.
fn bigrams<'a>(run: &'a str, out: &mut Vec<&'a str>) {
    let mut clusters: Vec<(usize, usize)> = Vec::new();
    let flush = |clusters: &mut Vec<(usize, usize)>, out: &mut Vec<&'a str>| {
        match clusters.len() {
            0 => {}
            1 => out.push(&run[clusters[0].0..clusters[0].1]),
            _ => out.extend(clusters.windows(2).map(|w| &run[w[0].0..w[1].1])),
        }
        clusters.clear();
    };
    let mut other: Option<usize> = None;
    for (at, c) in run.char_indices() {
        let end = at + c.len_utf8();
        if unicode_normalization::char::is_combining_mark(c) {
            if let Some(last) = clusters.last_mut() {
                last.1 = end;
            }
            continue;
        }
        if bigram_script(c) {
            if let Some(start) = other.take() {
                out.push(&run[start..at]);
            }
            clusters.push((at, end));
        } else {
            flush(&mut clusters, out);
            other.get_or_insert(at);
        }
    }
    flush(&mut clusters, out);
    if let Some(start) = other {
        out.push(&run[start..]);
    }
}

/// Decimal digits of the scripts these analyzers serve, as ASCII (Lucene `DecimalDigitFilter`).
fn fold_digits(word: &mut String) {
    const ZEROS: [u32; 6] = [0x0660, 0x06F0, 0x0966, 0x09E6, 0x0E50, 0x0ED0];
    if !word
        .chars()
        .any(|c| ZEROS.iter().any(|&z| (z..z + 10).contains(&(c as u32))))
    {
        return;
    }
    *word = word
        .chars()
        .map(|c| {
            ZEROS
                .iter()
                .find(|&&z| (z..z + 10).contains(&(c as u32)))
                .map_or(c, |&z| char::from(b'0' + (c as u32 - z) as u8))
        })
        .collect();
}

fn persian_normalize(word: &mut String) {
    *word = word
        .chars()
        .filter_map(|c| match c {
            // ArabicNormalizer
            '\u{0622}' | '\u{0623}' | '\u{0625}' => Some('\u{0627}'),
            '\u{0649}' => Some('\u{064A}'),
            '\u{0629}' => Some('\u{0647}'),
            '\u{0640}' | '\u{064B}'..='\u{0652}' => None,
            // PersianNormalizer
            '\u{06CC}' | '\u{06D2}' => Some('\u{064A}'),
            '\u{06A9}' => Some('\u{0643}'),
            '\u{06C0}' | '\u{06C1}' => Some('\u{0647}'),
            '\u{0654}' => None,
            c => Some(c),
        })
        .collect();
}

/// Lucene's Persian light stemmer: each suffix in turn, every one that matches with at least
/// two characters left.
fn persian_stem(word: &mut String) {
    const SUFFIXES: [&str; 8] = [
        "\u{0627}\u{062A}",
        "\u{0627}\u{0646}",
        "\u{062A}\u{0631}\u{064A}\u{0646}",
        "\u{062A}\u{0631}",
        "\u{064A}\u{064A}",
        "\u{064A}",
        "\u{0647}\u{0627}",
        "\u{200C}",
    ];
    for suffix in SUFFIXES {
        if word.ends_with(suffix) && word.chars().count() >= suffix.chars().count() + 2 {
            word.truncate(word.len() - suffix.len());
        }
    }
}

fn hindi_normalize(word: &mut String) {
    let mut out = String::with_capacity(word.len());
    let mut chars = word.chars().peekable();
    while let Some(c) = chars.next() {
        let mapped = match c {
            // dead n: na + virama -> anusvara
            '\u{0928}' if chars.peek() == Some(&'\u{094D}') => {
                chars.next();
                Some('\u{0902}')
            }
            '\u{0901}' => Some('\u{0902}'),
            '\u{093C}' | '\u{200D}' | '\u{200C}' | '\u{094D}' => None,
            '\u{0929}' => Some('\u{0928}'),
            '\u{0931}' => Some('\u{0930}'),
            '\u{0934}' => Some('\u{0933}'),
            '\u{0958}' => Some('\u{0915}'),
            '\u{0959}' => Some('\u{0916}'),
            '\u{095A}' => Some('\u{0917}'),
            '\u{095B}' => Some('\u{091C}'),
            '\u{095C}' => Some('\u{0921}'),
            '\u{095D}' => Some('\u{0922}'),
            '\u{095E}' => Some('\u{092B}'),
            '\u{095F}' => Some('\u{092F}'),
            '\u{0945}' | '\u{0946}' => Some('\u{0947}'),
            '\u{0949}' | '\u{094A}' => Some('\u{094B}'),
            '\u{090D}' | '\u{090E}' => Some('\u{090F}'),
            '\u{0911}' | '\u{0912}' => Some('\u{0913}'),
            '\u{0972}' => Some('\u{0905}'),
            '\u{0906}' => Some('\u{0905}'),
            '\u{0908}' => Some('\u{0907}'),
            '\u{090A}' => Some('\u{0909}'),
            '\u{0960}' => Some('\u{090B}'),
            '\u{0961}' => Some('\u{090C}'),
            '\u{0910}' => Some('\u{090F}'),
            '\u{0914}' => Some('\u{0913}'),
            '\u{0940}' => Some('\u{093F}'),
            '\u{0942}' => Some('\u{0941}'),
            '\u{0944}' => Some('\u{0943}'),
            '\u{0963}' => Some('\u{0962}'),
            '\u{0948}' => Some('\u{0947}'),
            '\u{094C}' => Some('\u{094B}'),
            c => Some(c),
        };
        out.extend(mapped);
    }
    *word = out;
}

/// Lucene's Hindi light stemmer: the longest suffix group that leaves the word long enough,
/// lengths in characters (Lucene counts UTF-16 units, the same for Devanagari).
fn hindi_stem(word: &mut String) {
    const GROUPS: [(usize, &[&str]); 5] = [
        (6, &["ाएंगी", "ाएंगे", "ाऊंगी", "ाऊंगा", "ाइयाँ", "ाइयों", "ाइयां"]),
        (
            5,
            &[
                "ाएगी",
                "ाएगा",
                "ाओगी",
                "ाओगे",
                "एंगी",
                "ेंगी",
                "एंगे",
                "ेंगे",
                "ूंगी",
                "ूंगा",
                "ातीं",
                "नाओं",
                "नाएं",
                "ताओं",
                "ताएं",
                "ियाँ",
                "ियों",
                "ियां",
            ],
        ),
        (
            4,
            &[
                "ाकर",
                "ाइए",
                "ाईं",
                "ाया",
                "ेगी",
                "ेगा",
                "ोगी",
                "ोगे",
                "ाने",
                "ाना",
                "ाते",
                "ाती",
                "ाता",
                "तीं",
                "ाओं",
                "ाएं",
                "ुओं",
                "ुएं",
                "ुआं",
            ],
        ),
        (
            3,
            &[
                "कर", "ाओ", "िए", "ाई", "ाए", "ने", "नी", "ना", "ते", "ीं", "ती", "ता", "ाँ", "ां", "ों",
                "ें",
            ],
        ),
        (2, &["ो", "े", "ू", "ु", "ी", "ि", "ा"]),
    ];
    let len = word.chars().count();
    for (min, suffixes) in GROUPS {
        if len > min {
            if let Some(suffix) = suffixes.iter().find(|s| word.ends_with(*s)) {
                word.truncate(word.len() - suffix.len());
                return;
            }
        }
    }
}

#[cfg(test)]
pub(super) fn tests_names() -> &'static [(Analyzer, &'static str)] {
    &NAMES
}
