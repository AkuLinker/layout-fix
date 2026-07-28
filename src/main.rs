use std::io::Write;
use std::process::{Command, Stdio};

use zbus::blocking::Connection;

const OSD_DEST: &str = "org.kde.plasmashell";
const OSD_PATH: &str = "/org/kde/osdService";
const OSD_IFACE: &str = "org.kde.osdService";

fn show_osd(icon: &str, text: &str) {
    // Not critical if this fails — silently drop the notification, don't crash.
    if let Ok(session) = Connection::session() {
        let _ = session.call_method(
            Some(OSD_DEST),
            OSD_PATH,
            Some(OSD_IFACE),
            "showText",
            &(icon, text),
        );
    }
}

/// Reads the PRIMARY selection (whatever is highlighted with the mouse/Shift+arrows).
fn read_primary_selection() -> Option<String> {
    let output = Command::new("wl-paste")
        .arg("--primary")
        .arg("--no-newline")
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout).to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Puts text into the regular clipboard (not PRIMARY) via wl-copy.
fn write_clipboard(text: &str) -> std::io::Result<()> {
    let mut child = Command::new("wl-copy").stdin(Stdio::piped()).spawn()?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(text.as_bytes())?;
    }
    child.wait()?;
    Ok(())
}

fn is_cyrillic_letter(c: char) -> bool {
    matches!(c, '\u{0400}'..='\u{04FF}')
}

fn is_latin_letter(c: char) -> bool {
    c.is_ascii_alphabetic()
}

#[derive(Debug, PartialEq)]
enum Direction {
    EnToRu,
    RuToEn,
}

/// Determines the conversion direction by the majority of letters in the text.
/// Returns None if there are no letters at all (only punctuation/digits) —
/// in that case direction is fundamentally undecidable, so we don't guess.
fn detect_direction(text: &str) -> Option<Direction> {
    let mut latin = 0usize;
    let mut cyrillic = 0usize;
    for c in text.chars() {
        if is_latin_letter(c) {
            latin += 1;
        } else if is_cyrillic_letter(c) {
            cyrillic += 1;
        }
    }
    if latin == 0 && cyrillic == 0 {
        None
    } else if latin >= cyrillic {
        Some(Direction::EnToRu)
    } else {
        Some(Direction::RuToEn)
    }
}

fn convert(text: &str, direction: &Direction) -> String {
    let f: fn(char) -> char = match direction {
        Direction::EnToRu => en_to_ru,
        Direction::RuToEn => ru_to_en,
    };
    text.chars().map(f).collect()
}

fn preview(text: &str, max_chars: usize) -> String {
    let mut s: String = text.chars().take(max_chars).collect();
    if text.chars().count() > max_chars {
        s.push('…');
    }
    s
}

fn en_to_ru(c: char) -> char {
    match c {
        '`' => 'ё',
        '~' => 'Ё',
        '@' => '"',
        '#' => '№',
        '$' => ';',
        '^' => ':',
        '&' => '?',
        'q' => 'й',
        'Q' => 'Й',
        'w' => 'ц',
        'W' => 'Ц',
        'e' => 'у',
        'E' => 'У',
        'r' => 'к',
        'R' => 'К',
        't' => 'е',
        'T' => 'Е',
        'y' => 'н',
        'Y' => 'Н',
        'u' => 'г',
        'U' => 'Г',
        'i' => 'ш',
        'I' => 'Ш',
        'o' => 'щ',
        'O' => 'Щ',
        'p' => 'з',
        'P' => 'З',
        '[' => 'х',
        '{' => 'Х',
        ']' => 'ъ',
        '}' => 'Ъ',
        'a' => 'ф',
        'A' => 'Ф',
        's' => 'ы',
        'S' => 'Ы',
        'd' => 'в',
        'D' => 'В',
        'f' => 'а',
        'F' => 'А',
        'g' => 'п',
        'G' => 'П',
        'h' => 'р',
        'H' => 'Р',
        'j' => 'о',
        'J' => 'О',
        'k' => 'л',
        'K' => 'Л',
        'l' => 'д',
        'L' => 'Д',
        ';' => 'ж',
        ':' => 'Ж',
        '\'' => 'э',
        '"' => 'Э',
        '|' => '/',
        'z' => 'я',
        'Z' => 'Я',
        'x' => 'ч',
        'X' => 'Ч',
        'c' => 'с',
        'C' => 'С',
        'v' => 'м',
        'V' => 'М',
        'b' => 'и',
        'B' => 'И',
        'n' => 'т',
        'N' => 'Т',
        'm' => 'ь',
        'M' => 'Ь',
        ',' => 'б',
        '<' => 'Б',
        '.' => 'ю',
        '>' => 'Ю',
        '/' => '.',
        '?' => ',',
        _ => c,
    }
}

