//! Synchronous wrappers around the `handlr` CLI. The UI layer wraps these in
//! `gio::spawn_blocking` to keep the main loop responsive.

use anyhow::{Result, anyhow};
use serde::Deserialize;
use std::path::Path;
use std::process::Command;

#[derive(Debug)]
pub(crate) struct State {
    pub(crate) defaults: Vec<(String, Vec<String>)>,
    pub(crate) system_apps: Vec<App>,
}

#[derive(Debug, Clone)]
pub(crate) struct App {
    pub(crate) desktop: String,
    pub(crate) name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HandlrCmd {
    Set { mime: String, desktop: String },
    Add { mime: String, desktop: String },
    Remove { mime: String, desktop: String },
    Unset { mime: String },
}

#[derive(Deserialize)]
struct RawEntry {
    mime: String,
    handlers: Vec<String>,
}

#[derive(Deserialize)]
struct RawListAll {
    default_apps: Vec<RawEntry>,
    system_apps: Vec<RawEntry>,
}

#[derive(Deserialize)]
struct RawMime {
    mime: String,
}

pub(crate) fn check_present() -> Result<String> {
    let out = Command::new("handlr").arg("--version").output()?;
    if !out.status.success() {
        return Err(anyhow!("handlr --version failed: {}", stderr(&out.stderr)));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub(crate) fn list_all() -> Result<State> {
    let out = Command::new("handlr")
        .args(["list", "--all", "--json"])
        .output()?;
    if !out.status.success() {
        return Err(anyhow!("handlr list failed: {}", stderr(&out.stderr)));
    }
    let raw: RawListAll = serde_json::from_slice(&out.stdout)?;
    Ok(map_state(raw))
}

pub(crate) fn detect_mime(path: &Path) -> Result<String> {
    let out = Command::new("handlr")
        .arg("mime")
        .arg("--json")
        .arg(path)
        .output()?;
    if !out.status.success() {
        return Err(anyhow!("handlr mime failed: {}", stderr(&out.stderr)));
    }
    let raw: Vec<RawMime> = serde_json::from_slice(&out.stdout)?;
    raw.into_iter()
        .next()
        .map(|r| r.mime)
        .ok_or_else(|| anyhow!("handlr mime returned empty array"))
}

pub(crate) fn run(cmd: &HandlrCmd) -> Result<()> {
    let (sub, args) = match cmd {
        HandlrCmd::Set { mime, desktop } => ("set", vec![mime.as_str(), desktop.as_str()]),
        HandlrCmd::Add { mime, desktop } => ("add", vec![mime.as_str(), desktop.as_str()]),
        HandlrCmd::Remove { mime, desktop } => ("remove", vec![mime.as_str(), desktop.as_str()]),
        HandlrCmd::Unset { mime } => ("unset", vec![mime.as_str()]),
    };
    let out = Command::new("handlr").arg(sub).args(&args).output()?;
    if !out.status.success() {
        return Err(anyhow!("handlr {} failed: {}", sub, stderr(&out.stderr)));
    }
    Ok(())
}

fn stderr(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    let trimmed = s.trim_end_matches('\n').trim_end();
    let cap = 200;
    if trimmed.chars().count() > cap {
        let capped: String = trimmed.chars().take(cap).collect();
        format!("{}…", capped)
    } else {
        trimmed.to_string()
    }
}

fn map_state(raw: RawListAll) -> State {
    let defaults = raw
        .default_apps
        .into_iter()
        .map(|e| (e.mime, e.handlers))
        .collect();

    // handlr emits the same .desktop multiple times across MIME entries; entries are identical.
    let mut seen = std::collections::BTreeSet::new();
    let mut system_apps = Vec::new();
    for entry in raw.system_apps {
        for desktop in entry.handlers {
            if seen.insert(desktop.clone()) {
                let name = humanize(&desktop);
                system_apps.push(App { desktop, name });
            }
        }
    }
    system_apps.sort_by_key(|a| a.name.to_lowercase());

    State { defaults, system_apps }
}

// TODO(module-3): replace humanize() with gio::DesktopAppInfo::from_filename().name() once we're in GTK context.
fn humanize(desktop: &str) -> String {
    let stem = desktop.strip_suffix(".desktop").unwrap_or(desktop);
    let last = stem.rsplit('.').next().unwrap_or(stem);
    let mut chars = last.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST_ALL_FIXTURE: &str = r#"{
        "added_associations": [
            {"mime": "x-scheme-handler/terminal", "handlers": ["org.wezfurlong.wezterm.desktop"]}
        ],
        "default_apps": [
            {"mime": "audio/*", "handlers": ["mpv.desktop"]},
            {"mime": "video/*", "handlers": ["mpv.desktop", "vlc.desktop"]},
            {"mime": "video/mp4", "handlers": ["vlc.desktop"]},
            {"mime": "text/*", "handlers": ["helix.desktop"]},
            {"mime": "inode/directory", "handlers": ["org.gnome.Nautilus.desktop"]}
        ],
        "system_apps": [
            {"mime": "video/mp4", "handlers": ["mpv.desktop", "vlc.desktop"]},
            {"mime": "audio/mpeg", "handlers": ["mpv.desktop"]},
            {"mime": "text/plain", "handlers": ["org.gnome.TextEditor.desktop", "helix.desktop"]}
        ]
    }"#;

    const MIME_FIXTURE: &str = r#"[{"path": "README.md", "mime": "text/markdown"}]"#;

    #[test]
    fn parses_list_all() {
        let raw: RawListAll = serde_json::from_str(LIST_ALL_FIXTURE).unwrap();
        let state = map_state(raw);

        assert_eq!(state.defaults.len(), 5);
        assert_eq!(state.defaults[0].0, "audio/*");
        assert_eq!(state.defaults[0].1, vec!["mpv.desktop"]);

        let video_star = state.defaults.iter().find(|(m, _)| m == "video/*").unwrap();
        assert_eq!(video_star.1, vec!["mpv.desktop", "vlc.desktop"]);

        let video_mp4 = state.defaults.iter().find(|(m, _)| m == "video/mp4").unwrap();
        assert_eq!(video_mp4.1, vec!["vlc.desktop"]);
    }

    #[test]
    fn system_apps_deduped_and_named() {
        let raw: RawListAll = serde_json::from_str(LIST_ALL_FIXTURE).unwrap();
        let state = map_state(raw);

        let desktops: Vec<&str> = state.system_apps.iter().map(|a| a.desktop.as_str()).collect();
        assert_eq!(desktops.len(), 4, "mpv/vlc duplicates should collapse, got {:?}", desktops);
        assert!(desktops.contains(&"mpv.desktop"));
        assert!(desktops.contains(&"vlc.desktop"));
        assert!(desktops.contains(&"helix.desktop"));
        assert!(desktops.contains(&"org.gnome.TextEditor.desktop"));

        let textedit = state
            .system_apps
            .iter()
            .find(|a| a.desktop == "org.gnome.TextEditor.desktop")
            .unwrap();
        assert_eq!(textedit.name, "TextEditor");

        let mpv = state.system_apps.iter().find(|a| a.desktop == "mpv.desktop").unwrap();
        assert_eq!(mpv.name, "Mpv");
    }

    #[test]
    fn parses_mime_json() {
        let raw: Vec<RawMime> = serde_json::from_str(MIME_FIXTURE).unwrap();
        assert_eq!(raw.len(), 1);
        assert_eq!(raw[0].mime, "text/markdown");
    }

    #[test]
    fn humanize_strips_reverse_dns() {
        assert_eq!(humanize("firefox.desktop"), "Firefox");
        assert_eq!(humanize("org.gnome.Nautilus.desktop"), "Nautilus");
        assert_eq!(humanize("vlc.desktop"), "Vlc");
    }

    #[test]
    fn stderr_caps_long_output() {
        let big = vec![b'x'; 1000];
        let result = stderr(&big);
        assert_eq!(result.chars().count(), 201); // 200 x's plus ellipsis
        assert!(result.ends_with('…'));
    }

    #[test]
    fn stderr_trims_trailing_newlines() {
        assert_eq!(stderr(b"boom\n\n"), "boom");
    }
}
