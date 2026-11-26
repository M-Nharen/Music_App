//! Keeping the Spotify login between runs, safely.
//!
//! - **Keyring** (preferred): the desktop's Secret Service (GNOME Keyring,
//!   KeePassXC, KWallet's Secret Service) through `secret-tool` from
//!   `libsecret`. Nothing on disk in plain text.
//! - **File** (fallback): `~/.local/share/qwertone/spotify-login.json`,
//!   readable by you only (0600, folder 0700), written atomically.
//!
//! `config.toml` never holds tokens. "auto" tries the keyring and falls
//! back to the file when no keyring answers (e.g. a bare Hyprland session
//! without gnome-keyring). Logging out deletes both.

use super::auth::Tokens;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const ATTRS: [&str; 4] = ["application", "qwertone", "service", "spotify"];
const LABEL: &str = "Qwertone: Spotify login";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Where {
    Keyring,
    File,
}

impl Where {
    pub fn describe(&self) -> String {
        match self {
            Where::Keyring => "in your desktop keyring".into(),
            Where::File => format!("in {} (only you can read it)", file_path().display()),
        }
    }
}

pub fn file_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("qwertone")
        .join("spotify-login.json")
}

fn secret_tool() -> bool {
    Command::new("secret-tool")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

fn keyring_load() -> Option<Tokens> {
    let out = Command::new("secret-tool")
        .arg("lookup")
        .args(ATTRS)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    serde_json::from_slice(&out.stdout).ok()
}

fn keyring_save(t: &Tokens) -> Result<(), String> {
    let mut child = Command::new("secret-tool")
        .args(["store", "--label", LABEL])
        .args(ATTRS)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("secret-tool: {e}"))?;
    let body = serde_json::to_string(t).map_err(|e| e.to_string())?;
    // The secret goes through stdin, never the command line.
    child
        .stdin
        .take()
        .ok_or("secret-tool: no stdin")?
        .write_all(body.as_bytes())
        .map_err(|e| format!("secret-tool: {e}"))?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "the keyring refused it: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

fn keyring_clear() {
    let _ = Command::new("secret-tool")
        .arg("clear")
        .args(ATTRS)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn file_load_from(path: &std::path::Path) -> Option<Tokens> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Write a private file (0600 in a 0700 folder), atomically.
pub fn write_private(path: &std::path::Path, body: &[u8]) -> Result<(), String> {
    let dir = path.parent().ok_or("bad path")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp).map_err(|e| e.to_string())?;
    f.write_all(body).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())?;
    drop(f);
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

fn file_save_to(path: &std::path::Path, t: &Tokens) -> Result<(), String> {
    let body = serde_json::to_vec_pretty(t).map_err(|e| e.to_string())?;
    write_private(path, &body)
}

/// Load the saved login (keyring first, then the file).
pub fn load(pref: &str) -> Option<(Tokens, Where)> {
    if pref != "file"
        && secret_tool()
        && let Some(t) = keyring_load()
    {
        return Some((t, Where::Keyring));
    }
    if pref != "keyring"
        && let Some(t) = file_load_from(&file_path())
    {
        return Some((t, Where::File));
    }
    None
}

/// Save the login. `pref` is "auto", "keyring" or "file".
pub fn save(pref: &str, t: &Tokens) -> Result<Where, String> {
    if pref != "file" {
        let res = if secret_tool() {
            keyring_save(t)
        } else {
            Err("secret-tool isn't installed (sudo pacman -S libsecret)".into())
        };
        match res {
            Ok(()) => {
                // Don't leave an old copy in the file.
                let _ = std::fs::remove_file(file_path());
                return Ok(Where::Keyring);
            }
            Err(e) if pref == "keyring" => return Err(e),
            Err(_) => {}
        }
    }
    file_save_to(&file_path(), t)?;
    Ok(Where::File)
}

/// Log out: forget it everywhere.
pub fn clear() {
    if secret_tool() {
        keyring_clear();
    }
    let _ = std::fs::remove_file(file_path());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_file_round_trip() {
        let dir = std::env::temp_dir().join(format!("qwertone-store-{}", std::process::id()));
        let path = dir.join("sub").join("login.json");
        let t = Tokens {
            access_token: "a".into(),
            refresh_token: "r".into(),
            expires_at: 5,
            scope: "s".into(),
            client_id: "c".into(),
        };
        file_save_to(&path, &t).unwrap();
        assert_eq!(file_load_from(&path), Some(t.clone()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let m = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(m & 0o777, 0o600);
            let d = std::fs::metadata(path.parent().unwrap()).unwrap().permissions().mode();
            assert_eq!(d & 0o777, 0o700);
        }
        // Overwrite works (atomic rename over the old file).
        let mut t2 = t.clone();
        t2.access_token = "b".into();
        file_save_to(&path, &t2).unwrap();
        assert_eq!(file_load_from(&path).unwrap().access_token, "b");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
