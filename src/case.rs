//! Fixing words where Shift was held a little too long: "HEllo" -> "Hello",
//! "CALling" -> "Calling", "ПРивет" -> "Привет".
//!
//! Only this exact pattern is touched, because anything else could be
//! intended: a run of letters that starts with 2 or 3 uppercase letters,
//! followed by at least 2 lowercase letters and nothing else. So "TVs",
//! "URLs" and "APIs" (only one lowercase letter), "HELLO" and "hello", and
//! "HELLo" (four uppercase letters) all stay as they are.

/// Fixes every matching run of letters in `text`; everything else is kept.
pub fn fix_shift_case(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut run = String::new();
    for c in text.chars() {
        if c.is_alphabetic() {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
}

fn flush(run: &mut String, out: &mut String) {
    match fix_run(run) {
        Some(fixed) => out.push_str(&fixed),
        None => out.push_str(run),
    }
    run.clear();
}

/// Returns the fixed run, or None if it doesn't match the pattern.
fn fix_run(run: &str) -> Option<String> {
    let chars: Vec<char> = run.chars().collect();
    let upper = chars.iter().take_while(|c| c.is_uppercase()).count();
    let rest = &chars[upper..];
    let matches = (2..=3).contains(&upper)
        && rest.len() >= 2
        && rest.iter().all(|c| c.is_lowercase());
    if !matches {
        return None;
    }

    let mut fixed = String::with_capacity(run.len());
    fixed.push(chars[0]);
    for c in &chars[1..upper] {
        fixed.extend(c.to_lowercase());
    }
    fixed.extend(rest);
    Some(fixed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixes_two_or_three_uppercase_letters() {
        assert_eq!(fix_shift_case("HEllo"), "Hello");
        assert_eq!(fix_shift_case("CALling"), "Calling");
        assert_eq!(fix_shift_case("ПРивет"), "Привет");
        assert_eq!(fix_shift_case("МИРу"), "МИРу"); // only one lowercase letter
    }

    #[test]
    fn leaves_intended_words_alone() {
        for word in ["TVs", "URLs", "APIs", "HELLO", "hello", "Hello", "HELLo", "hELLO", "I", "OK", "a"] {
            assert_eq!(fix_shift_case(word), word, "{word} should stay unchanged");
        }
    }

    #[test]
    fn keeps_punctuation_spaces_and_other_words() {
        assert_eq!(fix_shift_case("HEllo, WOrld!"), "Hello, World!");
        assert_eq!(fix_shift_case("  THere\n\tNASA rocks "), "  There\n\tNASA rocks ");
        assert_eq!(fix_shift_case("e-MAil"), "e-Mail");
    }

    #[test]
    fn digits_split_runs() {
        assert_eq!(fix_shift_case("2HEllo"), "2Hello");
        assert_eq!(fix_shift_case("HE2llo"), "HE2llo");
    }

    #[test]
    fn empty_and_letterless() {
        assert_eq!(fix_shift_case(""), "");
        assert_eq!(fix_shift_case("123 !?"), "123 !?");
    }
}
