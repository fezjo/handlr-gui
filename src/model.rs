//! Tree-row data model and owning AppState. See spec §3 for the tree layout and §11 for
//! the always-on seed categories.

use crate::handlr;
use crate::undo::{self, UndoEntry};
use anyhow::Result;
use std::collections::{BTreeMap, VecDeque};

#[derive(Debug, Clone)]
pub(crate) enum Row {
    Category { mime: String, handlers: Vec<String> },
    Exception { mime: String, handlers: Vec<String> },
    Handler {
        mime: String,
        desktop: String,
        index: usize,
        // Reserved for future "N of M" badge rendering; kept in the variant so the
        // builder in model.rs already populates it.
        #[allow(dead_code)]
        total: usize,
    },
}

pub(crate) const SEED_CATEGORIES: &[&str] = &[
    "audio/*",
    "video/*",
    "image/*",
    "text/*",
    "application/*",
    "inode/*",
    "x-scheme-handler/*",
];

// Exact-MIME entries that always appear as exception rows (with empty handlers when not
// configured), so users can find and set them without knowing the MIME name.
const SEED_EXCEPTIONS: &[&str] = &[
    "x-scheme-handler/http",
    "x-scheme-handler/https",
    "x-scheme-handler/mailto",
    "x-scheme-handler/magnet",
];

pub(crate) fn build_categories(state: &handlr::State) -> Vec<Row> {
    let mut map: BTreeMap<&str, &Vec<String>> = BTreeMap::new();
    for (mime, handlers) in &state.defaults {
        if mime.ends_with("/*") {
            map.insert(mime.as_str(), handlers);
        }
    }
    let empty: Vec<String> = Vec::new();
    for seed in SEED_CATEGORIES {
        map.entry(seed).or_insert(&empty);
    }
    map.into_iter()
        .map(|(mime, handlers)| Row::Category {
            mime: mime.to_string(),
            handlers: handlers.clone(),
        })
        .collect()
}

pub(crate) fn exceptions_for(category_mime: &str, state: &handlr::State) -> Vec<Row> {
    debug_assert!(category_mime.ends_with("/*"));
    let prefix = category_mime.strip_suffix('*').unwrap_or(category_mime);

    // BTreeMap keeps entries sorted by MIME and lets seeds fill in gaps.
    // Value is None for seed placeholders (no handler), Some for configured entries.
    let mut map: std::collections::BTreeMap<&str, Option<&Vec<String>>> =
        std::collections::BTreeMap::new();

    for (mime, handlers) in &state.defaults {
        if !mime.ends_with('*') && mime.starts_with(prefix) {
            map.insert(mime.as_str(), Some(handlers));
        }
    }
    for &seed in SEED_EXCEPTIONS {
        if seed.starts_with(prefix) {
            map.entry(seed).or_insert(None);
        }
    }

    map.into_iter()
        .map(|(mime, handlers)| Row::Exception {
            mime: mime.to_string(),
            handlers: handlers.cloned().unwrap_or_default(),
        })
        .collect()
}

pub(crate) fn handlers_for(mime: &str, handlers: &[String]) -> Vec<Row> {
    let total = handlers.len();
    handlers
        .iter()
        .enumerate()
        .map(|(index, desktop)| Row::Handler {
            mime: mime.to_string(),
            desktop: desktop.clone(),
            index,
            total,
        })
        .collect()
}

pub(crate) struct AppState {
    state: handlr::State,
    undo: VecDeque<UndoEntry>,
    redo: VecDeque<UndoEntry>,
}

impl AppState {
    pub(crate) fn load() -> Result<Self> {
        Ok(Self {
            state: handlr::list_all()?,
            undo: VecDeque::new(),
            redo: VecDeque::new(),
        })
    }

    pub(crate) fn state(&self) -> &handlr::State {
        &self.state
    }

    pub(crate) fn set_state(&mut self, s: handlr::State) {
        self.state = s;
    }

    // Add a placeholder (mime, []) entry so the tree shows the MIME under its category
    // even though no handler has been configured yet. The placeholder is purely in-memory;
    // it disappears after the next rebuild from handlr (any action or Reload). This lets
    // the user see the entry in context and click "Set handler" to make it permanent.
    pub(crate) fn add_pending_exception(&mut self, mime: String) {
        let already = self.state.defaults.iter().any(|(m, _)| m == &mime);
        if !already {
            self.state.defaults.push((mime, Vec::new()));
        }
    }

    pub(crate) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub(crate) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub(crate) fn undo_label(&self) -> Option<&str> {
        self.undo.back().map(|e| e.label.as_str())
    }

    pub(crate) fn redo_label(&self) -> Option<&str> {
        self.redo.back().map(|e| e.label.as_str())
    }

    pub(crate) fn apply(&mut self, entry: UndoEntry) -> Result<()> {
        if entry.forward.is_empty() {
            return Ok(());
        }
        let fwd = run_all(&entry.forward);
        let resync = handlr::list_all();
        match (fwd, resync) {
            (Ok(()), Ok(s)) => {
                self.state = s;
                self.redo.clear();
                undo::push_bounded(&mut self.undo, entry);
                Ok(())
            }
            (Ok(()), Err(e)) => {
                // Forward ran; on-disk state is the new state but we couldn't refresh
                // our snapshot. Stacks stay consistent so undo still works.
                self.redo.clear();
                undo::push_bounded(&mut self.undo, entry);
                Err(e)
            }
            (Err(e), Ok(s)) => {
                self.state = s;
                Err(e)
            }
            (Err(e), Err(_)) => Err(e),
        }
    }

