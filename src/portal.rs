//! Pressing Ctrl+V through the XDG RemoteDesktop portal.
//!
//! This is the standard, permission-based way for a Wayland program to send
//! keyboard input. The first time, KDE asks whether layout-fix may control the
//! keyboard. If you allow it and keep "remember" checked, the portal gives us
//! a restore token; we pass it back next time and no dialog is shown.
//!
//! Two details make "ask only once" work:
//! - Restore tokens are single-use. Every successful start returns a fresh
//!   token, which we save over the old one.
//! - The permission is stored per application ID. Programs that aren't
//!   Flatpaks get an ID guessed from how they were launched (empty from a
//!   terminal, something else from a KDE shortcut), so we register a fixed
//!   ID first. It has to match a .desktop file name, which the README's
//!   install steps put in place.

use std::cell::Cell;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

/// Must match the .desktop file installed as `<APP_ID>.desktop`.
pub const APP_ID: &str = "io.github.akulinker.LayoutFix";

const PORTAL_DEST: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const REGISTRY_IFACE: &str = "org.freedesktop.host.portal.Registry";
const REMOTE_DESKTOP_IFACE: &str = "org.freedesktop.portal.RemoteDesktop";
const REQUEST_IFACE: &str = "org.freedesktop.portal.Request";
const SESSION_IFACE: &str = "org.freedesktop.portal.Session";

const DEVICE_KEYBOARD: u32 = 1;
const PERSIST_UNTIL_REVOKED: u32 = 2;

// Linux input event codes.
const KEY_LEFTCTRL: i32 = 29;
const KEY_V: i32 = 47;

/// How long to wait for the portal, including the first-time dialog.
const PORTAL_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug)]
pub enum PasteError {
    /// The user said no in the permission dialog.
    Denied,
    Failed(String),
}

impl fmt::Display for PasteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PasteError::Denied => write!(f, "keyboard control was not allowed"),
            PasteError::Failed(msg) => write!(f, "{msg}"),
        }
    }
}

fn failed(e: impl fmt::Display) -> PasteError {
    PasteError::Failed(e.to_string())
}

/// Presses Ctrl+V through the portal, after waiting `delay` (so the user can
/// release the hotkey). Gives up after a timeout instead of hanging forever.
pub fn press_ctrl_v(delay: Duration) -> Result<(), PasteError> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(press_ctrl_v_blocking(delay));
    });
    rx.recv_timeout(PORTAL_TIMEOUT + delay)
        .unwrap_or_else(|_| Err(failed("timed out waiting for the RemoteDesktop portal")))
}

fn press_ctrl_v_blocking(delay: Duration) -> Result<(), PasteError> {
    let portal = Portal::connect()?;
    let session = portal.start_keyboard_session()?;
    thread::sleep(delay);
    session.press_ctrl_v()
    // `session` is dropped here, which closes the portal session.
}

struct Portal {
    conn: Connection,
    /// Our unique bus name in the form used inside request paths: ":1.42" -> "1_42".
    sender: String,
    counter: Cell<u32>,
}

impl Portal {
    fn connect() -> Result<Self, PasteError> {
        let conn = Connection::session().map_err(failed)?;

        // Has to be the first portal call on this connection.
        if let Err(e) = conn.call_method(
            Some(PORTAL_DEST),
            PORTAL_PATH,
            Some(REGISTRY_IFACE),
            "Register",
            &(APP_ID, HashMap::<&str, Value>::new()),
        ) {
            // Older xdg-desktop-portal without the Registry: still works, but
            // the permission may not be remembered between launch methods.
            eprintln!("[layout-fix] couldn't register app id with the portal: {e}");
        }

        let sender = conn
            .unique_name()
            .ok_or_else(|| failed("no unique D-Bus name"))?
            .as_str()
            .trim_start_matches(':')
            .replace('.', "_");

        Ok(Self {
            conn,
            sender,
            counter: Cell::new(0),
        })
    }

