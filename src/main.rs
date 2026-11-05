mod clipboard;
mod kde;
mod layout;
mod portal;

use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use kde::Kde;
use layout::{Converter, Layout, LayoutSpec};

const USAGE: &str = "\
Usage: layout-fix [OPTIONS]

Converts the selected text typed in the wrong keyboard layout and puts the
result into the clipboard.

Options:
      --paste             Paste the result automatically and restore the
                          previous clipboard (asks for permission the first time)
      --paste-delay <MS>  Wait before pasting, so you can release the hotkey
                          [default: 150]
      --switch-layout     Switch the active keyboard layout to the one the text
                          was converted into
  -h, --help              Show this help
  -V, --version           Show the version
";

const DEFAULT_PASTE_DELAY: Duration = Duration::from_millis(150);
/// How long the target app gets to fetch the pasted text before we put the
/// old clipboard back. Restoring too early makes it paste the old content.
const RESTORE_DELAY: Duration = Duration::from_millis(300);

struct Options {
    paste: bool,
    paste_delay: Duration,
    switch_layout: bool,
}

enum Cli {
    Run(Options),
    Help,
    Version,
}

fn parse_ms(value: &str) -> Result<Duration, String> {
    value
        .parse::<u64>()
        .map(Duration::from_millis)
        .map_err(|_| format!("invalid --paste-delay value: {value:?} (expected milliseconds)"))
}

fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<Cli, String> {
    let mut opts = Options {
        paste: false,
        paste_delay: DEFAULT_PASTE_DELAY,
        switch_layout: false,
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--paste" => opts.paste = true,
            "--switch-layout" => opts.switch_layout = true,
            "--paste-delay" => {
                let value = args.next().ok_or("--paste-delay needs a value in milliseconds")?;
                opts.paste_delay = parse_ms(&value)?;
            }
            "-h" | "--help" => return Ok(Cli::Help),
            "-V" | "--version" => return Ok(Cli::Version),
            other => match other.strip_prefix("--paste-delay=") {
                Some(value) => opts.paste_delay = parse_ms(value)?,
                None => return Err(format!("unknown argument: {other}")),
            },
        }
    }
    Ok(Cli::Run(opts))
}

fn preview(text: &str, max_chars: usize) -> String {
    let mut s: String = text.chars().take(max_chars).collect();
    if text.chars().count() > max_chars {
        s.push('…');
    }
    s
}

fn build_converter(kde: &Kde) -> Option<Converter> {
    let specs = kde
        .layout_specs()
        .unwrap_or_else(|| vec![LayoutSpec::new("us", ""), LayoutSpec::new("ru", "")]);
    Converter::new(Layout::compile_all(specs))
}

fn run(opts: &Options, kde: &Kde) {
    let Some(selected) = clipboard::read_primary() else {
        kde.osd("edit-select-none", "Nothing to convert — nothing selected");
        return;
    };

    let Some(converter) = build_converter(kde) else {
        kde.osd("dialog-error", "Need at least two keyboard layouts to convert between");
        return;
    };

    let active = kde.active_layout();
    let Some(conversion) = converter.convert(&selected, active) else {
        kde.osd(
            "dialog-question",
            "Couldn't detect the language — no letters in the selection",
        );
        return;
    };

    // Save the clipboard before overwriting it, but only when we paste
    // ourselves: otherwise the user still needs the result in the clipboard.
    let snapshot = opts.paste.then(clipboard::snapshot);

    if let Err(e) = clipboard::write_text(&conversion.text) {
        eprintln!("[layout-fix] failed to write to clipboard: {e}");
        kde.osd("dialog-error", "Error: failed to write to clipboard");
        return;
    }

    let mut paste_error = None;
    if let Some(snapshot) = snapshot {
        match portal::press_ctrl_v(opts.paste_delay) {
            Ok(()) => {
                thread::sleep(RESTORE_DELAY);
                if let Err(e) = clipboard::restore(&snapshot) {
                    eprintln!("[layout-fix] failed to restore the clipboard: {e}");
                }
            }
            Err(e) => {
                // Leave the converted text in the clipboard so Ctrl+V still works.
                eprintln!("[layout-fix] auto-paste failed: {e}");
                paste_error = Some(e);
            }
        }
    }

    // After pasting: switching first could change what the target app sees.
    if opts.switch_layout && active != Some(conversion.final_target) {
        if !kde.set_layout(conversion.final_target) {
            eprintln!("[layout-fix] failed to switch the keyboard layout");
        }
    }

    match paste_error {
        Some(portal::PasteError::Denied) => kde.osd(
            "dialog-cancel",
            "Auto-paste not allowed — the result is in the clipboard, press Ctrl+V",
        ),
        Some(portal::PasteError::Failed(_)) => kde.osd(
            "dialog-error",
            "Auto-paste failed — the result is in the clipboard, press Ctrl+V",
        ),
        None => kde.osd(
            "input-keyboard",
            &format!("{}: {}", conversion.summary(), preview(&conversion.text, 40)),
        ),
    }
}

fn main() -> ExitCode {
    let opts = match parse_args(std::env::args().skip(1)) {
        Ok(Cli::Run(opts)) => opts,
        Ok(Cli::Help) => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Ok(Cli::Version) => {
            println!("layout-fix {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("layout-fix: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    run(&opts, &Kde::connect());
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, String> {
        parse_args(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn no_flags_is_the_old_behaviour() {
        let Ok(Cli::Run(o)) = parse(&[]) else { panic!() };
        assert!(!o.paste && !o.switch_layout);
        assert_eq!(o.paste_delay, DEFAULT_PASTE_DELAY);
    }

    #[test]
    fn all_flags() {
        let Ok(Cli::Run(o)) = parse(&["--paste", "--switch-layout", "--paste-delay", "300"]) else {
            panic!()
        };
        assert!(o.paste && o.switch_layout);
        assert_eq!(o.paste_delay, Duration::from_millis(300));
    }

    #[test]
    fn paste_delay_with_equals() {
        let Ok(Cli::Run(o)) = parse(&["--paste-delay=50"]) else { panic!() };
        assert_eq!(o.paste_delay, Duration::from_millis(50));
    }

    #[test]
    fn bad_arguments() {
        assert!(parse(&["--nope"]).is_err());
        assert!(parse(&["--paste-delay"]).is_err());
        assert!(parse(&["--paste-delay", "soon"]).is_err());
    }

    #[test]
    fn preview_truncates() {
        assert_eq!(preview("привет", 3), "при…");
        assert_eq!(preview("hi", 3), "hi");
    }
}