fn ru_to_en(c: char) -> char {
    match c {
        'ё' => '`',
        'Ё' => '~',
        '"' => '@',
        '№' => '#',
        ';' => '$',
        ':' => '^',
        '?' => '&',
        'й' => 'q',
        'Й' => 'Q',
        'ц' => 'w',
        'Ц' => 'W',
        'у' => 'e',
        'У' => 'E',
        'к' => 'r',
        'К' => 'R',
        'е' => 't',
        'Е' => 'T',
        'н' => 'y',
        'Н' => 'Y',
        'г' => 'u',
        'Г' => 'U',
        'ш' => 'i',
        'Ш' => 'I',
        'щ' => 'o',
        'Щ' => 'O',
        'з' => 'p',
        'З' => 'P',
        'х' => '[',
        'Х' => '{',
        'ъ' => ']',
        'Ъ' => '}',
        'ф' => 'a',
        'Ф' => 'A',
        'ы' => 's',
        'Ы' => 'S',
        'в' => 'd',
        'В' => 'D',
        'а' => 'f',
        'А' => 'F',
        'п' => 'g',
        'П' => 'G',
        'р' => 'h',
        'Р' => 'H',
        'о' => 'j',
        'О' => 'J',
        'л' => 'k',
        'Л' => 'K',
        'д' => 'l',
        'Д' => 'L',
        'ж' => ';',
        'Ж' => ':',
        'э' => '\'',
        'Э' => '"',
        '/' => '|',
        'я' => 'z',
        'Я' => 'Z',
        'ч' => 'x',
        'Ч' => 'X',
        'с' => 'c',
        'С' => 'C',
        'м' => 'v',
        'М' => 'V',
        'и' => 'b',
        'И' => 'B',
        'т' => 'n',
        'Т' => 'N',
        'ь' => 'm',
        'Ь' => 'M',
        'б' => ',',
        'Б' => '<',
        'ю' => '.',
        'Ю' => '>',
        '.' => '/',
        ',' => '?',
        _ => c,
    }
}

fn main() {
    let Some(selected) = read_primary_selection() else {
        show_osd("edit-select-none", "Nothing to convert — no selection");
        return;
    };

    let Some(direction) = detect_direction(&selected) else {
        show_osd(
            "dialog-question",
            "Could not detect language — no letters in selection",
        );
        return;
    };

    let converted = convert(&selected, &direction);

    if let Err(e) = write_clipboard(&converted) {
        eprintln!("[layout-fix] failed to write to clipboard: {e}");
        show_osd("dialog-error", "Error: failed to write to clipboard");
        return;
    }

    let arrow = match direction {
        Direction::EnToRu => "EN→RU",
        Direction::RuToEn => "RU→EN",
    };
    show_osd(
        "input-keyboard",
        &format!("{arrow}: {}", preview(&converted, 40)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn en_to_ru_basic_word() {
        // "hands" typed on a US layout while meaning Russian "руки" -> "руки"
        let converted = convert("herb", &Direction::EnToRu);
        assert_eq!(converted, "руки");
    }

    #[test]
    fn ru_to_en_basic_word() {
        let converted = convert("руки", &Direction::RuToEn);
        assert_eq!(converted, "herb");
    }

    #[test]
    fn punctuation_and_digits_untouched() {
        // Digits, spaces, and symbols identical on both layouts stay untouched
        let converted = convert("test 123 (ok) *5*", &Direction::EnToRu);
        assert_eq!(converted, "еуые 123 (щл) *5*");
    }

    #[test]
    fn comma_and_period_direction_dependent() {
        // EN->RU: comma and period turn into the letters б/ю
        assert_eq!(convert(",", &Direction::EnToRu), "б");
        assert_eq!(convert(".", &Direction::EnToRu), "ю");
        // RU->EN: period and comma (from the RU key that winkeys remaps to
        // AB10) turn into / and ?
        assert_eq!(convert(".", &Direction::RuToEn), "/");
        assert_eq!(convert(",", &Direction::RuToEn), "?");
    }

    #[test]
    fn semicolon_pair() {
        assert_eq!(convert(";", &Direction::EnToRu), "ж");
        assert_eq!(convert("ж", &Direction::RuToEn), ";");
    }

    #[test]
    fn roundtrip_all_ascii_printable() {
        // Every character present in the en_to_ru table must return to where
        // it came from when converted back through ru_to_en.
        for c in ('!'..='~').chain(std::iter::once(' ')) {
            let ru = en_to_ru(c);
            if ru != c {
                let back = ru_to_en(ru);
                assert_eq!(back, c, "roundtrip failed for {c:?} -> {ru:?} -> {back:?}");
            }
        }
    }

    #[test]
    fn direction_detection() {
        assert_eq!(detect_direction("hello world"), Some(Direction::EnToRu));
        assert_eq!(detect_direction("привет мир"), Some(Direction::RuToEn));
        assert_eq!(detect_direction("123 456 !@#"), None);
        assert_eq!(detect_direction(""), None);
    }

    #[test]
    fn mixed_case_preserved() {
        let converted = convert("Herb", &Direction::EnToRu);
        assert_eq!(converted, "Руки");
    }

    #[test]
    fn other_scripts_and_emoji_untouched() {
        let converted = convert("日本語 🎉 café", &Direction::EnToRu);
        // 'a' and 'e' in café are Latin letters, so they do get converted
        // (expected — we don't distinguish "French e" from "English e"),
        // but the CJK characters and emoji should stay untouched.
        assert!(converted.contains('日'));
        assert!(converted.contains('本'));
        assert!(converted.contains('語'));
        assert!(converted.contains('🎉'));
    }
}
