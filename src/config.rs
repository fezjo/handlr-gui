//! Round-trip read/write for ~/.config/handlr/handlr.toml.
//! Uses toml_edit so existing [[handlers]] entries are not clobbered.

use anyhow::Context;
use std::path::{Path, PathBuf};
use toml_edit::{value, DocumentMut, Item};

#[derive(Debug, Clone)]
pub(crate) struct Config {
    pub enable_selector: bool,
    pub selector: String,
    pub term_exec_args: String,
    pub expand_wildcards: bool,
    pub handlers: Vec<RegexHandler>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enable_selector: false,
            selector: String::new(),
            term_exec_args: "-e".into(),
            expand_wildcards: false,
            handlers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RegexHandler {
    pub regexes: Vec<String>,
    pub exec: String,
    pub terminal: bool,
}

fn config_dir() -> anyhow::Result<PathBuf> {
    if let Ok(p) = std::env::var("XDG_CONFIG_HOME")
        && !p.is_empty()
    {
        return Ok(PathBuf::from(p));
    }
    let home = std::env::var("HOME")
        .map_err(|_| anyhow::anyhow!("HOME environment variable not set"))?;
    if home.is_empty() {
        anyhow::bail!("HOME environment variable is empty");
    }
    Ok(PathBuf::from(home).join(".config"))
}

fn config_path() -> anyhow::Result<PathBuf> {
    Ok(config_dir()?.join("handlr").join("handlr.toml"))
}

pub(crate) fn load() -> anyhow::Result<Config> {
    load_from(&config_path()?)
}

pub(crate) fn save(cfg: &Config) -> anyhow::Result<()> {
    save_to(cfg, &config_path()?)
}

fn load_from(path: &Path) -> anyhow::Result<Config> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
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
    if let Some(aot) = doc.get("handlers").and_then(Item::as_array_of_tables) {
        for t in aot {
            let regexes: Vec<String> = t
                .get("regexes")
                .and_then(Item::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let exec = t
                .get("exec")
                .and_then(Item::as_str)
                .unwrap_or("")
                .to_string();
            let terminal = t
                .get("terminal")
                .and_then(Item::as_bool)
                .unwrap_or(false);
            if !regexes.is_empty() || !exec.is_empty() {
                cfg.handlers.push(RegexHandler { regexes, exec, terminal });
            }
        }
    }
    Ok(cfg)
}

fn save_to(cfg: &Config, path: &Path) -> anyhow::Result<()> {
    // Read existing doc so [[handlers]] is preserved; start fresh if absent.
    let mut doc: DocumentMut = match std::fs::read_to_string(path) {
        Ok(t) => t
            .parse()
            .with_context(|| format!("parsing {}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };

    doc["enable_selector"] = value(cfg.enable_selector);
    doc["selector"] = value(cfg.selector.as_str());
    doc["term_exec_args"] = value(cfg.term_exec_args.as_str());
    doc["expand_wildcards"] = value(cfg.expand_wildcards);

    // Rebuild [[handlers]] array-of-tables from cfg.handlers.
    doc.remove("handlers");
    if !cfg.handlers.is_empty() {
        let mut aot = toml_edit::ArrayOfTables::new();
        for h in &cfg.handlers {
            let mut t = toml_edit::Table::new();
            let mut regexes_arr = toml_edit::Array::new();
            for r in &h.regexes {
                regexes_arr.push(r.as_str());
            }
            t.insert(
                "regexes",
                toml_edit::Item::Value(toml_edit::Value::Array(regexes_arr)),
            );
            t.insert("exec", toml_edit::value(h.exec.as_str()));
            t.insert("terminal", toml_edit::value(h.terminal));
            aot.push(t);
        }
        doc["handlers"] = toml_edit::Item::ArrayOfTables(aot);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating dir {}", parent.display()))?;
    }
    std::fs::write(path, doc.to_string())
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
        assert!(cfg.handlers.is_empty());
    }

    #[test]
    fn round_trip_save_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("handlr.toml");

        let cfg = Config {
            enable_selector: true,
            selector: "fzf".into(),
            term_exec_args: "-e".into(),
            expand_wildcards: true,
            handlers: vec![RegexHandler {
                regexes: vec!["https://youtu\\.be/.*".into()],
                exec: "freetube %u".into(),
                terminal: false,
            }],
        };
        save_to(&cfg, &path).unwrap();

        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded.enable_selector, cfg.enable_selector);
        assert_eq!(loaded.selector, cfg.selector);
        assert_eq!(loaded.term_exec_args, cfg.term_exec_args);
        assert_eq!(loaded.expand_wildcards, cfg.expand_wildcards);
        assert_eq!(loaded.handlers.len(), 1);
        assert_eq!(loaded.handlers[0].regexes, cfg.handlers[0].regexes);
        assert_eq!(loaded.handlers[0].exec, cfg.handlers[0].exec);
        assert_eq!(loaded.handlers[0].terminal, cfg.handlers[0].terminal);
    }

    #[test]
    fn handlers_round_trip_multiple() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("handlr.toml");

        let cfg = Config {
            handlers: vec![
                RegexHandler {
                    regexes: vec!["https://youtu\\.be/.*".into(), "https://youtube\\.com/.*".into()],
                    exec: "freetube %u".into(),
                    terminal: false,
                },
                RegexHandler {
                    regexes: vec!["magnet:.*".into()],
                    exec: "transmission-gtk %u".into(),
                    terminal: false,
                },
            ],
            ..Default::default()
        };
        save_to(&cfg, &path).unwrap();
        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded.handlers.len(), 2);
        assert_eq!(loaded.handlers[0].regexes.len(), 2);
        assert_eq!(loaded.handlers[1].exec, "transmission-gtk %u");
        assert_eq!(loaded.handlers[0].regexes[0], "https://youtu\\.be/.*");
        assert_eq!(loaded.handlers[0].exec, "freetube %u");
        assert!(!loaded.handlers[0].terminal);
    }

    #[test]
    fn handlers_empty_vec_removes_section() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("handlr.toml");

        // Write a file with handlers, then save with empty list.
        let with_handlers = Config {
            handlers: vec![RegexHandler {
                regexes: vec!["https://.*".into()],
                exec: "firefox %u".into(),
                terminal: false,
            }],
            ..Default::default()
        };
        save_to(&with_handlers, &path).unwrap();

        let empty = Config { handlers: vec![], ..Default::default() };
        save_to(&empty, &path).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("[[handlers]]"), "handlers section should be absent");
    }

    #[test]
    fn terminal_flag_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("handlr.toml");
        let cfg = Config {
            handlers: vec![RegexHandler {
                regexes: vec!["magnet:.*".into()],
                exec: "transmission-cli %u".into(),
                terminal: true,
            }],
            ..Default::default()
        };
        save_to(&cfg, &path).unwrap();
        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded.handlers.len(), 1);
        assert!(loaded.handlers[0].terminal);
    }

    #[test]
    fn load_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("handlr.toml");
        let cfg = load_from(&path).unwrap();
        assert!(!cfg.enable_selector);
        assert_eq!(cfg.term_exec_args, "-e");
        assert!(cfg.handlers.is_empty());
    }
}
