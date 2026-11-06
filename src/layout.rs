//! Keyboard layouts compiled through libxkbcommon, and the conversion logic.
//!
//! Instead of a hard-coded table, every layout the user has configured is
//! compiled with libxkbcommon, and we record which character each physical key
//! produces (alone, with Shift, with AltGr and with AltGr+Shift). Converting text then means: find which
//! key typed a character in the source layout, and look up what the same key
//! gives in the target layout.

use std::collections::HashMap;

use xkbcommon::xkb;

/// One configured layout, e.g. `us` or `ru` with variant `winkeys`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutSpec {
    pub name: String,
    pub variant: String,
}

impl LayoutSpec {
    pub fn new(name: &str, variant: &str) -> Self {
        Self {
            name: name.trim().to_string(),
            variant: variant.trim().to_string(),
        }
    }

    /// Short label for the OSD, e.g. "US", "RU".
    pub fn label(&self) -> String {
        self.name.to_uppercase()
    }
}

/// (XKB keycode, shift level). Level 0 = no modifiers, level 1 = Shift.
type KeyPos = (u32, u32);

/// The main typing block. XKB keycodes are evdev codes + 8.
/// Only these keys are used, so the numpad, function keys etc. can't create
/// conflicting duplicates (e.g. the numpad decimal key also produces '.' or ',').
fn typing_keys() -> impl Iterator<Item = u32> {
    (10..=21) // AE01..AE12: number row 1 … 0 - =
        .chain(24..=35) // AD01..AD12: q … ]
        .chain(38..=49) // AC01..AC11: a … '   and TLDE: `
        .chain(51..=61) // BKSL, AB01..AB10: \ z … /
}

pub struct Layout {
    pub spec: LayoutSpec,
    /// Index of this layout in the KDE layout list (used for getLayout/setLayout).
    pub index: u32,
    by_pos: HashMap<KeyPos, char>,
    by_char: HashMap<char, KeyPos>,
}

impl Layout {
    /// Compiles one layout. Returns None if XKB doesn't know it.
    fn compile(ctx: &xkb::Context, spec: LayoutSpec, index: u32) -> Option<Self> {
        let keymap = xkb::Keymap::new_from_names(
            ctx,
            "",
            "",
            spec.name.as_str(),
            spec.variant.as_str(),
            None,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )?;

        let mut by_pos = HashMap::new();
        let mut by_char = HashMap::new();
        // Two passes: first every key without and with Shift, then the AltGr
        // levels (AltGr, AltGr+Shift). Since the first position found for a
        // character wins, a normal position always beats an AltGr one, and
        // AltGr only adds characters that aren't on any key otherwise
        // (e.g. Polish ą on AltGr+A, German @ on AltGr+Q).
        for levels in [0..2u32, 2..4u32] {
            for key in typing_keys() {
                for level in levels.clone() {
                    let syms = keymap.key_get_syms_by_level(xkb::Keycode::new(key), 0, level);
                    let Some(&sym) = syms.first() else { continue };
                    let Some(c) = char::from_u32(xkb::keysym_to_utf32(sym)) else {
                        continue;
                    };
                    // keysym_to_utf32 gives 0 for keysyms without a character
                    // (dead keys etc.); '\0' counts as a control character.
                    if c.is_control() {
                        continue;
                    }
                    by_pos.insert((key, level), c);
                    // If one character sits on two keys, the first one wins.
                    by_char.entry(c).or_insert((key, level));
                }
            }
        }

        if by_char.is_empty() {
            return None;
        }
        Some(Self {
            spec,
            index,
            by_pos,
            by_char,
        })
    }

    /// Compiles every layout that XKB can handle; the rest are reported and skipped.
    pub fn compile_all(specs: Vec<LayoutSpec>) -> Vec<Self> {
        let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        specs
            .into_iter()
            .enumerate()
            .filter_map(|(i, spec)| {
                let layout = Self::compile(&ctx, spec.clone(), i as u32);
                if layout.is_none() {
                    eprintln!("[layout-fix] couldn't compile XKB layout {spec:?}, skipping it");
                }
                layout
            })
            .collect()
    }