    /// Subscribes to the Response signal of the Request object at `path`.
    /// Takes the path by value so the returned iterator doesn't borrow it.
    fn responses_for(&self, path: String) -> Result<impl Iterator<Item = zbus::Message>, PasteError> {
        let proxy = Proxy::new(&self.conn, PORTAL_DEST, path, REQUEST_IFACE).map_err(failed)?;
        proxy.receive_signal("Response").map_err(failed)
    }

    /// Calls a portal method that answers asynchronously through a Request
    /// object, and waits for its Response. `call` gets the handle_token to use.
    fn request(
        &self,
        call: impl FnOnce(&Connection, &str) -> zbus::Result<zbus::Message>,
    ) -> Result<HashMap<String, OwnedValue>, PasteError> {
        let n = self.counter.get() + 1;
        self.counter.set(n);
        let token = format!("layoutfix_{}_{n}", std::process::id());
        let expected = format!("{PORTAL_PATH}/request/{}/{token}", self.sender);

        // Subscribe *before* calling, so the Response can't arrive unseen.
        let responses = self.responses_for(expected.clone())?;
        let reply = call(&self.conn, &token).map_err(failed)?;
        let handle: OwnedObjectPath = reply.body().deserialize().map_err(failed)?;

        let mut responses = if handle.as_str() == expected {
            responses
        } else {
            // Very old portals don't use the predictable path.
            self.responses_for(handle.as_str().to_owned())?
        };

        let msg = responses
            .next()
            .ok_or_else(|| failed("the portal closed the request without answering"))?;
        let (code, results): (u32, HashMap<String, OwnedValue>) =
            msg.body().deserialize().map_err(failed)?;
        match code {
            0 => Ok(results),
            1 => Err(PasteError::Denied),
            _ => Err(failed(format!("the portal request failed (code {code})"))),
        }
    }

    fn start_keyboard_session(&self) -> Result<Session<'_>, PasteError> {
        // 1. CreateSession
        let results = self.request(|conn, token| {
            let options = HashMap::from([
                ("handle_token", Value::from(token)),
                ("session_handle_token", Value::from(token)),
            ]);
            conn.call_method(
                Some(PORTAL_DEST),
                PORTAL_PATH,
                Some(REMOTE_DESKTOP_IFACE),
                "CreateSession",
                &(options,),
            )
        })?;
        let session_path = results
            .get("session_handle")
            .and_then(as_string)
            .ok_or_else(|| failed("the portal didn't return a session"))?;
        let session = Session {
            conn: &self.conn,
            path: OwnedObjectPath::try_from(session_path).map_err(failed)?,
        };

        // 2. SelectDevices: keyboard only, remember the permission, and pass
        //    the saved token so no dialog is needed.
        let saved_token = load_token();
        self.request(|conn, token| {
            let mut options = HashMap::from([
                ("handle_token", Value::from(token)),
                ("types", Value::from(DEVICE_KEYBOARD)),
                ("persist_mode", Value::from(PERSIST_UNTIL_REVOKED)),
            ]);
            if let Some(saved) = &saved_token {
                options.insert("restore_token", Value::from(saved.as_str()));
            }
            conn.call_method(
                Some(PORTAL_DEST),
                PORTAL_PATH,
                Some(REMOTE_DESKTOP_IFACE),
                "SelectDevices",
                &(&session.path, options),
            )
        })?;

        // 3. Start: shows the dialog only if the token was missing or invalid.
        let results = self.request(|conn, token| {
            let options = HashMap::from([("handle_token", Value::from(token))]);
            conn.call_method(
                Some(PORTAL_DEST),
                PORTAL_PATH,
                Some(REMOTE_DESKTOP_IFACE),
                "Start",
                &(&session.path, "", options),
            )
        })?;

        // The old token is used up now; keep the new one (or none, if
        // "remember" was unchecked).
        save_token(results.get("restore_token").and_then(as_string).as_deref());

        let devices = results.get("devices").and_then(as_u32).unwrap_or(0);
        if devices & DEVICE_KEYBOARD == 0 {
            return Err(failed("the portal didn't grant keyboard access"));
        }

