//! Everything that talks to KDE: the OSD popup and the keyboard layout service.

use std::path::PathBuf;

use zbus::blocking::Connection;

use crate::layout::LayoutSpec;

const OSD_DEST: &str = "org.kde.plasmashell";
const OSD_PATH: &str = "/org/kde/osdService";
const OSD_IFACE: &str = "org.kde.osdService";

const KB_DEST: &str = "org.kde.keyboard";
const KB_PATH: &str = "/Layouts";
const KB_IFACE: &str = "org.kde.KeyboardLayouts";

pub struct Kde {
    conn: Option<Connection>,
}

impl Kde {
    /// Connects to the session bus once. If that fails, every method below
    /// quietly does nothing / returns None — the tool still works with defaults.
    pub fn connect() -> Self {
        Self {
            conn: Connection::session().ok(),
        }
    }

    pub fn osd(&self, icon: &str, text: &str) {
        // Not critical if this fails — silently drop the notification, don't crash the program.
        if let Some(conn) = &self.conn {
            let _ = conn.call_method(
                Some(OSD_DEST),
                OSD_PATH,
                Some(OSD_IFACE),
                "showText",
                &(icon, text),
            );
        }
    }

    /// The configured layouts, in KDE's order. Variants come from kxkbrc,
    /// because the D-Bus layout list doesn't include them.
    pub fn layout_specs(&self) -> Option<Vec<LayoutSpec>> {
        read_kxkbrc().or_else(|| {
            let names = self.dbus_layout_names()?;
            Some(names.iter().map(|n| LayoutSpec::new(n, "")).collect())
        })
    }

    fn dbus_layout_names(&self) -> Option<Vec<String>> {
        let conn = self.conn.as_ref()?;
        let reply = conn
            .call_method(Some(KB_DEST), KB_PATH, Some(KB_IFACE), "getLayoutsList", &())
            .ok()?;
        // a(sss): (short name, display name, long name)
        let list: Vec<(String, String, String)> = reply.body().deserialize().ok()?;
        let names: Vec<String> = list.into_iter().map(|(short, _, _)| short).collect();
        (!names.is_empty()).then_some(names)
    }

    /// Index of the currently active layout.
    pub fn active_layout(&self) -> Option<u32> {
        let conn = self.conn.as_ref()?;
        let reply = conn
            .call_method(Some(KB_DEST), KB_PATH, Some(KB_IFACE), "getLayout", &())
            .ok()?;
        reply.body().deserialize::<u32>().ok()
    }

    pub fn set_layout(&self, index: u32) -> bool {
        let Some(conn) = &self.conn else { return false };
        conn.call_method(Some(KB_DEST), KB_PATH, Some(KB_IFACE), "setLayout", &(index,))
            .ok()
            .and_then(|reply| reply.body().deserialize::<bool>().ok())
            .unwrap_or(false)
    }
}

fn read_kxkbrc() -> Option<Vec<LayoutSpec>> {
    let config_dir = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    let text = std::fs::read_to_string(config_dir.join("kxkbrc")).ok()?;
    parse_kxkbrc(&text)
}

/// Reads `LayoutList` and `VariantList` from the `[Layout]` group of kxkbrc:
///
/// ```text
/// [Layout]
/// LayoutList=us,ru
/// VariantList=,winkeys
/// ```
fn parse_kxkbrc(text: &str) -> Option<Vec<LayoutSpec>> {
    let mut in_layout_group = false;
    let mut layouts = None;
    let mut variants = None;

    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_layout_group = line == "[Layout]";
            continue;
        }
        if !in_layout_group {
            continue;
        }
        if let Some(v) = line.strip_prefix("LayoutList=") {
            layouts = Some(v.to_string());
        } else if let Some(v) = line.strip_prefix("VariantList=") {
            variants = Some(v.to_string());
        }
    }

    let layouts = layouts.filter(|l| !l.trim().is_empty())?;
    let variants = variants.unwrap_or_default();
    let variants: Vec<&str> = variants.split(',').collect();

    // Keep every entry (even an odd empty one) so indices match KDE's numbering.
    Some(
        layouts
            .split(',')
            .enumerate()
            .map(|(i, name)| LayoutSpec::new(name, variants.get(i).copied().unwrap_or("")))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kxkbrc_with_variants() {
        let text = "[$Version]\nupdate_info=foo\n\n[Layout]\nDisplayNames=,\nLayoutList=us,ru\nUse=true\nVariantList=,winkeys\n";
        assert_eq!(
            parse_kxkbrc(text).unwrap(),
            vec![LayoutSpec::new("us", ""), LayoutSpec::new("ru", "winkeys")]
        );
    }

    #[test]
    fn kxkbrc_without_variant_list() {
        let text = "[Layout]\nLayoutList=us,ru,ua\n";
        assert_eq!(
            parse_kxkbrc(text).unwrap(),
            vec![
                LayoutSpec::new("us", ""),
                LayoutSpec::new("ru", ""),
                LayoutSpec::new("ua", "")
            ]
        );
    }

    #[test]
    fn kxkbrc_ignores_other_groups() {
        let text = "[Other]\nLayoutList=de\n[Layout]\nUse=false\n";
        assert!(parse_kxkbrc(text).is_none());
    }
}
