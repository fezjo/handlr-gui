//! Undo/redo entry types and builders mapping user actions to handlr CLI command pairs.
//! See spec §4 for the forward/inverse contract.

use crate::handlr::HandlrCmd;
use std::collections::VecDeque;

pub(crate) const UNDO_BOUND: usize = 100;

#[derive(Debug, Clone)]
pub(crate) struct UndoEntry {
    pub(crate) label: String,
    pub(crate) forward: Vec<HandlrCmd>,
    pub(crate) inverse: Vec<HandlrCmd>,
}

pub(crate) fn set_category_default(mime: &str, new: &str, prior: &[String]) -> UndoEntry {
    let forward = vec![HandlrCmd::Set {
        mime: mime.into(),
        desktop: new.into(),
    }];
    // Inverse depends on prior state: empty → unset; single → set; multiple → set + N-1 adds.
    let inverse = inverse_for_set(mime, prior);
    let label = if prior.is_empty() {
        format!("set {} to {}", mime, new)
    } else {
        format!("change {} default to {}", mime, new)
    };
    UndoEntry { label, forward, inverse }
}

pub(crate) fn clear_category_default(mime: &str, prior: &[String]) -> UndoEntry {
    // If alternatives exist, remove only the default so the next one is promoted.
    // If there are no alternatives, unset the entry entirely.
    let forward = if prior.len() > 1 {
        vec![HandlrCmd::Remove { mime: mime.into(), desktop: prior[0].clone() }]
    } else {
        vec![HandlrCmd::Unset { mime: mime.into() }]
    };
    UndoEntry {
        label: format!("clear {} default", mime),
        forward,
        inverse: inverse_for_set(mime, prior),
    }
}

pub(crate) fn add_exception(mime: &str, desktop: &str, prior: &[String]) -> UndoEntry {
    UndoEntry {
        label: format!("add exception {} -> {}", mime, desktop),
        forward: vec![HandlrCmd::Set {
            mime: mime.into(),
            desktop: desktop.into(),
        }],
        inverse: inverse_for_set(mime, prior),
    }
}

pub(crate) fn remove_exception(mime: &str, prior: &[String]) -> UndoEntry {
    UndoEntry {
        label: format!("remove exception {}", mime),
        forward: vec![HandlrCmd::Unset { mime: mime.into() }],
        inverse: inverse_for_set(mime, prior),
    }
}

pub(crate) fn add_handler(mime: &str, desktop: &str) -> UndoEntry {
    UndoEntry {
        label: format!("add handler {} += {}", mime, desktop),
        forward: vec![HandlrCmd::Add {
            mime: mime.into(),
            desktop: desktop.into(),
        }],
        inverse: vec![HandlrCmd::Remove {
            mime: mime.into(),
            desktop: desktop.into(),
        }],
    }
}

pub(crate) fn remove_handler(mime: &str, desktop: &str) -> UndoEntry {
    UndoEntry {
        label: format!("remove handler {} -= {}", mime, desktop),
        forward: vec![HandlrCmd::Remove {
            mime: mime.into(),
            desktop: desktop.into(),
        }],
        inverse: vec![HandlrCmd::Add {
            mime: mime.into(),
            desktop: desktop.into(),
        }],
    }
}

/// Swap the handler at `index` with the one above it (`index - 1`). Returns `None` if
/// `index` is 0 or out of range. Both forward and inverse are full reorders so they're
/// always consistent regardless of what other operations run in between.
pub(crate) fn move_handler_up(mime: &str, handlers: &[String], index: usize) -> Option<UndoEntry> {
    if index == 0 || index >= handlers.len() {
        return None;
    }
    let mut new_order = handlers.to_vec();
    new_order.swap(index - 1, index);
    Some(UndoEntry {
        label: format!("promote {} in {}", handlers[index], mime),
        forward: order_cmds(mime, &new_order),
        inverse: order_cmds(mime, handlers),
    })
}

fn order_cmds(mime: &str, handlers: &[String]) -> Vec<HandlrCmd> {
    if handlers.is_empty() {
        return vec![HandlrCmd::Unset { mime: mime.into() }];
    }
    let mut cmds = Vec::with_capacity(handlers.len());
    cmds.push(HandlrCmd::Set { mime: mime.into(), desktop: handlers[0].clone() });
    for d in &handlers[1..] {
        cmds.push(HandlrCmd::Add { mime: mime.into(), desktop: d.clone() });
    }
    cmds
}

pub(crate) fn push_bounded(stack: &mut VecDeque<UndoEntry>, entry: UndoEntry) {
    if stack.len() >= UNDO_BOUND {
        stack.pop_front();
    }
    stack.push_back(entry);
}

