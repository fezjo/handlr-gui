//! Tree-row data model and owning AppState. See spec §3 for the tree layout and §11 for
//! the always-on seed categories.

use crate::handlr;
use crate::undo::{self, UndoEntry};
use anyhow::Result;
use std::collections::{BTreeMap, HashSet, VecDeque};

/// Static compile-time description of one tree node. Children are always a
/// `'static` slice so the structure can be embedded in ROM without allocation.
#[derive(Debug)]
pub(crate) struct NodeDef {
    pub(crate) kind: NodeKind,
    pub(crate) children: &'static [NodeDef],
}

#[derive(Debug)]
pub(crate) enum NodeKind {
    /// Pure container — collapsible section with a label and icon, no handler.
    SemanticGroup { name: &'static str, icon: &'static str },
    /// Sets every listed MIME to the same handler in one action.
    BatchGroup { name: &'static str, icon: &'static str, mimes: &'static [&'static str] },
    /// A wildcard MIME category (e.g. "audio/*") — expands to exceptions + alts.
    WildcardCategory { mime: &'static str },
    /// An exact MIME always shown as a seed exception row.
    SeedException { mime: &'static str },
}

pub(crate) static ROOT_TREE: &[NodeDef] = &[
    // ── Semantic groups (collapsed by default) ─────────────────────────────
    NodeDef {
        kind: NodeKind::SemanticGroup {
            name: "System defaults",
            icon: "preferences-system-symbolic",
        },
        children: &[
            NodeDef {
                kind: NodeKind::BatchGroup {
                    name: "Directory opener",
                    icon: "folder-symbolic",
                    mimes: &["inode/directory"],
                },
                children: &[],
            },
            NodeDef {
                kind: NodeKind::BatchGroup {
                    name: "Web browser",
                    icon: "web-browser-symbolic",
                    mimes: &[
                        "x-scheme-handler/http",
                        "x-scheme-handler/https",
                        "x-scheme-handler/chrome",
                        "text/html",
                        "application/xhtml+xml",
                    ],
                },
                children: &[],
            },
            NodeDef {
                kind: NodeKind::BatchGroup {
                    name: "Email client",
                    icon: "mail-unread-symbolic",
                    mimes: &["x-scheme-handler/mailto"],
                },
                children: &[],
            },
            NodeDef {
                kind: NodeKind::BatchGroup {
                    name: "Terminal",
                    icon: "utilities-terminal-symbolic",
                    mimes: &["x-scheme-handler/terminal"],
                },
                children: &[],
            },
        ],
    },
    NodeDef {
        kind: NodeKind::SemanticGroup {
            name: "Documents & Files",
            icon: "folder-documents-symbolic",
        },
        children: &[
            NodeDef {
                kind: NodeKind::BatchGroup {
                    name: "PDF viewer",
                    icon: "application-pdf-symbolic",
                    mimes: &["application/pdf"],
                },
                children: &[],
            },
            NodeDef {
                kind: NodeKind::BatchGroup {
                    name: "E-book reader",
                    icon: "accessories-ebook-reader-symbolic",
                    mimes: &[
                        "application/epub+zip",
                        "application/x-mobipocket-ebook",
                        "application/x-fictionbook+xml",
                    ],
                },
                children: &[],
            },
            NodeDef {
                kind: NodeKind::SemanticGroup {
                    name: "Office documents",
                    icon: "x-office-document-symbolic",
                },
                children: &[
                    NodeDef {
                        kind: NodeKind::BatchGroup {
                            name: "Word processor",
                            icon: "x-office-document-symbolic",
                            mimes: &[
                                "application/vnd.oasis.opendocument.text",
                                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                                "application/msword",
                                "text/rtf",
                            ],
                        },
                        children: &[],
                    },
                    NodeDef {
                        kind: NodeKind::BatchGroup {
                            name: "Spreadsheet",
                            icon: "x-office-spreadsheet-symbolic",
                            mimes: &[
                                "application/vnd.oasis.opendocument.spreadsheet",
                                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                                "application/vnd.ms-excel",
                                "text/csv",
                            ],
                        },
                        children: &[],
                    },
                    NodeDef {
                        kind: NodeKind::BatchGroup {
                            name: "Presentation",
                            icon: "x-office-presentation-symbolic",
                            mimes: &[
                                "application/vnd.oasis.opendocument.presentation",
                                "application/vnd.openxmlformats-officedocument.presentationml.presentation",
                                "application/vnd.ms-powerpoint",
                            ],
                        },
                        children: &[],
                    },
                ],
            },
            NodeDef {
                kind: NodeKind::BatchGroup {
                    name: "Archive manager",
                    icon: "package-x-generic-symbolic",
                    mimes: &[
                        "application/zip",
                        "application/x-tar",
                        "application/x-bzip2",
                        "application/gzip",
                        "application/x-7z-compressed",
                        "application/x-rar",
                        "application/x-xz",
                        "application/zstd",
                    ],
                },
                children: &[],
            },
            NodeDef {
                kind: NodeKind::SemanticGroup {
                    name: "System packages",
                    icon: "system-software-install-symbolic",
                },
                children: &[
                    NodeDef { kind: NodeKind::SeedException { mime: "application/x-deb" }, children: &[] },
                    NodeDef { kind: NodeKind::SeedException { mime: "application/x-rpm" }, children: &[] },
                    NodeDef { kind: NodeKind::SeedException { mime: "application/vnd.appimage" }, children: &[] },
                    NodeDef { kind: NodeKind::SeedException { mime: "application/x-ms-dos-executable" }, children: &[] },
                    NodeDef { kind: NodeKind::SeedException { mime: "application/x-cd-image" }, children: &[] },
                ],
            },
        ],
    },
    NodeDef {
        kind: NodeKind::SemanticGroup {
            name: "Internet & Communication",
            icon: "network-workgroup-symbolic",
        },
        children: &[
            NodeDef {
                kind: NodeKind::BatchGroup {
                    name: "Feed reader",
                    icon: "application-rss+xml-symbolic",
                    mimes: &[
                        "x-scheme-handler/feed",
                        "x-scheme-handler/feeds",
                        "application/rss+xml",
                        "application/atom+xml",
                    ],
                },
                children: &[],
            },
            NodeDef {
                kind: NodeKind::BatchGroup {
                    name: "Calendar",
                    icon: "x-office-calendar-symbolic",
                    mimes: &["text/calendar", "x-scheme-handler/webcal"],
                },
                children: &[],
            },
            NodeDef {
                kind: NodeKind::SemanticGroup {
                    name: "Messaging",
                    icon: "user-available-symbolic",
                },
                children: &[
                    NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/discord" }, children: &[] },
                    NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/slack" }, children: &[] },
                    NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/telegram-desktop" }, children: &[] },
                    NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/tg" }, children: &[] },
                    NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/signal" }, children: &[] },
                    NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/element" }, children: &[] },
                    NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/mattermost" }, children: &[] },
                    NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/zoom" }, children: &[] },
                    NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/teams" }, children: &[] },
                    NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/skype" }, children: &[] },
                    NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/viber" }, children: &[] },
                    NodeDef {
                        kind: NodeKind::SemanticGroup {
                            name: "Protocol URIs",
                            icon: "network-server-symbolic",
                        },
                        children: &[
                            NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/xmpp" }, children: &[] },
                            NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/matrix" }, children: &[] },
                            NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/irc" }, children: &[] },
                            NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/ircs" }, children: &[] },
                            NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/sip" }, children: &[] },
                            NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/tel" }, children: &[] },
                        ],
                    },
                ],
            },
        ],
    },
    NodeDef {
        kind: NodeKind::SemanticGroup {
            name: "Media",
            icon: "applications-multimedia-symbolic",
        },
        children: &[
            NodeDef {
                kind: NodeKind::BatchGroup {
                    name: "Torrent client",
                    icon: "network-transmit-receive-symbolic",
                    mimes: &["x-scheme-handler/magnet", "application/x-bittorrent"],
                },
                children: &[],
            },
            NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/spotify" }, children: &[] },
        ],
    },
    NodeDef {
        kind: NodeKind::SemanticGroup {
            name: "Gaming",
            icon: "applications-games-symbolic",
        },
        children: &[
            NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/steam" }, children: &[] },
            NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/lutris" }, children: &[] },
            NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/heroic" }, children: &[] },
            NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/itch" }, children: &[] },
            NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/minecraftlauncher" }, children: &[] },
            NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/prismlauncher" }, children: &[] },
        ],
    },
    NodeDef {
        kind: NodeKind::SemanticGroup {
            name: "Development",
            icon: "applications-development-symbolic",
        },
        children: &[
            NodeDef {
                kind: NodeKind::SemanticGroup {
                    name: "Code editors",
                    icon: "text-editor-symbolic",
                },
                children: &[
                    NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/vscode" }, children: &[] },
                    NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/vscode-insiders" }, children: &[] },
                    NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/vscodium" }, children: &[] },
                    NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/cursor" }, children: &[] },
                ],
            },
            NodeDef {
                kind: NodeKind::SemanticGroup {
                    name: "IDEs & Dev tools",
                    icon: "applications-development-symbolic",
                },
                children: &[
                    NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/jetbrains-toolbox" }, children: &[] },
                    NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/idea" }, children: &[] },
                    NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/rider" }, children: &[] },
                    NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/webstorm" }, children: &[] },
                    NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/pycharm" }, children: &[] },
                    NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/clion" }, children: &[] },
                    NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/github-desktop" }, children: &[] },
                    NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/gitkraken" }, children: &[] },
                ],
            },
        ],
    },
    NodeDef {
        kind: NodeKind::SemanticGroup {
            name: "Security & Passwords",
            icon: "security-high-symbolic",
        },
        children: &[
            NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/bitwarden" }, children: &[] },
            NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/1password" }, children: &[] },
            NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/lastpass" }, children: &[] },
            NodeDef { kind: NodeKind::SeedException { mime: "x-scheme-handler/dashlane" }, children: &[] },
        ],
    },
    // ── Standard MIME type wildcard categories ──────────────────────────────
    NodeDef { kind: NodeKind::WildcardCategory { mime: "audio/*" }, children: &[] },
    NodeDef { kind: NodeKind::WildcardCategory { mime: "video/*" }, children: &[] },
    NodeDef { kind: NodeKind::WildcardCategory { mime: "image/*" }, children: &[] },
    NodeDef { kind: NodeKind::WildcardCategory { mime: "text/*" }, children: &[] },
    NodeDef { kind: NodeKind::WildcardCategory { mime: "application/*" }, children: &[] },
    NodeDef { kind: NodeKind::WildcardCategory { mime: "x-scheme-handler/*" }, children: &[] },
];

#[derive(Debug, Clone)]
pub(crate) enum Row {
    /// Pure container — label + icon, no handler. Children come from static_children.
    SemanticGroup {
        name: String,
        icon_name: String,
        static_children: &'static [NodeDef],
    },
    /// Sets every listed MIME to the same handler in one action.
    BatchGroup {
        name: String,
        icon_name: String,
        mimes: Vec<String>,
        prior_handlers: Vec<Vec<String>>,
        display_handler: Option<String>,
    },
    /// One child row per MIME inside a BatchGroup.
    BatchMime {
        mime: String,
        mime_handlers: Vec<String>,
        group_handler: Option<String>,
    },
    Category { mime: String, handlers: Vec<String> },
    Exception { mime: String, handlers: Vec<String> },
    Handler {
        mime: String,
        desktop: String,
        index: usize,
        #[allow(dead_code)]
        total: usize,
    },
    AddException { category_mime: String },
}

/// Convert a static node + live handlr state into a runtime Row.
/// The node must be `'static` so SemanticGroup can carry `node.children` by reference.
pub(crate) fn node_to_row(node: &'static NodeDef, state: &handlr::State) -> Row {
    match &node.kind {
        NodeKind::SemanticGroup { name, icon } => Row::SemanticGroup {
            name: name.to_string(),
            icon_name: icon.to_string(),
            static_children: node.children,
        },
        NodeKind::BatchGroup { name, icon, mimes } => {
            let prior_handlers: Vec<Vec<String>> = mimes
                .iter()
                .map(|mime| {
                    state
                        .defaults
                        .iter()
                        .find(|(m, _)| m == mime)
                        .map(|(_, h)| h.clone())
                        .unwrap_or_default()
                })
                .collect();
            let display_handler = compute_display_handler(&prior_handlers);
            Row::BatchGroup {
                name: name.to_string(),
                icon_name: icon.to_string(),
                mimes: mimes.iter().map(|m| m.to_string()).collect(),
                prior_handlers,
                display_handler,
            }
        }
        NodeKind::WildcardCategory { mime } => {
            let handlers = state
                .defaults
                .iter()
                .find(|(m, _)| m == mime)
                .map(|(_, h)| h.clone())
                .unwrap_or_default();
            Row::Category { mime: mime.to_string(), handlers }
        }
        NodeKind::SeedException { mime } => {
            let handlers = state
                .defaults
                .iter()
                .find(|(m, _)| m == mime)
                .map(|(_, h)| h.clone())
                .unwrap_or_default();
            Row::Exception { mime: mime.to_string(), handlers }
        }
    }
}

/// Returns `Some(desktop)` iff every MIME in the group has the same first handler.
/// Returns `None` when all MIMEs are unset OR when handlers disagree across MIMEs.
fn compute_display_handler(prior_handlers: &[Vec<String>]) -> Option<String> {
    let firsts: Vec<&str> = prior_handlers
        .iter()
        .filter_map(|h| h.first().map(String::as_str))
        .collect();
    if firsts.is_empty() {
        return None;
    }
    let first = firsts[0];
    if firsts.iter().all(|&h| h == first) { Some(first.to_string()) } else { None }
}

/// Build runtime Row values for every node in ROOT_TREE.
pub(crate) fn build_root_rows(state: &handlr::State) -> Vec<Row> {
    ROOT_TREE.iter().map(|node| node_to_row(node, state)).collect()
}

/// Find wildcard MIMEs in handlr state that are NOT already represented as
/// WildcardCategory nodes anywhere in ROOT_TREE. Returned as Category rows,
/// sorted alphabetically.
pub(crate) fn build_extra_wildcards(state: &handlr::State) -> Vec<Row> {
    let known = wildcards_in_tree(ROOT_TREE);
    let mut extra: Vec<_> = state
        .defaults
        .iter()
        .filter(|(mime, _)| mime.ends_with("/*") && !known.contains(mime.as_str()))
        .map(|(mime, handlers)| Row::Category { mime: mime.clone(), handlers: handlers.clone() })
        .collect();
    extra.sort_by(|a, b| match (a, b) {
        (Row::Category { mime: ma, .. }, Row::Category { mime: mb, .. }) => ma.cmp(mb),
        _ => std::cmp::Ordering::Equal,
    });
    extra
}

fn wildcards_in_tree(nodes: &[NodeDef]) -> HashSet<&'static str> {
    let mut set = HashSet::new();
    for node in nodes {
        if let NodeKind::WildcardCategory { mime } = &node.kind {
            set.insert(*mime);
        }
        set.extend(wildcards_in_tree(node.children));
    }
    set
}

