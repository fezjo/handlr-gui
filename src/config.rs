//! Round-trip read/write for ~/.config/handlr/handlr.toml.
//! Uses toml_edit so existing [[handlers]] entries are not clobbered.

use anyhow::Context;
use std::path::PathBuf;
use toml_edit::{value, DocumentMut, Item};

#[derive(Debug, Clone)]
pub(crate) struct Config {
    pub enable_selector: bool,
    pub selector: String,
    pub term_exec_args: String,
    pub expand_wildcards: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enable_selector: false,
            selector: String::new(),
            term_exec_args: "-e".into(),
            expand_wildcards: false,
        }
    }
}

fn config_path() -> PathBuf {
    dirs_next_or_manual()
        .join("handlr")
        .join("handlr.toml")
}

fn dirs_next_or_manual() -> PathBuf {
    // XDG_CONFIG_HOME or ~/.config
    if let Ok(p) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(p);
    }
    let mut home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    home.push(".config");
    home
}

pub(crate) fn load() -> anyhow::Result<Config> {
    let path = config_path();
    if !path.exists() {
        return Ok(Config::default());
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))?;
    let doc: DocumentMut = text
        .parse()
        .with_context(|| format!("parsing {}", path.display()))?;

    let mut cfg = Config::default();
    if let Some(v) = doc.get("enable_selector").and_then(Item::as_bool) {
        cfg.enable_selector = v;
    }
    if let Some(v) = doc.get("selector").and_then(Item::as_str) {
        cfg.selector = v.to_string();
    }
    if let Some(v) = doc.get("term_exec_args").and_then(Item::as_str) {
        cfg.term_exec_args = v.to_string();
    }
    if let Some(v) = doc.get("expand_wildcards").and_then(Item::as_bool) {
        cfg.expand_wildcards = v;
    }
    Ok(cfg)
}

pub(crate) fn save(cfg: &Config) -> anyhow::Result<()> {
    let path = config_path();
    // Read existing doc so [[handlers]] is preserved; start fresh if absent.
    let mut doc: DocumentMut = if path.exists() {
        std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?
            .parse()
            .with_context(|| format!("parsing {}", path.display()))?
    } else {
        DocumentMut::new()
    };

    doc["enable_selector"] = value(cfg.enable_selector);
    doc["selector"] = value(cfg.selector.as_str());
    doc["term_exec_args"] = value(cfg.term_exec_args.as_str());
    doc["expand_wildcards"] = value(cfg.expand_wildcards);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating dir {}", parent.display()))?;
    }
    std::fs::write(&path, doc.to_string())
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values() {
        let cfg = Config::default();
        assert!(!cfg.enable_selector);
        assert_eq!(cfg.term_exec_args, "-e");
        assert!(!cfg.expand_wildcards);
        assert!(cfg.selector.is_empty());
    }

    #[test]
    fn round_trip_save_load() {
        let dir = tempfile::tempdir().unwrap();
        // Override config path by temporarily setting XDG_CONFIG_HOME
        // SAFETY: single-threaded test (run with --test-threads=1)
        unsafe { std::env::set_var("XDG_CONFIG_HOME", dir.path()) };

        let cfg = Config {
            enable_selector: true,
            selector: "fzf".into(),
            term_exec_args: "-e".into(),
            expand_wildcards: true,
        };
        save(&cfg).unwrap();

        let loaded = load().unwrap();
        assert_eq!(loaded.enable_selector, cfg.enable_selector);
        assert_eq!(loaded.selector, cfg.selector);
        assert_eq!(loaded.term_exec_args, cfg.term_exec_args);
        assert_eq!(loaded.expand_wildcards, cfg.expand_wildcards);

        // SAFETY: single-threaded test
        unsafe { std::env::remove_var("XDG_CONFIG_HOME") };
    }

    #[test]
    fn save_preserves_handlers_section() {
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: single-threaded test (run with --test-threads=1)
        unsafe { std::env::set_var("XDG_CONFIG_HOME", dir.path()) };

        // Write a file that already has a [[handlers]] block.
        let path = dir.path().join("handlr").join("handlr.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "enable_selector = false\n\n[[handlers]]\nregex = 'https?://'\nhandler = 'firefox.desktop'\n",
        )
        .unwrap();

        let mut cfg = load().unwrap();
        cfg.enable_selector = true;
        save(&cfg).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[[handlers]]"), "handlers section must be preserved");
        assert!(text.contains("enable_selector = true"));

        // SAFETY: single-threaded test
        unsafe { std::env::remove_var("XDG_CONFIG_HOME") };
    }

    #[test]
    fn load_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: single-threaded test (run with --test-threads=1)
        unsafe { std::env::set_var("XDG_CONFIG_HOME", dir.path()) };
        // No file created — load must succeed with defaults.
        let cfg = load().unwrap();
        assert!(!cfg.enable_selector);
        // SAFETY: single-threaded test
        unsafe { std::env::remove_var("XDG_CONFIG_HOME") };
    }
}