    /// How many letters of `word` this layout can type.
    fn letter_score(&self, word: &str) -> usize {
        word.chars()
            .filter(|c| c.is_alphabetic() && self.by_char.contains_key(c))
            .count()
    }

    /// The character the same physical key gives in `target`.
    /// Characters this layout can't type pass through unchanged.
    fn translate_to(&self, c: char, target: &Layout) -> char {
        self.by_char
            .get(&c)
            .and_then(|pos| target.by_pos.get(pos))
            .copied()
            .unwrap_or(c)
    }
}

enum Segment<'a> {
    Word(&'a str),
    Space(&'a str),
}

/// Splits text into alternating runs of non-whitespace (words) and whitespace,
/// keeping everything, so joining the segments gives back the original text.
fn split_segments(text: &str) -> Vec<Segment<'_>> {
    fn make(s: &str, is_space: bool) -> Segment<'_> {
        if is_space {
            Segment::Space(s)
        } else {
            Segment::Word(s)
        }
    }

    let mut out = Vec::new();
    let mut start = 0;
    let mut current: Option<bool> = None;
    for (i, c) in text.char_indices() {
        let is_space = c.is_whitespace();
        if let Some(prev) = current {
            if prev != is_space {
                out.push(make(&text[start..i], prev));
                start = i;
            }
        }
        current = Some(is_space);
    }
    if let Some(prev) = current {
        out.push(make(&text[start..], prev));
    }
    out
}

pub struct Conversion {
    pub text: String,
    /// Directions used, in order of first appearance, e.g. ["US→RU"].
    pub directions: Vec<String>,
    /// Layout index (KDE numbering) of the last converted word's target.
    /// That's the layout you'd want to keep typing in.
    pub final_target: u32,
}

impl Conversion {
    pub fn summary(&self) -> String {
        self.directions.join(", ")
    }
}

pub struct Converter {
    layouts: Vec<Layout>,
}

impl Converter {
    /// Needs at least two layouts to convert between.
    pub fn new(layouts: Vec<Layout>) -> Option<Self> {
        (layouts.len() >= 2).then_some(Self { layouts })
    }

    /// The layout that can type the most letters of this word. None if the
    /// word has no letters any layout knows.
    ///
    /// Ties happen with layouts sharing an alphabet (us + de: "hallo" fits
    /// both). Then the active layout wins, since text typed in the wrong
    /// layout was usually typed in the one that's still active; otherwise the
    /// layout listed first. Layouts with different alphabets (us + ru) never
    /// tie on a word with letters.
    fn source_for(&self, word: &str, active: Option<usize>) -> Option<usize> {
        let scores: Vec<usize> = self.layouts.iter().map(|l| l.letter_score(word)).collect();
        let best = *scores.iter().max()?;
        if best == 0 {
            return None;
        }
        match active {
            Some(a) if scores.get(a) == Some(&best) => Some(a),
            _ => scores.iter().position(|&s| s == best),
        }
    }

    /// With two layouts it's simply "the other one". With more: the active
    /// layout if it differs from the source (you already switched to the one
    /// you meant), otherwise the next layout in the list.
    fn target_for(&self, source: usize, active: Option<usize>) -> usize {
        match active {
            Some(a) if a != source => a,
            _ => (source + 1) % self.layouts.len(),
        }
    }