        Ok(session)
    }
}

struct Session<'a> {
    conn: &'a Connection,
    path: OwnedObjectPath,
}

impl Session<'_> {
    fn key(&self, code: i32, pressed: bool) -> Result<(), PasteError> {
        self.conn
            .call_method(
                Some(PORTAL_DEST),
                PORTAL_PATH,
                Some(REMOTE_DESKTOP_IFACE),
                "NotifyKeyboardKeycode",
                &(&self.path, HashMap::<&str, Value>::new(), code, u32::from(pressed)),
            )
            .map(|_| ())
            .map_err(failed)
    }

    fn press_ctrl_v(&self) -> Result<(), PasteError> {
        self.key(KEY_LEFTCTRL, true)?;
        let result = self.key(KEY_V, true).and_then(|()| self.key(KEY_V, false));
        // Release Ctrl even if V failed, so it doesn't stay "held".
        let release = self.key(KEY_LEFTCTRL, false);
        result.and(release)
    }
}

impl Drop for Session<'_> {
    fn drop(&mut self) {
        let _ = self.conn.call_method(
            Some(PORTAL_DEST),
            self.path.as_str(),
            Some(SESSION_IFACE),
            "Close",
            &(),
        );
    }
}

/// The portal sends `session_handle` as a string even though it's an object
/// path, so accept both. Also used for `restore_token`.
fn as_string(value: &OwnedValue) -> Option<String> {
    match &**value {
        Value::Str(s) => Some(s.as_str().to_owned()),
        Value::ObjectPath(p) => Some(p.as_str().to_owned()),
        _ => None,
    }
}

fn as_u32(value: &OwnedValue) -> Option<u32> {
    match &**value {
        Value::U32(n) => Some(*n),
        _ => None,
    }
}

/// `$XDG_STATE_HOME/layout-fix/remote-desktop-token`, or `~/.local/state/…`.
fn token_path() -> Option<PathBuf> {
    let state_dir = std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(state_dir.join("layout-fix").join("remote-desktop-token"))
}

fn load_token() -> Option<String> {
    let token = fs::read_to_string(token_path()?).ok()?;
    let token = token.trim();
    (!token.is_empty()).then(|| token.to_string())
}

fn save_token(token: Option<&str>) {
    let Some(path) = token_path() else { return };
    let result = match token {
        Some(token) => path
            .parent()
            .map_or(Ok(()), |dir| fs::create_dir_all(dir))
            .and_then(|()| {
                // Readable only by you: the token grants keyboard control.
                fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .mode(0o600)
                    .open(&path)
            })
            .and_then(|mut file| file.write_all(token.as_bytes())),
        None => match fs::remove_file(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        },
    };
    if let Err(e) = result {
        eprintln!("[layout-fix] couldn't save the portal token: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_string_and_object_path() {
        let s = OwnedValue::try_from(Value::from("/org/freedesktop/portal/desktop/session/1_2/x")).unwrap();
        assert_eq!(as_string(&s).as_deref(), Some("/org/freedesktop/portal/desktop/session/1_2/x"));

        let p = zbus::zvariant::ObjectPath::try_from("/a/b").unwrap();
        let p = OwnedValue::try_from(Value::from(p)).unwrap();
        assert_eq!(as_string(&p).as_deref(), Some("/a/b"));

        let n = OwnedValue::try_from(Value::from(5u32)).unwrap();
        assert_eq!(as_u32(&n), Some(5));
        assert_eq!(as_string(&n), None);
    }

    #[test]
    fn app_id_is_valid() {
        // App IDs follow D-Bus well-known name rules: dot-separated,
        // at least two elements, letters/digits/underscore.
        let parts: Vec<&str> = APP_ID.split('.').collect();
        assert!(parts.len() >= 2);
        assert!(parts.iter().all(|p| !p.is_empty()
            && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !p.starts_with(|c: char| c.is_ascii_digit())));
    }
}
