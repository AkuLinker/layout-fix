mod case;
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
      --fix-case          Also fix words where Shift was held too long
                          (HEllo -> Hello, CALling -> Calling)
      --case-only         Only fix those words, without changing the layout
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
    fix_case: bool,
    case_only: bool,
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
        fix_case: false,
        case_only: false,
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--paste" => opts.paste = true,
            "--switch-layout" => opts.switch_layout = true,
            "--fix-case" => opts.fix_case = true,
            "--case-only" => opts.case_only = true,
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

/// The text to put into the clipboard, and how to describe it.
struct Output {
    text: String,
    summary: String,
    /// Layout to switch to with --switch-layout (None in --case-only mode).
    switch_to: Option<u32>,
}

/// Turns the selection into the fixed text, or returns the popup to show
/// (icon, message) if there's nothing to do.
fn transform(
    opts: &Options,
    kde: &Kde,
    selected: &str,
    active: Option<u32>,
) -> Result<Output, (&'static str, &'static str)> {
    if opts.case_only {
        let text = case::fix_shift_case(selected);
        if text == selected {
            return Err(("dialog-information", "Nothing to fix — no word with Shift held too long"));
        }
        return Ok(Output {
            text,
            summary: "Case fixed".to_string(),
            switch_to: None,
        });
    }

    let converter = build_converter(kde).ok_or((
        "dialog-error",
        "Need at least two keyboard layouts to convert between",
    ))?;
    let conversion = converter.convert(selected, active).ok_or((
        "dialog-question",
        "Couldn't detect the language — no letters in the selection",
    ))?;

    let mut summary = conversion.summary();
    let mut text = conversion.text;
    if opts.fix_case {
        let fixed = case::fix_shift_case(&text);
        if fixed != text {
            summary.push_str(", case fixed");
            text = fixed;
        }
    }
    Ok(Output {
        text,
        summary,
        switch_to: Some(conversion.final_target),
    })
}

fn run(opts: &Options, kde: &Kde) {
    let Some(selected) = clipboard::read_primary() else {
        kde.osd("edit-select-none", "Nothing to convert — nothing selected");
        return;
    };

    let active = kde.active_layout();
    let output = match transform(opts, kde, &selected, active) {
        Ok(output) => output,
        Err((icon, message)) => {
            kde.osd(icon, message);
            return;
        }
    };

    // Save the clipboard before overwriting it, but only when we paste
    // ourselves: otherwise the user still needs the result in the clipboard.
    let snapshot = opts.paste.then(clipboard::snapshot);

    if let Err(e) = clipboard::write_text(&output.text) {
        eprintln!("[layout-fix] failed to write to clipboard: {e}");
        kde.osd("dialog-error", "Error: failed to write to clipboard");
        return;
    }

    let mut pasted = None;
    if let Some(snapshot) = snapshot {
        let result = portal::press_ctrl_v(opts.paste_delay);
        match &result {
            Ok(_) => {
                thread::sleep(RESTORE_DELAY);
                if let Err(e) = clipboard::restore(&snapshot) {
                    eprintln!("[layout-fix] failed to restore the clipboard: {e}");
                }
            }
            // Leave the converted text in the clipboard so Ctrl+V still works.
            Err(e) => eprintln!("[layout-fix] auto-paste failed: {e}"),
        }
        pasted = Some(result);
    }

    // After pasting: switching first could change what the target app sees.
    if let Some(target) = output.switch_to {
        if opts.switch_layout && active != Some(target) && !kde.set_layout(target) {
            eprintln!("[layout-fix] failed to switch the keyboard layout");
        }
    }

    match pasted {
        Some(Err(portal::PasteError::Denied)) => kde.osd(
            "dialog-cancel",
            "Auto-paste not allowed — the result is in the clipboard, press Ctrl+V",
        ),
        Some(Err(portal::PasteError::Failed(_))) => kde.osd(
            "dialog-error",
            "Auto-paste failed — the result is in the clipboard, press Ctrl+V",
        ),
        _ => kde.osd(
            "input-keyboard",
            &format!("{}: {}", output.summary, preview(&output.text, 40)),
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
        assert!(!o.paste && !o.switch_layout && !o.fix_case && !o.case_only);
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
    fn case_flags() {
        let Ok(Cli::Run(o)) = parse(&["--fix-case"]) else { panic!() };
        assert!(o.fix_case && !o.case_only);
        let Ok(Cli::Run(o)) = parse(&["--case-only", "--paste"]) else { panic!() };
        assert!(o.case_only && o.paste);
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
