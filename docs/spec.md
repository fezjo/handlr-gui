# handlr-gui Spec (current state)

GTK4 desktop app for managing default MIME associations and regex URL handlers via the
`handlr-regex` CLI. Covers v1 (MIME defaults tree) and v2 (settings panel, regex
handlers editor, tester tab).

---

## 1. Scope

Three tabs in a `GtkStackSwitcher`:

- **Defaults** — indented tree of MIME category defaults with per-type exceptions,
  alternative handlers, and live undo/redo.
- **Regex Handlers** — card-based editor for `[[handlers]]` entries in `handlr.toml`.
- **Tester** — real-time resolution chain: paste/drag a path, URL, or MIME type to see
  which handler wins and why.

Every Defaults action is applied live via `handlr` subprocesses. Regex Handler and
Settings changes go through `toml_edit` round-trip writes to `handlr.toml`. An in-memory
undo stack covers Defaults actions only.

**Out of scope**: added-associations editing, per-individual-MIME editing outside the
category/exception model, file watching (inotify), undo for settings/regex changes.

---

## 2. Widget Tree / Screen Layout

```
GtkApplicationWindow
└── GtkBox (vertical)
    ├── GtkHeaderBar
    │   ├── [start] Undo, Redo buttons
    │   ├── [title] GtkStackSwitcher (Defaults | Regex Handlers | Tester)
    │   └── [end]   Reload, Settings (gear), Hamburger menu
    ├── GtkRevealer → GtkInfoBar  (error banner)
    └── GtkStack (main content area)
        ├── "defaults" page  (GtkStack: populated tree | empty-state label)
        │   └── GtkScrolledWindow → GtkListView
        ├── "handlers" page  (GtkScrolledWindow → GtkBox of handler cards)
        └── "tester" page    (GtkBox: entry + drop target + results)
```

### Defaults tab row template

```
GtkTreeExpander > GtkBox (horizontal)
  [main_icon 24px]  [main_label]  [spacer hexpand]
  [handler_icon 20px]  [handler_label dim]  [badge dim]  [actions GtkBox]
```

- **Category / Exception rows**: mime icon + mime text left; inline default handler
  (icon + name) right; action buttons far right.
- **Handler (alternative) rows**: left side empty; app icon + app name in the right
  handler slots; `Alt.` badge; action buttons far right.
- **AddException rows**: left side empty; single flat `GtkButton` "＋  Add exception"
  in the actions slot.

### Button layout (left → right within actions)

| Row type | Buttons |
|---|---|
| Category — has handlers | edit (change default), trash (clear default), plus (add handler) |
| Category — empty | plus (add handler) |
| Exception — has handlers | edit (change default), trash (remove exception), plus (add handler) |
| Exception — pending (no handler) | trash (remove exception), plus (set handler) |
| Handler alternative (index ≥ 1) | trash (remove handler), up (promote handler) |

---

## 3. Data Model

```rust
enum Row {
    Category    { mime: String, handlers: Vec<String> },
    Exception   { mime: String, handlers: Vec<String> },
    Handler     { mime: String, desktop: String, index: usize, total: usize },
    AddException { category_mime: String },
}

struct App     { desktop: String, name: String }
struct State   { defaults: Vec<(String, Vec<String>)>, system_apps: Vec<App> }

struct UndoEntry { label: String, forward: Vec<HandlrCmd>, inverse: Vec<HandlrCmd> }

enum HandlrCmd {
    Set { mime: String, desktop: String },
    Add { mime: String, desktop: String },
    Remove { mime: String, desktop: String },
    Unset { mime: String },
}

struct Config {
    enable_selector:  bool,
    selector:         String,
    term_exec_args:   String,
    expand_wildcards: bool,
    handlers:         Vec<RegexHandler>,
}

struct RegexHandler { regexes: Vec<String>, exec: String, terminal: bool }
```

### Tree child ordering (per Category)

1. `Row::Handler` children for `handlers[1..]` (alternatives, right below parent)
2. `Row::Exception` children (MIME-level overrides)
3. One `Row::AddException` sentinel (renders the "＋ Add exception" button)

Categories always have children (at minimum the AddException row), so the expand arrow
is always present.

### App display names

`gio::DesktopAppInfo::new(desktop)?.name()` is used everywhere. `humanize()` (strip
reverse-DNS prefix, capitalise) is the fallback when no `.desktop` file is found.

---

## 4. Operations / handlr CLI Binding

| User action | Forward | Inverse |
|---|---|---|
| Set category default (was empty) | `set mime app` | `unset mime` |
| Change category default (was `[A, …]`) | `set mime app` | `set mime A` + `add mime …` |
| Clear category default — has alternatives `[A, B, …]` | `remove mime A` | `set mime A` + `add mime B …` |
| Clear category default — no alternatives `[A]` | `unset mime` | `set mime A` |
| Add exception (no prior) | `set mime app` | `unset mime` |
| Remove exception `[A, …]` | `unset mime` | `set mime A` + `add mime …` |
| Add secondary handler | `add mime app` | `remove mime app` |
| Remove secondary handler | `remove mime app` | `add mime app` |
| Promote handler at index i | `set mime h[i]` + `add` N-1 others in new order | `set mime h[0]` + `add` N-1 others in prior order |

`clear_category_default` uses `remove` (not `unset`) when alternatives exist so the
next handler is promoted rather than clearing the entire entry.

`move_handler_up` issues a full reorder (Set + N-1 Adds) for both forward and inverse,
making it safe even when the list has changed between bind and click.

---

## 5. Keyboard Shortcuts

