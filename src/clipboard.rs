//! Clipboard access through wl-clipboard.

use std::io::{self, Write};
use std::process::{Command, Stdio};

/// MIME type password managers use to tell clipboard managers (Klipper,
/// cliphist, …) "don't store this in history". `wl-copy --sensitive` sets it.
const SENSITIVE_HINT: &str = "x-kde-passwordManagerHint";

/// Reads the PRIMARY selection (whatever is selected with the mouse/Shift+arrows).
pub fn read_primary() -> Option<String> {
    let output = Command::new("wl-paste")
        .args(["--primary", "--no-newline"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    (!text.is_empty()).then_some(text)
}

/// Runs wl-copy with `args`, feeding `data` on stdin. Ok(false) = wl-copy failed.
fn wl_copy(args: &[&str], data: &[u8]) -> io::Result<bool> {
    let mut child = Command::new("wl-copy")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        match stdin.write_all(data) {
            Ok(()) => {}
            // wl-copy already exited (e.g. it rejected an option); its exit
            // status below tells us what happened.
            Err(e) if e.kind() == io::ErrorKind::BrokenPipe => {}
            Err(e) => return Err(e),
        }
        // stdin is dropped (closed) at the end of this block, which sends EOF
        // to wl-copy. Without that, wl-copy waits forever for more data and
        // never forks into the background, while we sit blocked in wait().
    }

    Ok(child.wait()?.success())
}

/// Puts the converted text into the regular clipboard, marked as sensitive so
/// clipboard managers don't save it in their history.
pub fn write_text(text: &str) -> io::Result<()> {
    if wl_copy(&["--sensitive"], text.as_bytes())? {
        return Ok(());
    }
    // wl-clipboard older than 2.3 doesn't know --sensitive; copy without the hint.
    if wl_copy(&[], text.as_bytes())? {
        Ok(())
    } else {
        Err(io::Error::other("wl-copy exited with an error"))
    }
}

/// What was in the clipboard before we touched it.
pub enum Snapshot {
    /// The clipboard was empty; restoring means clearing it.
    Empty,
    Data {
        mime: String,
        bytes: Vec<u8>,
        sensitive: bool,
    },
    /// Couldn't read it; restoring does nothing rather than risk damage.
    Unknown,
}

fn is_text_mime(mime: &str) -> bool {
    mime.starts_with("text/plain") || matches!(mime, "UTF8_STRING" | "STRING" | "TEXT")
}

/// Plain text if offered, otherwise the first real MIME type (image/png, …).
fn pick_mime(types: &[String]) -> Option<String> {
    const TEXT_PREFERENCE: [&str; 3] = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"];
    TEXT_PREFERENCE
        .into_iter()
        .find(|pref| types.iter().any(|t| t == pref))
        .map(|s| s.to_string())
        .or_else(|| {
            types
                .iter()
                .find(|t| t.contains('/') && !t.starts_with("x-kde-"))
                .cloned()
        })
}

pub fn snapshot() -> Snapshot {
    let Ok(out) = Command::new("wl-paste").arg("--list-types").output() else {
        return Snapshot::Unknown;
    };
    let types: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect();
    // wl-paste exits with an error when nothing is copied.
    if types.is_empty() {
        return if out.status.success() {
            Snapshot::Unknown
        } else {
            Snapshot::Empty
        };
    }

    let sensitive = types.iter().any(|t| t == SENSITIVE_HINT);
    let Some(mime) = pick_mime(&types) else {
        return Snapshot::Unknown;
    };
    let Ok(out) = Command::new("wl-paste")
        .args(["--no-newline", "--type", mime.as_str()])
        .output()
    else {
        return Snapshot::Unknown;
    };
    if !out.status.success() {
        return Snapshot::Unknown;
    }

    Snapshot::Data {
        mime,
        bytes: out.stdout,
        sensitive,
    }
}

/// Puts the saved clipboard back. Only one MIME type is restored (plain text
/// for text, the original type for images and other data).
pub fn restore(snapshot: &Snapshot) -> io::Result<()> {
    match snapshot {
        Snapshot::Unknown => Ok(()),
        Snapshot::Empty => {
            Command::new("wl-copy").arg("--clear").status()?;
            Ok(())
        }
        Snapshot::Data {
            mime,
            bytes,
            sensitive,
        } => {
            let mut args: Vec<&str> = Vec::new();
            // For text, let wl-copy offer all the usual text types itself.
            if !is_text_mime(mime) {
                args.extend(["--type", mime.as_str()]);
            }
            // If it came from a password manager, keep it out of history again.
            if *sensitive {
                args.push("--sensitive");
            }
            if wl_copy(&args, bytes)? {
                Ok(())
            } else {
                Err(io::Error::other("wl-copy exited with an error"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn prefers_utf8_text() {
        let t = types(&["text/html", "UTF8_STRING", "text/plain;charset=utf-8"]);
        assert_eq!(pick_mime(&t).as_deref(), Some("text/plain;charset=utf-8"));
    }

    #[test]
    fn falls_back_to_first_real_type() {
        let t = types(&["x-kde-passwordManagerHint", "SAVE_TARGETS", "image/png"]);
        assert_eq!(pick_mime(&t).as_deref(), Some("image/png"));
    }

    #[test]
    fn text_mime_detection() {
        assert!(is_text_mime("text/plain;charset=utf-8"));
        assert!(is_text_mime("UTF8_STRING"));
        assert!(!is_text_mime("image/png"));
        assert!(!is_text_mime("text/html"));
    }
}