    pub(crate) fn undo(&mut self) -> Result<Option<String>> {
        let Some(entry) = self.undo.pop_back() else {
            return Ok(None);
        };
        let inv = run_all(&entry.inverse);
        let resync = handlr::list_all();
        match (inv, resync) {
            (Ok(()), Ok(s)) => {
                self.state = s;
                let label = entry.label.clone();
                self.redo.push_back(entry);
                Ok(Some(label))
            }
            (Ok(()), Err(e)) => {
                // Inverse applied on disk; push to redo so the stacks remain consistent.
                self.redo.push_back(entry);
                Err(e)
            }
            (Err(e), Ok(s)) => {
                // Inverse failed — don't push to redo; restore the undo slot.
                self.state = s;
                self.undo.push_back(entry);
                Err(e)
            }
            (Err(e), Err(_)) => {
                self.undo.push_back(entry);
                Err(e)
            }
        }
    }

    pub(crate) fn redo(&mut self) -> Result<Option<String>> {
        let Some(entry) = self.redo.pop_back() else {
            return Ok(None);
        };
        let fwd = run_all(&entry.forward);
        let resync = handlr::list_all();
        match (fwd, resync) {
            (Ok(()), Ok(s)) => {
                self.state = s;
                let label = entry.label.clone();
                undo::push_bounded(&mut self.undo, entry);
                Ok(Some(label))
            }
            (Ok(()), Err(e)) => {
                undo::push_bounded(&mut self.undo, entry);
                Err(e)
            }
            (Err(e), Ok(s)) => {
                self.state = s;
                self.redo.push_back(entry);
                Err(e)
            }
            (Err(e), Err(_)) => {
                self.redo.push_back(entry);
                Err(e)
            }
        }
    }
}

fn run_all(cmds: &[handlr::HandlrCmd]) -> Result<()> {
    for cmd in cmds {
        handlr::run(cmd)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlr::{App, State};

    fn state_with(defaults: Vec<(&str, Vec<&str>)>) -> State {
        State {
            defaults: defaults
                .into_iter()
                .map(|(m, hs)| (m.to_string(), hs.into_iter().map(String::from).collect()))
                .collect(),
            system_apps: Vec::<App>::new(),
        }
    }

    fn cat_mime(row: &Row) -> &str {
        match row {
            Row::Category { mime, .. } => mime,
            _ => panic!("expected category"),
        }
    }

    #[test]
    fn build_categories_unions_seed() {
        let s = state_with(vec![("video/*", vec!["mpv.desktop"])]);
        let rows = build_categories(&s);
        assert_eq!(rows.len(), SEED_CATEGORIES.len());

        let video = rows
            .iter()
            .find(|r| matches!(r, Row::Category { mime, .. } if mime == "video/*"))
            .unwrap();
        if let Row::Category { handlers, .. } = video {
            assert_eq!(handlers, &vec!["mpv.desktop".to_string()]);
        }

        let audio = rows
            .iter()
            .find(|r| matches!(r, Row::Category { mime, .. } if mime == "audio/*"))
            .unwrap();
        if let Row::Category { handlers, .. } = audio {
            assert!(handlers.is_empty());
        }
    }

    #[test]
    fn build_categories_sorted() {
        let s = state_with(vec![("video/*", vec![]), ("audio/*", vec![])]);
        let rows = build_categories(&s);
        let mimes: Vec<&str> = rows.iter().map(cat_mime).collect();
        let mut expected = mimes.clone();
        expected.sort();
        assert_eq!(mimes, expected);
    }

    #[test]
    fn build_categories_dedupes_seed_overlap() {
        // A configured seed should not duplicate the same mime in the output.
        let s = state_with(vec![("audio/*", vec!["mpv.desktop"])]);
        let rows = build_categories(&s);
        let audios: Vec<_> = rows
            .iter()
            .filter(|r| matches!(r, Row::Category { mime, .. } if mime == "audio/*"))
            .collect();
        assert_eq!(audios.len(), 1);
    }

    #[test]
    fn exceptions_for_filters_correctly() {
        let s = state_with(vec![
            ("video/*", vec!["mpv.desktop"]),
            ("video/mp4", vec!["vlc.desktop"]),
            ("video/x-mkv", vec!["mpv.desktop"]),
            ("text/plain", vec!["helix.desktop"]),
        ]);
        let rows = exceptions_for("video/*", &s);
        assert_eq!(rows.len(), 2);
        let mimes: Vec<&str> = rows
            .iter()
            .map(|r| match r {
                Row::Exception { mime, .. } => mime.as_str(),
                _ => panic!("expected exception"),
            })
            .collect();
        assert_eq!(mimes, vec!["video/mp4", "video/x-mkv"]);
    }

    #[test]
    fn handlers_for_indices() {
        let handlers = vec!["a.desktop".into(), "b.desktop".into(), "c.desktop".into()];
        let rows = handlers_for("video/*", &handlers);
        assert_eq!(rows.len(), 3);
        for (i, row) in rows.iter().enumerate() {
            match row {
                Row::Handler {
                    mime,
                    desktop,
                    index,
                    total,
                } => {
                    assert_eq!(mime, "video/*");
                    assert_eq!(*index, i);
                    assert_eq!(*total, 3);
                    assert_eq!(desktop, &handlers[i]);
                }
                _ => panic!("expected handler"),
            }
        }
    }
}