    /// Converts each word from the layout it was typed in to the target layout.
    /// Words without letters (digits, lone punctuation) follow the direction of
    /// the nearest word that has letters. Returns None if the whole text has
    /// no letters at all, since there's nothing to base a direction on.
    pub fn convert(&self, text: &str, active_index: Option<u32>) -> Option<Conversion> {
        let active = active_index.and_then(|a| self.layouts.iter().position(|l| l.index == a));
        let segments = split_segments(text);

        let mut sources: Vec<Option<usize>> = segments
            .iter()
            .map(|seg| match seg {
                Segment::Word(w) => self.source_for(w, active),
                Segment::Space(_) => None,
            })
            .collect();
        if sources.iter().all(Option::is_none) {
            return None;
        }

        // Letterless words: take the previous lettered word's source…
        let mut last = None;
        for (seg, src) in segments.iter().zip(sources.iter_mut()) {
            if let Segment::Word(_) = seg {
                if src.is_some() {
                    last = *src;
                } else {
                    *src = last;
                }
            }
        }
        // …or, at the start of the text, the next one's.
        let mut next = None;
        for (seg, src) in segments.iter().zip(sources.iter_mut()).rev() {
            if let Segment::Word(_) = seg {
                if src.is_some() {
                    next = *src;
                } else {
                    *src = next;
                }
            }
        }

        let mut out = String::with_capacity(text.len());
        let mut pairs: Vec<(usize, usize)> = Vec::new();
        let mut final_target = None;
        for (seg, src) in segments.iter().zip(&sources) {
            match (seg, src) {
                (Segment::Word(w), Some(src)) => {
                    let dst = self.target_for(*src, active);
                    if !pairs.contains(&(*src, dst)) {
                        pairs.push((*src, dst));
                    }
                    final_target = Some(dst);
                    let (from, to) = (&self.layouts[*src], &self.layouts[dst]);
                    out.extend(w.chars().map(|c| from.translate_to(c, to)));
                }
                (Segment::Word(s), None) | (Segment::Space(s), _) => out.push_str(s),
            }
        }

        let directions = pairs
            .iter()
            .map(|&(s, d)| {
                format!(
                    "{}→{}",
                    self.layouts[s].spec.label(),
                    self.layouts[d].spec.label()
                )
            })
            .collect();

        Some(Conversion {
            text: out,
            directions,
            final_target: self.layouts[final_target?].index,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests compile real XKB layouts, so they need xkeyboard-config
    // installed (it always is on a desktop with KDE).

    fn converter(layouts: &[(&str, &str)]) -> Converter {
        let specs = layouts.iter().map(|(n, v)| LayoutSpec::new(n, v)).collect();
        let compiled = Layout::compile_all(specs);
        assert_eq!(compiled.len(), layouts.len(), "some XKB layouts failed to compile");
        Converter::new(compiled).unwrap()
    }

    fn us_ru() -> Converter {
        converter(&[("us", ""), ("ru", "")])
    }

    fn conv(c: &Converter, text: &str) -> String {
        c.convert(text, None).expect("text has letters").text
    }

    #[test]
    fn en_to_ru_basic_word() {
        // "руки" ("hands" in Russian) typed on an English layout -> "herb"
        assert_eq!(conv(&us_ru(), "herb"), "руки");
    }

    #[test]
    fn ru_to_en_basic_word() {
        assert_eq!(conv(&us_ru(), "руки"), "herb");
    }

    #[test]
    fn mixed_case_preserved() {
        assert_eq!(conv(&us_ru(), "Herb"), "Руки");
    }

    #[test]
    fn digits_and_shared_symbols_untouched() {
        assert_eq!(conv(&us_ru(), "test 123 (ok) *5*"), "еуые 123 (щл) *5*");
    }

    #[test]
    fn punctuation_follows_the_key() {
        let c = us_ru();
        // "привет." typed on the US layout
        assert_eq!(conv(&c, "ghbdtn/"), "привет.");
        // "без" typed on the US layout: ',' is the б key
        assert_eq!(conv(&c, ",tp"), "без");
        // In the RU layout '.' sits on the US '/' key
        assert_eq!(conv(&c, "руки."), "herb/");
    }

    #[test]
    fn each_word_gets_its_own_direction() {
        let c = us_ru();
        let result = c.convert("hello мир", None).unwrap();
        assert_eq!(result.text, "руддщ vbh");
        assert_eq!(result.summary(), "US→RU, RU→US");
    }

    #[test]
    fn letterless_word_follows_its_neighbour() {
        // '?' on the US layout is Shift+/, which gives ',' on the RU layout
        assert_eq!(conv(&us_ru(), "ghbdtn ?"), "привет ,");
        // At the start of the text it follows the next word
        assert_eq!(conv(&us_ru(), "? ghbdtn"), ", привет");
    }

    #[test]
    fn whitespace_preserved() {
        assert_eq!(conv(&us_ru(), "  ghbdtn\n\tvbh  "), "  привет\n\tмир  ");
    }

    #[test]
    fn no_letters_means_no_direction() {
        let c = us_ru();
        assert!(c.convert("123 !?", None).is_none());
        assert!(c.convert("", None).is_none());
        assert!(c.convert("   ", None).is_none());
    }

    #[test]
    fn roundtrip_all_ascii_printable() {
        let c = us_ru();
        let (us, ru) = (&c.layouts[0], &c.layouts[1]);
        for ch in '!'..='~' {
            let there = us.translate_to(ch, ru);
            let back = ru.translate_to(there, us);
            assert_eq!(back, ch, "roundtrip failed for {ch:?} -> {there:?} -> {back:?}");
        }
    }

    #[test]
    fn other_scripts_and_emoji_untouched() {
        // 'c', 'a', 'f' are Latin and get converted; 'é', CJK and emoji can't
        // be typed on either layout, so they stay as they are.
        assert_eq!(conv(&us_ru(), "日本語 🎉 café"), "日本語 🎉 сфаé");
    }

    #[test]
    fn summary_and_final_target() {
        let result = us_ru().convert("ghbdtn", None).unwrap();
        assert_eq!(result.summary(), "US→RU");
        assert_eq!(result.final_target, 1);
    }

    #[test]
    fn three_layouts_prefer_active_target() {
        let c = converter(&[("us", ""), ("ru", ""), ("ua", "")]);
        // Not switched yet: the next layout after US
        assert_eq!(c.convert("s", None).unwrap().text, "ы");
        // Already switched to Ukrainian: convert into it
        assert_eq!(c.convert("s", Some(2)).unwrap().text, "і");
    }

    #[test]
    fn tie_goes_to_the_active_layout() {
        // "tea" fits both us and de; only the ';' tells the directions apart:
        // on us it's the key that gives 'ö' on de, on de it's Shift+, which
        // gives '<' on us.
        let c = converter(&[("us", ""), ("de", "")]);
        assert_eq!(c.convert("tea;", None).unwrap().text, "teaö");
        assert_eq!(c.convert("tea;", Some(0)).unwrap().text, "teaö");
        assert_eq!(c.convert("tea;", Some(1)).unwrap().text, "tea<");
    }

    #[test]
    fn active_layout_doesnt_matter_for_different_alphabets() {
        let c = us_ru();
        for active in [None, Some(0), Some(1)] {
            assert_eq!(c.convert("ghbdtn", active).unwrap().text, "привет");
            assert_eq!(c.convert("руки", active).unwrap().text, "herb");
        }
    }

    #[test]
    fn altgr_characters_are_known() {
        let layouts = Layout::compile_all(vec![LayoutSpec::new("pl", ""), LayoutSpec::new("de", "")]);
        let (pl, de) = (&layouts[0], &layouts[1]);
        // AC01 (the A key) is keycode 38, AD01 (the Q key) is 24; level 2 = AltGr.
        assert_eq!(pl.by_char.get(&'ą'), Some(&(38, 2)));
        assert_eq!(pl.by_char.get(&'a'), Some(&(38, 0)));
        assert_eq!(de.by_char.get(&'@'), Some(&(24, 2)));
    }

    #[test]
    fn normal_positions_beat_altgr() {
        // Whenever a character can be typed without AltGr, that position
        // must be the one used for converting it.
        let specs = ["us", "ru", "de", "pl", "fr", "ua"]
            .iter()
            .map(|n| LayoutSpec::new(n, ""))
            .collect();
        for layout in Layout::compile_all(specs) {
            for (&(key, level), &c) in &layout.by_pos {
                if level < 2 {
                    let (_, found_level) = layout.by_char[&c];
                    assert!(
                        found_level < 2,
                        "{}: {c:?} on key {key} level {level}, but mapped to level {found_level}",
                        layout.spec.name
                    );
                }
            }
        }
    }

    #[test]
    fn altgr_letter_without_counterpart_passes_through() {
        // us has nothing on AltGr+A, so 'ą' stays as it is.
        let c = converter(&[("pl", ""), ("us", "")]);
        assert_eq!(c.convert("ząb", Some(0)).unwrap().text, "ząb");
    }
}