pub(crate) fn exceptions_for(category_mime: &str, state: &handlr::State) -> Vec<Row> {
    debug_assert!(category_mime.ends_with("/*"));
    let prefix = category_mime.strip_suffix('*').unwrap_or(category_mime);
    let mut map: BTreeMap<&str, &Vec<String>> = BTreeMap::new();
    for (mime, handlers) in &state.defaults {
        if !mime.ends_with('*') && mime.starts_with(prefix) {
            map.insert(mime.as_str(), handlers);
        }
    }
    map.into_iter()
        .map(|(mime, handlers)| Row::Exception {
            mime: mime.to_string(),
            handlers: handlers.clone(),
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

    #[test]
    fn node_to_row_wildcard_category_reads_handlers() {
        let s = state_with(vec![("audio/*", vec!["mpv.desktop"])]);
        // Find the audio/* WildcardCategory node in ROOT_TREE
        let node = ROOT_TREE
            .iter()
            .find(|n| matches!(&n.kind, NodeKind::WildcardCategory { mime } if *mime == "audio/*"))
            .unwrap();
        let row = node_to_row(node, &s);
        match row {
            Row::Category { mime, handlers } => {
                assert_eq!(mime, "audio/*");
                assert_eq!(handlers, vec!["mpv.desktop".to_string()]);
            }
            _ => panic!("expected Category"),
        }
    }

    #[test]
    fn node_to_row_seed_exception_reads_handlers() {
        let s = state_with(vec![("x-scheme-handler/discord", vec!["discord.desktop"])]);
        // node_to_row requires &'static NodeDef; use ROOT_TREE to find a real static one
        let static_node = ROOT_TREE
            .iter()
            .flat_map(|n| n.children)
            .flat_map(|n| n.children)
            .flat_map(|n| n.children)
            .chain(ROOT_TREE.iter().flat_map(|n| n.children).flat_map(|n| n.children))
            .chain(ROOT_TREE.iter().flat_map(|n| n.children))
            .find(|n| matches!(&n.kind, NodeKind::SeedException { mime } if *mime == "x-scheme-handler/discord"))
            .unwrap();
        let row = node_to_row(static_node, &s);
        match row {
            Row::Exception { mime, handlers } => {
                assert_eq!(mime, "x-scheme-handler/discord");
                assert_eq!(handlers, vec!["discord.desktop".to_string()]);
            }
            _ => panic!("expected Exception"),
        }
    }

    #[test]
    fn compute_display_handler_agrees() {
        assert_eq!(
            compute_display_handler(&[vec!["a.desktop".into()], vec!["a.desktop".into()]]),
            Some("a.desktop".to_string())
        );
    }

    #[test]
    fn compute_display_handler_mixed_returns_none() {
        assert_eq!(
            compute_display_handler(&[vec!["a.desktop".into()], vec!["b.desktop".into()]]),
            None
        );
    }

    #[test]
    fn compute_display_handler_empty_returns_none() {
        assert_eq!(compute_display_handler(&[vec![], vec![]]), None);
    }

    #[test]
    fn build_root_rows_returns_one_per_root_node() {
        let s = state_with(vec![]);
        let rows = build_root_rows(&s);
        assert_eq!(rows.len(), ROOT_TREE.len());
    }

    #[test]
    fn build_extra_wildcards_finds_user_wildcards() {
        let s = state_with(vec![
            ("audio/*", vec!["mpv.desktop"]),  // already in ROOT_TREE, excluded
            ("font/*", vec!["fontforge.desktop"]),  // not in ROOT_TREE, included
        ]);
        let extras = build_extra_wildcards(&s);
        assert_eq!(extras.len(), 1);
        match &extras[0] {
            Row::Category { mime, .. } => assert_eq!(mime, "font/*"),
            _ => panic!("expected Category"),
        }
    }

    #[test]
    fn build_extra_wildcards_excludes_all_known_wildcards() {
        // All ROOT_TREE WildcardCategory mimes should be excluded even if configured.
        let s = state_with(vec![
            ("audio/*", vec!["mpv.desktop"]),
            ("video/*", vec!["mpv.desktop"]),
            ("image/*", vec![]),
            ("text/*", vec![]),
            ("application/*", vec![]),
            ("x-scheme-handler/*", vec![]),
        ]);
        let extras = build_extra_wildcards(&s);
        assert!(extras.is_empty(), "got: {:?}", extras);
    }

    #[test]
    fn exceptions_for_excludes_wildcard_and_star_entries() {
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
        // Sorted by BTreeMap key
        assert_eq!(mimes, vec!["video/mp4", "video/x-mkv"]);
    }

    #[test]
    fn handlers_for_indices() {
        let handlers = vec!["a.desktop".into(), "b.desktop".into(), "c.desktop".into()];
        let rows = handlers_for("video/*", &handlers);
        assert_eq!(rows.len(), 3);
        for (i, row) in rows.iter().enumerate() {
            match row {
                Row::Handler { mime, desktop, index, total } => {
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