// `set` overwrites the full handler list, so the inverse must restore every prior entry:
// `set` the first (clears + writes one), then `add` the rest in order.
fn inverse_for_set(mime: &str, prior: &[String]) -> Vec<HandlrCmd> {
    if prior.is_empty() {
        return vec![HandlrCmd::Unset { mime: mime.into() }];
    }
    let mut out = Vec::with_capacity(prior.len());
    out.push(HandlrCmd::Set {
        mime: mime.into(),
        desktop: prior[0].clone(),
    });
    for d in &prior[1..] {
        out.push(HandlrCmd::Add {
            mime: mime.into(),
            desktop: d.clone(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_category_default_empty_prior() {
        let e = set_category_default("video/*", "mpv.desktop", &[]);
        assert_eq!(
            e.forward,
            vec![HandlrCmd::Set {
                mime: "video/*".into(),
                desktop: "mpv.desktop".into()
            }]
        );
        assert_eq!(
            e.inverse,
            vec![HandlrCmd::Unset {
                mime: "video/*".into()
            }]
        );
    }

    #[test]
    fn set_category_default_with_prior() {
        let prior = vec!["vlc.desktop".to_string()];
        let e = set_category_default("video/*", "mpv.desktop", &prior);
        assert_eq!(
            e.forward,
            vec![HandlrCmd::Set {
                mime: "video/*".into(),
                desktop: "mpv.desktop".into()
            }]
        );
        assert_eq!(
            e.inverse,
            vec![HandlrCmd::Set {
                mime: "video/*".into(),
                desktop: "vlc.desktop".into()
            }]
        );
    }

    #[test]
    fn clear_category_default_no_prior() {
        let e = clear_category_default("video/*", &[]);
        assert_eq!(
            e.forward,
            vec![HandlrCmd::Unset {
                mime: "video/*".into()
            }]
        );
        assert_eq!(
            e.inverse,
            vec![HandlrCmd::Unset {
                mime: "video/*".into()
            }]
        );
    }

    #[test]
    fn clear_with_multiple_handlers_promotes_alternative() {
        let prior = vec!["mpv.desktop".to_string(), "vlc.desktop".to_string()];
        let e = clear_category_default("video/*", &prior);
        // Forward removes only the default so vlc is promoted, not a full unset.
        assert_eq!(
            e.forward,
            vec![HandlrCmd::Remove {
                mime: "video/*".into(),
                desktop: "mpv.desktop".into()
            }]
        );
        assert_eq!(
            e.inverse,
            vec![
                HandlrCmd::Set {
                    mime: "video/*".into(),
                    desktop: "mpv.desktop".into()
                },
                HandlrCmd::Add {
                    mime: "video/*".into(),
                    desktop: "vlc.desktop".into()
                },
            ]
        );
    }

    #[test]
    fn add_exception_pair() {
        let e = add_exception("video/mp4", "vlc.desktop", &[]);
        assert_eq!(
            e.forward,
            vec![HandlrCmd::Set {
                mime: "video/mp4".into(),
                desktop: "vlc.desktop".into()
            }]
        );
        assert_eq!(
            e.inverse,
            vec![HandlrCmd::Unset {
                mime: "video/mp4".into()
            }]
        );
    }

    #[test]
    fn add_exception_with_prior_restores_full_list() {
        let prior = vec!["vlc.desktop".to_string(), "mpv.desktop".to_string()];
        let e = add_exception("video/mp4", "totem.desktop", &prior);
        assert_eq!(
            e.forward,
            vec![HandlrCmd::Set {
                mime: "video/mp4".into(),
                desktop: "totem.desktop".into()
            }]
        );
        assert_eq!(
            e.inverse,
            vec![
                HandlrCmd::Set {
                    mime: "video/mp4".into(),
                    desktop: "vlc.desktop".into()
                },
                HandlrCmd::Add {
                    mime: "video/mp4".into(),
                    desktop: "mpv.desktop".into()
                },
            ]
        );
    }

    #[test]
    fn remove_exception_pair() {
        let prior = vec!["vlc.desktop".to_string()];
        let e = remove_exception("video/mp4", &prior);
        assert_eq!(
            e.forward,
            vec![HandlrCmd::Unset {
                mime: "video/mp4".into()
            }]
        );
        assert_eq!(
            e.inverse,
            vec![HandlrCmd::Set {
                mime: "video/mp4".into(),
                desktop: "vlc.desktop".into()
            }]
        );
    }

    #[test]
    fn remove_exception_with_multiple_handlers_chains_inverse() {
        let prior = vec!["vlc.desktop".to_string(), "mpv.desktop".to_string()];
        let e = remove_exception("video/mp4", &prior);
        assert_eq!(
            e.forward,
            vec![HandlrCmd::Unset {
                mime: "video/mp4".into()
            }]
        );
        assert_eq!(
            e.inverse,
            vec![
                HandlrCmd::Set {
                    mime: "video/mp4".into(),
                    desktop: "vlc.desktop".into()
                },
                HandlrCmd::Add {
                    mime: "video/mp4".into(),
                    desktop: "mpv.desktop".into()
                },
            ]
        );
    }

    #[test]
    fn add_handler_pair() {
        let e = add_handler("video/*", "vlc.desktop");
        assert_eq!(
            e.forward,
            vec![HandlrCmd::Add {
                mime: "video/*".into(),
                desktop: "vlc.desktop".into()
            }]
        );
        assert_eq!(
            e.inverse,
            vec![HandlrCmd::Remove {
                mime: "video/*".into(),
                desktop: "vlc.desktop".into()
            }]
        );
    }

    #[test]
    fn remove_handler_pair() {
        let e = remove_handler("video/*", "vlc.desktop");
        assert_eq!(
            e.forward,
            vec![HandlrCmd::Remove {
                mime: "video/*".into(),
                desktop: "vlc.desktop".into()
            }]
        );
        assert_eq!(
            e.inverse,
            vec![HandlrCmd::Add {
                mime: "video/*".into(),
                desktop: "vlc.desktop".into()
            }]
        );
    }

    #[test]
    fn push_bounded_drops_oldest() {
        let mut stack = VecDeque::new();
        for i in 0..=UNDO_BOUND {
            push_bounded(
                &mut stack,
                UndoEntry {
                    label: format!("e{}", i),
                    forward: vec![],
                    inverse: vec![],
                },
            );
        }
        assert_eq!(stack.len(), UNDO_BOUND);
        assert_eq!(stack.front().unwrap().label, "e1");
        assert_eq!(stack.back().unwrap().label, format!("e{}", UNDO_BOUND));
    }
}