| Key | Action |
|---|---|
| `Ctrl+Z` | Undo |
| `Ctrl+Shift+Z` / `Ctrl+Y` | Redo |
| `Ctrl+R` | Reload (re-runs `handlr list` + reloads `handlr.toml`, refreshes handler cards) |
| `Ctrl+Q` | Quit |
| `←` / `→` | Collapse / expand focused tree row (capture-phase controller) |
| `Delete` | Clear category default / remove exception / remove handler |
| `Insert` | Open app picker for focused Category (add handler) or Exception (change handler) |

`←` / `→` use a capture-phase `EventControllerKey` on the list view so they intercept
before GTK's focus traversal moves focus to row buttons.

---

## 6. Drag-and-Drop

Drop target: the whole `GtkApplicationWindow` accepts `gdk::FileList`.

**Defaults tab**: drops a file → detects MIME via `handlr mime --json`, scrolls tree
to the matching row (exact match or longest wildcard prefix).

**Tester tab**: the entire tab is also a drop target (outer box). A dropped file fills
the entry and triggers resolution immediately.

---

## 7. Tester Tab

Input: a text entry accepting a file path, URL, or raw MIME type. Drag-and-drop to
the whole tab is supported.

Resolution order:

1. **Regex handlers** — iterate `Config.handlers`, test each `regex` against the raw
   input string. First match wins.
2. **MIME detection** — `handlr mime --json <input>` (as file path or raw string).
3. **MIME default lookup** — find the detected MIME in `State.defaults`; fall back to
   wildcard prefix.

The result area always shows the full chain regardless of where matching stopped:

- Each regex attempt: handler display name, pattern, matched/no match/invalid.
- A separator line.
- MIME detection row (result or error).
- MIME handler lookup row (matched handler or "—" if skipped because regex won).

Rows after the winning step are rendered dimmed with a "—" marker to show they were
not evaluated.

---

## 8. Regex Handlers Tab

Expandable cards, one per `[[handlers]]` entry. Each card has:

- **Collapsed header**: toggle ▶/▼, exec label (truncated), regex count, app icon.
- **Expanded body**: regex entries (GtkEntry + ✕ per row, live validation via
  `regex::Regex::new`), exec entry with "Browse apps…" popover, terminal checkbox,
  Delete/Apply buttons.
- **New (unsaved) card**: dashed border, "Discard" instead of Delete.

Apply validates (≥1 non-empty regex, non-empty exec), saves via `toml_edit`, collapses
card. Invalid regex entries get an `error` CSS class (red border); Apply is insensitive
while any entry is invalid.

Handler reordering: ▲/▼ icon buttons in header, swap positions i and i±1, rebuild
cards. Full DnD reordering deferred.

---

## 9. Settings Dialog

Non-modal `gtk4::Window` (gear button). Fields:

| Field | Config key | Widget |
|---|---|---|
| Enable selector | `enable_selector` | GtkSwitch |
| Selector command | `selector` | GtkEntry |
| Terminal exec args | `term_exec_args` | GtkEntry |
| Expand wildcards | `expand_wildcards` | GtkSwitch |

Each change saves immediately via `config::save()`. Errors shown in an inline label.
Escape closes the dialog.

---

## 10. App Picker Popover

Anchored to the triggering button. Contains `GtkSearchEntry` + `GtkListView` of
system apps. Filtering is live (custom filter on every keystroke). Enter activates the
first visible item. Down arrow moves focus to the list. Escape closes via a
capture-phase `EventControllerKey` on the popover (required because `GtkSearchEntry`
would otherwise consume Escape for clearing its text).

---

## 11. Error Handling

- **Startup**: `handlr --version` failure → modal alert, quit.
- **Subprocess failure**: banner via `GtkInfoBar` revealer; tree rebuilt defensively.
- **Config save failure**: inline error label in the settings dialog or handler card.
- **Regex parse errors**: live red border on entry; Apply blocked.
- **`DesktopAppInfo` lookup failure**: `humanize()` fallback, no error surfaced.
- **All dialogs/popovers**: closeable with Escape.

---

## 12. Dependencies

| Crate | Purpose |
|---|---|
| `gtk4` (v4_12) | GTK4 bindings (includes gio, glib, gdk) |
| `serde`, `serde_json` | Parse `handlr list --all --json` output |
| `anyhow` | Error propagation |
| `toml_edit` | Round-trip `handlr.toml` read/write |
| `regex` | Live validation and resolution matching in the tester |
| `tempfile` (dev) | Hermetic config tests |

---

## 13. Packaging

`handlr-gui.desktop` — `Categories=Settings;`, icon `preferences-desktop-default-applications`.

`Makefile` targets:
- `build` — `cargo build --release`
- `install` — binary to `$(DESTDIR)$(PREFIX)/bin`, desktop file to `.../applications/`
- `install-user` — installs to `~/.local/bin` and `~/.local/share/applications/`
- `uninstall`

`PREFIX` defaults to `/usr/local`; `DESTDIR` supported for distro packaging.

---

## 14. Project Structure

```
handlr-gui/
├── Cargo.toml
├── Makefile
├── handlr-gui.desktop
├── AGENTS.md
├── docs/
│   ├── spec.md                  (this file)
│   ├── project-plan.md
│   ├── handlr-regex-reference.md
│   └── superpowers/
│       ├── specs/               (brainstorming design docs)
│       └── plans/               (implementation plans)
└── src/
    ├── main.rs       app entry, GtkApplication setup, startup error dialog
    ├── handlr.rs     subprocess wrappers, JSON types, HandlrCmd execution
    ├── model.rs      Row, AppState, tree-building logic
    ├── ui.rs         build_window(), all tabs, popovers, dialogs, shortcuts
    ├── undo.rs       UndoEntry builders and stack management
    ├── config.rs     Config + RegexHandler, toml_edit round-trip
    └── dnd.rs        drop target setup, MIME lookup, scroll-to-row
```
