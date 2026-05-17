//! Window shell + interactive wiring. Header bar, info bar, list view + tree model +
//! factory, app-picker popover, add-exception dialog, global keyboard shortcuts.

use crate::handlr;
use crate::model::{self, AppState, Row};
use crate::undo::{self, UndoEntry};
use gtk4::gdk;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::subclass::prelude::ObjectSubclassIsExt;
use std::cell::RefCell;
use std::rc::Rc;

mod imp {
    use super::*;
    use glib::subclass::prelude::*;

    #[derive(Default)]
    pub(crate) struct RowObject {
        pub(super) row: RefCell<Option<Row>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RowObject {
        const NAME: &'static str = "HandlrGuiRowObject";
        type Type = super::RowObject;
    }

    impl ObjectImpl for RowObject {}

    #[derive(Default)]
    pub(crate) struct AppObject {
        pub(super) app: RefCell<Option<handlr::App>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AppObject {
        const NAME: &'static str = "HandlrGuiAppObject";
        type Type = super::AppObject;
    }

    impl ObjectImpl for AppObject {}
}

glib::wrapper! {
    pub(crate) struct RowObject(ObjectSubclass<imp::RowObject>);
}

impl RowObject {
    fn new(row: Row) -> Self {
        let obj: Self = glib::Object::new();
        obj.imp().row.replace(Some(row));
        obj
    }

    fn row(&self) -> Row {
        self.imp().row.borrow().clone().expect("RowObject row not set")
    }
}

glib::wrapper! {
    pub(crate) struct AppObject(ObjectSubclass<imp::AppObject>);
}

impl AppObject {
    fn new(app: handlr::App) -> Self {
        let obj: Self = glib::Object::new();
        obj.imp().app.replace(Some(app));
        obj
    }

    fn desktop(&self) -> String {
        self.imp().app.borrow().as_ref().expect("app not set").desktop.clone()
    }

    fn name(&self) -> String {
        self.imp().app.borrow().as_ref().expect("app not set").name.clone()
    }
}

// Bundle of Rc-clonable references threaded into every action closure. Lets us hand
// one struct to bind_row instead of juggling N captured clones per closure.
#[derive(Clone)]
struct Wiring {
    state: Rc<RefCell<AppState>>,
    root_store: gio::ListStore,
    stack: gtk4::Stack,
    undo_btn: gtk4::Button,
    redo_btn: gtk4::Button,
    revealer: gtk4::Revealer,
    info_label: gtk4::Label,
    window: gtk4::ApplicationWindow,
}

// Handle to the parts of the window that out-of-module callers (dnd, future features) need
// to reach into. Kept small: just enough to find a row, scroll to it, and surface errors.
#[derive(Clone)]
pub(crate) struct TreeHandle {
    pub(crate) list_view: gtk4::ListView,
    pub(crate) root_store: gio::ListStore,
    pub(crate) revealer: gtk4::Revealer,
    pub(crate) info_label: gtk4::Label,
}

// Repopulate the root store from the current AppState. Call after mutating
// AppState without going through apply/undo/redo (e.g. add_pending_exception).
pub(crate) fn rebuild_tree(handle: &TreeHandle, state: &std::rc::Rc<std::cell::RefCell<AppState>>) {
    populate_root(&handle.root_store, state);
}

pub(crate) fn build_window(
    app: &gtk4::Application,
    state: Rc<RefCell<AppState>>,
    config: Rc<RefCell<crate::config::Config>>,
) -> (gtk4::ApplicationWindow, TreeHandle) {
    let window = gtk4::ApplicationWindow::builder()
        .application(app)
        .title("handlr-gui")
        .default_width(720)
        .default_height(600)
        .build();

    init_row_css(&window);

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    window.set_child(Some(&vbox));

    // Header bar with action buttons.
    let header = gtk4::HeaderBar::new();

    let undo_btn = gtk4::Button::from_icon_name("edit-undo-symbolic");
    undo_btn.set_tooltip_text(Some("Undo"));
    undo_btn.set_sensitive(false);
    header.pack_start(&undo_btn);

    let redo_btn = gtk4::Button::from_icon_name("edit-redo-symbolic");
    redo_btn.set_tooltip_text(Some("Redo"));
    redo_btn.set_sensitive(false);
    header.pack_start(&redo_btn);

    // Outer tab stack: Defaults (tree) | Regex Handlers (stub).
    let main_tabs = gtk4::Stack::new();
    main_tabs.set_vexpand(true);

    let switcher = gtk4::StackSwitcher::new();
    switcher.set_stack(Some(&main_tabs));
    header.set_title_widget(Some(&switcher));

    // Pack end items right-to-left: hamburger | gear | reload (left to right in header).
    let menu = gio::Menu::new();
    menu.append(Some("About"), Some("win.about"));
    menu.append(Some("Quit"), Some("win.quit"));
    let menu_btn = gtk4::MenuButton::new();
    menu_btn.set_icon_name("open-menu-symbolic");
    menu_btn.set_menu_model(Some(&menu));
    header.pack_end(&menu_btn);

    let gear_btn = gtk4::Button::from_icon_name("preferences-system-symbolic");
    gear_btn.set_tooltip_text(Some("Settings"));
    header.pack_end(&gear_btn);

    let reload_btn = gtk4::Button::from_icon_name("view-refresh-symbolic");
    reload_btn.set_tooltip_text(Some("Reload"));
    header.pack_end(&reload_btn);

    window.set_titlebar(Some(&header));

    // Error info bar wrapped in a revealer. InfoBar is deprecated since GTK 4.10
    // but it's the spec'd widget and there's no drop-in replacement in v1.
    #[allow(deprecated)]
    let (revealer, info_label) = build_info_bar(&vbox);

    // Stack to flip between populated tree and the empty placeholder.
    let stack = gtk4::Stack::new();
    stack.set_vexpand(true);

    // Build the list view shell first (model + factory setup, no bind callback yet)
    // so we can put it in TreeHandle and attach bind_row after Wiring is constructed.
    let root_store = gio::ListStore::new::<RowObject>();
    let (list_view, factory) = build_list_view(root_store.clone(), state.clone());

    let wiring = Wiring {
        state: state.clone(),
        root_store: root_store.clone(),
        stack: stack.clone(),
        undo_btn: undo_btn.clone(),
        redo_btn: redo_btn.clone(),
        revealer: revealer.clone(),
        info_label: info_label.clone(),
        window: window.clone(),
    };

    // Attach the bind callback now that Wiring is fully constructed.
    {
        let wiring = wiring.clone();
        factory.connect_bind(move |_factory, item| bind_row(item, &wiring));
    }

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_child(Some(&list_view));
    stack.add_named(&scrolled, Some("tree"));

    let empty_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    empty_box.set_halign(gtk4::Align::Center);
    empty_box.set_valign(gtk4::Align::Center);
    let empty_label = gtk4::Label::new(Some(
        "No defaults configured. Drop a file here or use the CLI to add one.",
    ));
    empty_box.append(&empty_label);
    stack.add_named(&empty_box, Some("empty"));

    // Wrap inner stack in a named page of main_tabs.
    main_tabs.add_titled(&stack, Some("defaults"), "Defaults");

    let (regex_tab, refresh_handlers) = build_regex_handlers_tab(config.clone(), state.clone());
    main_tabs.add_titled(&regex_tab, Some("regex-handlers"), "Regex Handlers");

    vbox.append(&main_tabs);

    // Initial population + header sensitivity.
    refresh_view(&wiring);

    // Reload.
    {
        let wiring = wiring.clone();
        let config = config.clone();
        reload_btn.connect_clicked(move |_| match handlr::list_all() {
            Ok(s) => {
                wiring.state.borrow_mut().set_state(s);
                if let Ok(new_cfg) = crate::config::load() {
                    *config.borrow_mut() = new_cfg;
                }
                refresh_handlers();
                refresh_view(&wiring);
                hide_banner(&wiring.revealer);
            }
            Err(e) => show_error(
                &wiring.revealer,
                &wiring.info_label,
                &format!("Reload failed: {}", e),
            ),
        });
    }

    // Undo / Redo.
    {
        let wiring = wiring.clone();
        undo_btn.connect_clicked(move |_| {
            let result = wiring.state.borrow_mut().undo();
            handle_action_result(&wiring, result.map(|_| ()));
        });
    }
    {
        let wiring = wiring.clone();
        redo_btn.connect_clicked(move |_| {
            let result = wiring.state.borrow_mut().redo();
            handle_action_result(&wiring, result.map(|_| ()));
        });
    }

    // Win actions: about, quit, and click-forwarders so Ctrl+R/Z/Y/Shift+Z can fire
    // the header buttons through NamedAction triggers.
    add_action(&window, "about", {
        let window = window.clone();
        move || {
            gtk4::AboutDialog::builder()
                .program_name("handlr-gui")
                .comments("GTK4 frontend for the handlr CLI: manage default MIME associations.")
                .transient_for(&window)
                .modal(true)
                .build()
                .present();
        }
    });
    add_action(&window, "quit", {
        let app = app.clone();
        move || app.quit()
    });
    add_action(&window, "reload", {
        let b = reload_btn.clone();
        move || b.emit_clicked()
    });
    add_action(&window, "undo", {
        let b = undo_btn.clone();
        move || if b.is_sensitive() { b.emit_clicked() }
    });
    add_action(&window, "redo", {
        let b = redo_btn.clone();
        move || if b.is_sensitive() { b.emit_clicked() }
    });

    install_shortcuts(&window);

    // Settings dialog — single instance, present() if already open.
    let settings_win: Rc<RefCell<Option<gtk4::Window>>> = Rc::new(RefCell::new(None));
    {
        let window = window.clone();
        let config = config.clone();
        let settings_win = settings_win.clone();
        gear_btn.connect_clicked(move |_| {
            if let Some(existing) = settings_win.borrow().as_ref() {
                existing.present();
                return;
            }
            let dlg = build_settings_dialog(&window, config.clone());
            dlg.connect_close_request({
                let settings_win = settings_win.clone();
                move |_| {
                    *settings_win.borrow_mut() = None;
                    glib::Propagation::Proceed
                }
            });
            dlg.present();
            *settings_win.borrow_mut() = Some(dlg);
        });
    }

    // Delete on the focused tree row: clear/remove per §6.
    {
        let wiring = wiring.clone();
        let list_view_for_key = list_view.clone();
        let key_ctrl = gtk4::EventControllerKey::new();
        key_ctrl.connect_key_pressed(move |_, key, _, _| {
            if key != gdk::Key::Delete {
                return glib::Propagation::Proceed;
            }
            let Some(entry) = delete_target(&list_view_for_key) else {
                return glib::Propagation::Proceed;
            };
            apply_entry(&wiring, entry);
            glib::Propagation::Stop
        });
        list_view.add_controller(key_ctrl);
    }

    let tree = TreeHandle {
        list_view: list_view.clone(),
        root_store: root_store.clone(),
        revealer: revealer.clone(),
        info_label: info_label.clone(),
    };
    (window, tree)
}

// Find the row whose mime matches `mime`, scroll the list view to it, and return true.
// Returns false when no row matches — caller decides how to surface that (e.g. banner).
pub(crate) fn scroll_to_mime(handle: &TreeHandle, mime: &str) -> bool {
    let Some(idx) = locate_mime(handle, mime) else {
        return false;
    };
    // SELECT highlights the row so the jump is visible even when the row was already
    // on-screen; FOCUS lets keyboard navigation continue from there.
    handle.list_view.scroll_to(
        idx,
        gtk4::ListScrollFlags::FOCUS | gtk4::ListScrollFlags::SELECT,
        None,
    );
    true
}

// Walk the flattened TreeListModel that backs the ListView (NOT the root_store —
// its indices are the unexpanded root positions and don't match the visible list).
// Prefer an exact Category/Exception mime match, falling back to the longest wildcard
// prefix on a Category row.
fn locate_mime(handle: &TreeHandle, mime: &str) -> Option<u32> {
    let model = handle.list_view.model()?;
    let n = model.n_items();
    let mut best: Option<(usize, u32)> = None; // (prefix_len, flat index)

    for i in 0..n {
        let Some(item) = model.item(i) else { continue };
        let Some(tree_row) = item.downcast_ref::<gtk4::TreeListRow>() else {
            continue;
        };
        let Some(row_obj) = tree_row.item().and_then(|o| o.downcast::<RowObject>().ok())
        else {
            continue;
        };
        match row_obj.row() {
            Row::Category { mime: cat, .. } if cat == mime => return Some(i),
            Row::Exception { mime: exc, .. } if exc == mime => return Some(i),
            Row::Category { mime: cat, .. } => {
                if let Some(prefix) = cat.strip_suffix('*')
                    && mime.starts_with(prefix)
                    && prefix.len() > best.map(|b| b.0).unwrap_or(0)
                {
                    best = Some((prefix.len(), i));
                }
            }
            _ => {}
        }
    }
    best.map(|(_, idx)| idx)
}

// Surface a message in the window's info bar. Used by out-of-module callers (dnd) that
// don't have access to the private Wiring struct.
pub(crate) fn show_banner(handle: &TreeHandle, msg: &str) {
    handle.info_label.set_text(msg);
    handle.revealer.set_reveal_child(true);
}

fn add_action(window: &gtk4::ApplicationWindow, name: &str, callback: impl Fn() + 'static) {
    let action = gio::SimpleAction::new(name, None);
    action.connect_activate(move |_, _| callback());
    window.add_action(&action);
}

// Global keyboard shortcuts (§6). Row-level keys (Enter/Delete/Insert) are deferred —
// the button affordances cover the operations in v1.
fn install_shortcuts(window: &gtk4::ApplicationWindow) {
    let controller = gtk4::ShortcutController::new();
    controller.set_scope(gtk4::ShortcutScope::Global);

    let bindings = [
        ("<Control>z", "win.undo"),
        ("<Control><Shift>z", "win.redo"),
        ("<Control>y", "win.redo"),
        ("<Control>r", "win.reload"),
        ("<Control>q", "win.quit"),
    ];
    for (trigger, action) in bindings {
        let Some(t) = gtk4::ShortcutTrigger::parse_string(trigger) else {
            continue;
        };
        let a = gtk4::NamedAction::new(action);
        controller.add_shortcut(gtk4::Shortcut::new(Some(t), Some(a)));
    }
    window.add_controller(controller);
}

#[allow(deprecated)]
fn build_info_bar(vbox: &gtk4::Box) -> (gtk4::Revealer, gtk4::Label) {
    let revealer = gtk4::Revealer::new();
    revealer.set_reveal_child(false);
    let info_bar = gtk4::InfoBar::new();
    info_bar.set_message_type(gtk4::MessageType::Error);
    info_bar.set_show_close_button(true);
    let info_label = gtk4::Label::new(None);
    info_label.set_xalign(0.0);
    info_label.set_wrap(true);
    info_bar.add_child(&info_label);
    revealer.set_child(Some(&info_bar));
    vbox.append(&revealer);

    let revealer_clone = revealer.clone();
    info_bar.connect_response(move |_, _| revealer_clone.set_reveal_child(false));

    (revealer, info_label)
}

fn show_error(revealer: &gtk4::Revealer, label: &gtk4::Label, msg: &str) {
    label.set_text(msg);
    revealer.set_reveal_child(true);
}

fn hide_banner(revealer: &gtk4::Revealer) {
    revealer.set_reveal_child(false);
}

// Common tail for apply/undo/redo: refresh tree + header, surface errors.
fn handle_action_result(wiring: &Wiring, result: anyhow::Result<()>) {
    refresh_view(wiring);
    match result {
        Ok(()) => hide_banner(&wiring.revealer),
        Err(e) => show_error(&wiring.revealer, &wiring.info_label, &format!("{}", e)),
    }
}

fn apply_entry(wiring: &Wiring, entry: UndoEntry) {
    // On Err: AppState may have already applied the cmd to disk but failed to resync.
    // The tree shows pre-action state until Reload. show_error surfaces this to the user.
    let result = wiring.state.borrow_mut().apply(entry);
    handle_action_result(wiring, result);
}

fn refresh_view(wiring: &Wiring) {
    populate_root(&wiring.root_store, &wiring.state);
    update_stack(&wiring.stack, &wiring.state);
    refresh_header(&wiring.state, &wiring.undo_btn, &wiring.redo_btn);
}

fn refresh_header(
    state: &Rc<RefCell<AppState>>,
    undo_btn: &gtk4::Button,
    redo_btn: &gtk4::Button,
) {
    let s = state.borrow();
    undo_btn.set_sensitive(s.can_undo());
    undo_btn.set_tooltip_text(Some(
        &s.undo_label()
            .map(|l| format!("Undo: {}", l))
            .unwrap_or_else(|| "Undo".to_string()),
    ));
    redo_btn.set_sensitive(s.can_redo());
    redo_btn.set_tooltip_text(Some(
        &s.redo_label()
            .map(|l| format!("Redo: {}", l))
            .unwrap_or_else(|| "Redo".to_string()),
    ));
}

fn update_stack(stack: &gtk4::Stack, state: &Rc<RefCell<AppState>>) {
    let s = state.borrow();
    let any_handlers = s.state().defaults.iter().any(|(_, h)| !h.is_empty());
    let stack_name = if any_handlers { "tree" } else { "empty" };
    stack.set_visible_child_name(stack_name);
}

fn populate_root(root_store: &gio::ListStore, state: &Rc<RefCell<AppState>>) {
    root_store.remove_all();
    for row in model::build_categories(state.borrow().state()) {
        root_store.append(&RowObject::new(row));
    }
}

// Derive the UndoEntry for "delete the focused row". None when there's nothing to
// delete (e.g. empty Category, or the default Handler row whose removal must go through
// "change default" instead).
fn delete_target(list_view: &gtk4::ListView) -> Option<UndoEntry> {
    let selection = list_view
        .model()
        .and_then(|m| m.downcast::<gtk4::SingleSelection>().ok())?;
    let pos = selection.selected();
    let tree_row = selection.item(pos)?.downcast::<gtk4::TreeListRow>().ok()?;
    let row_obj = tree_row.item().and_then(|o| o.downcast::<RowObject>().ok())?;
    match row_obj.row() {
        Row::Category { mime, handlers } if !handlers.is_empty() => {
            Some(undo::clear_category_default(&mime, &handlers))
        }
        Row::Exception { mime, handlers } => Some(undo::remove_exception(&mime, &handlers)),
        Row::Handler { mime, desktop, index, .. } if index > 0 => {
            Some(undo::remove_handler(&mime, &desktop))
        }
        _ => None,
    }
}

// Returns (list_view, factory) so the caller can attach connect_bind after Wiring
// is constructed — bind_row needs Wiring, and Wiring carries the list_view.
fn build_list_view(
    root_store: gio::ListStore,
    state: Rc<RefCell<AppState>>,
) -> (gtk4::ListView, gtk4::SignalListItemFactory) {
    // passthrough=false, autoexpand=true: model items are RowObject directly, and categories auto-expand on first show.
    let tree_model = gtk4::TreeListModel::new(
        root_store,
        false,
        true,
        move |parent_obj: &glib::Object| create_children(parent_obj, &state),
    );

    let selection = gtk4::SingleSelection::new(Some(tree_model));

    let factory = gtk4::SignalListItemFactory::new();
    factory.connect_setup(setup_row);

    let list_view = gtk4::ListView::new(Some(selection), Some(factory.clone()));
    (list_view, factory)
}

fn create_children(
    parent_obj: &glib::Object,
    state: &Rc<RefCell<AppState>>,
) -> Option<gio::ListModel> {
    let row_obj: &RowObject = parent_obj.downcast_ref()?;
    let row = row_obj.row();
    match row {
        Row::Category { mime, handlers } => {
            let store = gio::ListStore::new::<RowObject>();
            // Skip handlers[0] — shown inline on the category row itself.
            for child in model::handlers_for(&mime, &handlers).into_iter().skip(1) {
                store.append(&RowObject::new(child));
            }
            for child in model::exceptions_for(&mime, state.borrow().state()) {
                store.append(&RowObject::new(child));
            }
            if store.n_items() == 0 {
                None
            } else {
                Some(store.upcast())
            }
        }
        Row::Exception { mime, handlers } => {
            let store = gio::ListStore::new::<RowObject>();
            // Skip handlers[0] — shown inline on the exception row itself.
            for child in model::handlers_for(&mime, &handlers).into_iter().skip(1) {
                store.append(&RowObject::new(child));
            }
            if store.n_items() == 0 {
                None
            } else {
                Some(store.upcast())
            }
        }
        Row::Handler { .. } => None,
    }
}

// Row template:
//   [main_icon] [main_label] [spacer*hexpand] [handler_icon] [handler_label] [badge] [actions]
// Cat/Exc rows show MIME on the left and the inline default handler on the right.
// Handler (secondary) rows reuse main_icon/main_label for the app icon+name and hide
// the handler_* widgets. bind_row clears+repopulates actions per variant so old click
// handlers from recycled rows don't leak.
fn setup_row(_factory: &gtk4::SignalListItemFactory, item: &glib::Object) {
    let item: &gtk4::ListItem = item.downcast_ref().unwrap();
    let expander = gtk4::TreeExpander::new();

    let row_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);

    let main_icon = gtk4::Image::new();
    main_icon.set_pixel_size(24);
    row_box.append(&main_icon);

    let main_label = gtk4::Label::new(None);
    main_label.set_xalign(0.0);
    row_box.append(&main_label);

    let spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    row_box.append(&spacer);

    let handler_icon = gtk4::Image::new();
    handler_icon.set_pixel_size(20);
    row_box.append(&handler_icon);

    let handler_label = gtk4::Label::new(None);
    handler_label.set_xalign(0.0);
    handler_label.add_css_class("dim-label");
    row_box.append(&handler_label);

    let badge = gtk4::Label::new(None);
    badge.set_xalign(0.0);
    badge.add_css_class("dim-label");
    row_box.append(&badge);

    let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
    row_box.append(&actions);

    expander.set_child(Some(&row_box));
    item.set_child(Some(&expander));
}

struct RowWidgets {
    main_icon: gtk4::Image,
    main_label: gtk4::Label,
    handler_icon: gtk4::Image,
    handler_label: gtk4::Label,
    badge: gtk4::Label,
    actions: gtk4::Box,
}

fn unpack_row_widgets(expander: &gtk4::TreeExpander) -> Option<RowWidgets> {
    let row_box = expander.child().and_then(|c| c.downcast::<gtk4::Box>().ok())?;
    let main_icon = row_box.first_child()?.downcast::<gtk4::Image>().ok()?;
    let main_label = main_icon.next_sibling()?.downcast::<gtk4::Label>().ok()?;
    let spacer = main_label.next_sibling()?.downcast::<gtk4::Box>().ok()?;
    let handler_icon = spacer.next_sibling()?.downcast::<gtk4::Image>().ok()?;
    let handler_label = handler_icon.next_sibling()?.downcast::<gtk4::Label>().ok()?;
    let badge = handler_label.next_sibling()?.downcast::<gtk4::Label>().ok()?;
    let actions = badge.next_sibling()?.downcast::<gtk4::Box>().ok()?;
    Some(RowWidgets {
        main_icon,
        main_label,
        handler_icon,
        handler_label,
        badge,
        actions,
    })
}

fn clear_actions(actions: &gtk4::Box) {
    while let Some(child) = actions.first_child() {
        actions.remove(&child);
    }
}

fn make_action_btn(icon: &str, tooltip: &str) -> gtk4::Button {
    let b = gtk4::Button::from_icon_name(icon);
    b.add_css_class("flat");
    b.set_tooltip_text(Some(tooltip));
    b
}

fn bind_row(item: &glib::Object, wiring: &Wiring) {
    let item: &gtk4::ListItem = item.downcast_ref().unwrap();
    let Some(tree_row) = item.item().and_then(|o| o.downcast::<gtk4::TreeListRow>().ok()) else {
        return;
    };
    let Some(row_obj) = tree_row.item().and_then(|o| o.downcast::<RowObject>().ok()) else {
        return;
    };
    let Some(expander) = item.child().and_then(|c| c.downcast::<gtk4::TreeExpander>().ok())
    else {
        return;
    };
    expander.set_list_row(Some(&tree_row));
    let Some(w) = unpack_row_widgets(&expander) else {
        return;
    };
    clear_actions(&w.actions);

    // Reset colour classes from any previous bind before applying the right one.
    w.main_label.remove_css_class("mime-category");
    w.main_label.remove_css_class("mime-exception");

    match row_obj.row() {
        Row::Category { mime, handlers } => {
            set_mime_icon(&w.main_icon, &strip_wildcard(&mime));
            w.main_label.set_text(&mime);
            w.main_label.add_css_class("mime-category");
            set_handler_inline(&w.handler_icon, &w.handler_label, handlers.first().map(String::as_str));
            w.badge.set_text("");

            if !handlers.is_empty() {
                w.actions.append(&pick_btn(
                    "document-edit-symbolic",
                    "Change default",
                    wiring,
                    {
                        let mime = mime.clone();
                        let handlers = handlers.clone();
                        move |desktop| undo::set_category_default(&mime, &desktop, &handlers)
                    },
                ));
            }
            w.actions.append(&pick_btn("list-add-symbolic", "Add handler", wiring, {
                let mime = mime.clone();
                let handlers_empty = handlers.is_empty();
                move |desktop| {
                    if handlers_empty {
                        undo::set_category_default(&mime, &desktop, &[])
                    } else {
                        undo::add_handler(&mime, &desktop)
                    }
                }
            }));
            let except = make_action_btn("mail-message-new-symbolic", "Add exception");
            let wiring_cl = wiring.clone();
            except.connect_clicked(move |_| open_add_exception_dialog(&wiring_cl, &mime));
            w.actions.append(&except);
        }
        Row::Exception { mime, handlers } => {
            set_mime_icon(&w.main_icon, &mime);
            w.main_label.set_text(&mime);
            w.main_label.add_css_class("mime-exception");
            set_handler_inline(&w.handler_icon, &w.handler_label, handlers.first().map(String::as_str));

            if handlers.is_empty() {
                // Pending (not yet configured in handlr). No handler to remove; offer Set.
                w.badge.set_text("");
                w.actions.append(&pick_btn("list-add-symbolic", "Set handler", wiring, {
                    let mime = mime.clone();
                    move |desktop| undo::set_category_default(&mime, &desktop, &[])
                }));
            } else {
                w.badge.set_text("");
                w.actions.append(&pick_btn(
                    "document-edit-symbolic",
                    "Change default",
                    wiring,
                    {
                        let mime = mime.clone();
                        let handlers = handlers.clone();
                        move |desktop| undo::set_category_default(&mime, &desktop, &handlers)
                    },
                ));
                w.actions.append(&simple_btn(
                    "user-trash-symbolic",
                    "Remove exception",
                    wiring,
                    move || undo::remove_exception(&mime, &handlers),
                ));
            }
        }
        Row::Handler { mime, desktop, index, .. } => {
            let info = gio::DesktopAppInfo::new(&desktop);
            if let Some(g) = info.as_ref().and_then(|a| a.icon()) {
                w.main_icon.set_from_gicon(&g);
            } else {
                w.main_icon.set_icon_name(Some("applications-other-symbolic"));
            }
            w.main_label.set_text(&handler_display_name(&desktop, info.as_ref()));
            w.handler_icon.set_visible(false);
            w.handler_label.set_text("");
            w.badge.set_text("");
            // create_children only emits index >= 1, but be defensive: index 0 has no action.
            if index > 0 {
                w.actions.append(&simple_btn(
                    "user-trash-symbolic",
                    "Remove handler",
                    wiring,
                    move || undo::remove_handler(&mime, &desktop),
                ));
            }
        }
    }
}

// Render the inline "default handler" segment on a Category/Exception row.
// `desktop` is the .desktop filename of the default handler, or None for an empty category.
fn set_handler_inline(icon: &gtk4::Image, label: &gtk4::Label, desktop: Option<&str>) {
    match desktop {
        Some(d) => {
            let info = gio::DesktopAppInfo::new(d);
            if let Some(g) = info.as_ref().and_then(|a| a.icon()) {
                icon.set_from_gicon(&g);
            } else {
                icon.set_icon_name(Some("applications-other-symbolic"));
            }
            icon.set_visible(true);
            label.set_text(&handler_display_name(d, info.as_ref()));
        }
        None => {
            icon.set_visible(false);
            label.set_text("(no default)");
        }
    }
}

// Resolve a .desktop filename to a human name. Fallback intentionally differs from
// handlr::humanize: we keep the full reverse-DNS id (e.g. "org.gnome.Nautilus") so the
// user sees what failed to resolve, rather than the capitalized last component.
fn handler_display_name(desktop: &str, info: Option<&gio::DesktopAppInfo>) -> String {
    info.map(|a| a.name().to_string())
        .unwrap_or_else(|| desktop.strip_suffix(".desktop").unwrap_or(desktop).to_string())
}

// Build an action button that opens the picker and applies an UndoEntry derived from
// the chosen desktop. `make_entry` is called once per pick.
fn pick_btn<F>(icon: &str, tooltip: &str, wiring: &Wiring, make_entry: F) -> gtk4::Button
where
    F: Fn(String) -> UndoEntry + Clone + 'static,
{
    let btn = make_action_btn(icon, tooltip);
    let wiring = wiring.clone();
    btn.connect_clicked(move |b| {
        let wiring_inner = wiring.clone();
        let make_entry = make_entry.clone();
        let apps = system_apps(&wiring);
        show_app_picker(b.upcast_ref::<gtk4::Widget>(), &apps, move |desktop| {
            apply_entry(&wiring_inner, make_entry(desktop));
        });
    });
    btn
}

// Build an action button that synchronously builds & applies an UndoEntry on click.
fn simple_btn<F>(icon: &str, tooltip: &str, wiring: &Wiring, make_entry: F) -> gtk4::Button
where
    F: Fn() -> UndoEntry + 'static,
{
    let btn = make_action_btn(icon, tooltip);
    let wiring = wiring.clone();
    btn.connect_clicked(move |_| apply_entry(&wiring, make_entry()));
    btn
}

fn system_apps(wiring: &Wiring) -> Vec<handlr::App> {
    wiring
        .state
        .borrow()
        .state()
        .system_apps
        .iter()
        .map(|a| handlr::App {
            desktop: a.desktop.clone(),
            name: a.name.clone(),
        })
        .collect()
}

// Load CSS that colours MIME labels by row variant using named theme colours.
// @accent_color and @success_color are defined by Adwaita and most GTK4 themes,
// so the palette adapts when the user switches themes.
fn init_row_css(window: &gtk4::ApplicationWindow) {
    let provider = gtk4::CssProvider::new();
    provider.load_from_string(
        ".mime-category { color: @accent_color; }\
         .mime-exception { color: @success_color; }",
    );
    #[allow(deprecated)]
    gtk4::style_context_add_provider_for_display(
        &gtk4::prelude::WidgetExt::display(window),
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn set_mime_icon(image: &gtk4::Image, mime: &str) {
    let icon = gio::functions::content_type_get_icon(mime);
    image.set_from_gicon(&icon);
}

fn strip_wildcard(mime: &str) -> String {
    // "video/*" -> "video/x-generic" (freedesktop's generic icon convention).
    if let Some(top) = mime.strip_suffix("/*") {
        format!("{}/x-generic", top)
    } else {
        mime.to_string()
    }
}

// --- App picker popover ---------------------------------------------------------------

fn show_app_picker(
    anchor: &gtk4::Widget,
    apps: &[handlr::App],
    on_chosen: impl Fn(String) + 'static,
) {
    let popover = gtk4::Popover::new();
    popover.set_has_arrow(true);
    popover.set_autohide(true);
    popover.set_parent(anchor);

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    vbox.set_size_request(320, -1);

    let search = gtk4::SearchEntry::new();
    vbox.append(&search);

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_min_content_height(280);
    scrolled.set_hscrollbar_policy(gtk4::PolicyType::Never);

    let store = gio::ListStore::new::<AppObject>();
    for a in apps {
        store.append(&AppObject::new(handlr::App {
            desktop: a.desktop.clone(),
            name: a.name.clone(),
        }));
    }

    let search_for_filter = search.clone();
    let filter = gtk4::CustomFilter::new(move |obj: &glib::Object| {
        let Some(ao) = obj.downcast_ref::<AppObject>() else {
            return false;
        };
        let needle = search_for_filter.text().to_lowercase();
        if needle.is_empty() {
            return true;
        }
        ao.name().to_lowercase().contains(&needle)
            || ao.desktop().to_lowercase().contains(&needle)
    });

    let filter_model = gtk4::FilterListModel::new(Some(store), Some(filter.clone()));
    let selection = gtk4::SingleSelection::new(Some(filter_model));

    // Re-run filter on every keystroke.
    {
        let filter = filter.clone();
        search.connect_search_changed(move |_| filter.changed(gtk4::FilterChange::Different));
    }

    let factory = gtk4::SignalListItemFactory::new();
    factory.connect_setup(|_factory, item| {
        let item: &gtk4::ListItem = item.downcast_ref().unwrap();
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let image = gtk4::Image::new();
        image.set_pixel_size(24);
        let label = gtk4::Label::new(None);
        label.set_xalign(0.0);
        label.set_hexpand(true);
        row.append(&image);
        row.append(&label);
        item.set_child(Some(&row));
    });
    factory.connect_bind(|_factory, item| {
        let item: &gtk4::ListItem = item.downcast_ref().unwrap();
        let (Some(app_obj), Some(row)) = (
            item.item().and_then(|o| o.downcast::<AppObject>().ok()),
            item.child().and_then(|c| c.downcast::<gtk4::Box>().ok()),
        ) else {
            return;
        };
        let (Some(image), Some(label)) = (
            row.first_child().and_then(|w| w.downcast::<gtk4::Image>().ok()),
            row.first_child()
                .and_then(|w| w.next_sibling())
                .and_then(|w| w.downcast::<gtk4::Label>().ok()),
        ) else {
            return;
        };
        let icon = gio::DesktopAppInfo::new(&app_obj.desktop()).and_then(|i| i.icon());
        match icon {
            Some(g) => image.set_from_gicon(&g),
            None => image.set_icon_name(Some("applications-other-symbolic")),
        }
        label.set_text(&app_obj.name());
    });

    let list_view = gtk4::ListView::new(Some(selection.clone()), Some(factory));
    list_view.set_single_click_activate(true);
    scrolled.set_child(Some(&list_view));
    vbox.append(&scrolled);

    popover.set_child(Some(&vbox));

    {
        let popover = popover.clone();
        list_view.connect_activate(move |lv, pos| {
            let Some(app_obj) = lv
                .model()
                .and_then(|m| m.item(pos))
                .and_then(|o| o.downcast::<AppObject>().ok())
            else {
                return;
            };
            popover.popdown();
            on_chosen(app_obj.desktop());
        });
    }

    // Enter in the search entry activates the first visible item.
    {
        let lv = list_view.clone();
        search.connect_activate(move |_| {
            if let Some(model) = lv.model()
                && model.n_items() > 0
            {
                let _ = lv.activate_action(
                    "list.activate-item",
                    Some(&glib::Variant::from(0u32)),
                );
            }
        });
    }

    // Down arrow in the search entry forwards focus to the list view so the standard
    // ListView keybindings (Up/Down/Enter) take over.
    {
        let lv = list_view.clone();
        let selection = selection.clone();
        let key_ctrl = gtk4::EventControllerKey::new();
        key_ctrl.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Down {
                if selection.n_items() > 0 {
                    selection.set_selected(0);
                }
                lv.grab_focus();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        search.add_controller(key_ctrl);
    }

    // Drop the popover (and its parent reference) when it closes so it doesn't leak.
    popover.connect_closed(|p| p.unparent());

    popover.popup();
    search.grab_focus();
}

// --- Add-exception dialog -------------------------------------------------------------

fn open_add_exception_dialog(wiring: &Wiring, category_mime: &str) {
    let prefix = category_mime
        .strip_suffix("/*")
        .map(|p| format!("{}/", p))
        .unwrap_or_default();

    let dialog = gtk4::Window::builder()
        .title("Add exception")
        .transient_for(&wiring.window)
        .modal(true)
        .default_width(360)
        .build();

    let vbox = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(8)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let prompt = gtk4::Label::builder()
        .label(format!(
            "Enter the exact MIME type to add as an exception under {}, or pick a file to detect it:",
            category_mime
        ))
        .xalign(0.0)
        .wrap(true)
        .build();

    let entry_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let entry = gtk4::Entry::builder()
        .placeholder_text(format!("{}example", prefix))
        .text(&prefix)
        .hexpand(true)
        .build();
    entry.set_position(prefix.len() as i32);
    let from_file = gtk4::Button::with_label("From file…");
    from_file.set_tooltip_text(Some("Pick a file and use its detected MIME type"));
    entry_row.append(&entry);
    entry_row.append(&from_file);

    let error_label = gtk4::Label::new(None);
    error_label.set_visible(false);
    error_label.set_xalign(0.0);
    error_label.add_css_class("error");

    let buttons = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    buttons.set_halign(gtk4::Align::End);
    let cancel = gtk4::Button::with_label("Cancel");
    let next = gtk4::Button::with_label("Choose app…");
    next.add_css_class("suggested-action");
    buttons.append(&cancel);
    buttons.append(&next);

    vbox.append(&prompt);
    vbox.append(&entry_row);
    vbox.append(&error_label);
    vbox.append(&buttons);
    dialog.set_child(Some(&vbox));

    {
        let dialog_parent = dialog.clone();
        let entry = entry.clone();
        let error_label = error_label.clone();
        from_file.connect_clicked(move |_| {
            let file_dialog = gtk4::FileDialog::builder()
                .title("Pick a file to detect its MIME type")
                .modal(true)
                .build();
            let entry = entry.clone();
            let error_label = error_label.clone();
            file_dialog.open(Some(&dialog_parent), gio::Cancellable::NONE, move |result| {
                let Ok(file) = result else { return }; // user cancelled
                let Some(path) = file.path() else { return };
                match handlr::detect_mime(&path) {
                    Ok(mime) => {
                        entry.set_text(&mime);
                        entry.set_position(mime.len() as i32);
                        error_label.set_visible(false);
                    }
                    Err(e) => {
                        error_label.set_text(&format!("Failed to detect MIME: {}", e));
                        error_label.set_visible(true);
                    }
                }
            });
        });
    }

    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| dialog.close());
    }

    let proceed = {
        let dialog = dialog.clone();
        let entry = entry.clone();
        let next_btn = next.clone();
        let wiring = wiring.clone();
        let error_label = error_label.clone();
        move || {
            let mime = entry.text().trim().to_string();
            if mime.is_empty() || mime.ends_with('*') || !mime.contains('/') {
                error_label.set_text("Enter a MIME like `video/mp4`");
                error_label.set_visible(true);
                return;
            }
            let prior: Vec<String> = wiring
                .state
                .borrow()
                .state()
                .defaults
                .iter()
                .find(|(m, _)| m == &mime)
                .map(|(_, h)| h.clone())
                .unwrap_or_default();
            let wiring_inner = wiring.clone();
            let dialog_inner = dialog.clone();
            let apps = system_apps(&wiring);
            show_app_picker(next_btn.upcast_ref::<gtk4::Widget>(), &apps, move |d| {
                apply_entry(&wiring_inner, undo::add_exception(&mime, &d, &prior));
                dialog_inner.close();
            });
        }
    };

    next.connect_clicked({
        let proceed = proceed.clone();
        move |_| proceed()
    });
    entry.connect_activate(move |_| proceed());

    dialog.present();
    entry.grab_focus();
}

// --- Settings dialog ------------------------------------------------------------------

fn build_settings_dialog(
    parent: &gtk4::ApplicationWindow,
    config: Rc<RefCell<crate::config::Config>>,
) -> gtk4::Window {
    let dlg = gtk4::Window::builder()
        .title("Settings")
        .transient_for(parent)
        .modal(false)
        .default_width(420)
        .build();

    dlg.set_titlebar(Some(&gtk4::HeaderBar::new()));

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    outer.set_margin_top(16);
    outer.set_margin_bottom(16);
    outer.set_margin_start(16);
    outer.set_margin_end(16);
    dlg.set_child(Some(&outer));

    // Error label (hidden unless save fails).
    let err_label = gtk4::Label::new(None);
    err_label.add_css_class("error");
    err_label.set_wrap(true);
    err_label.set_visible(false);

    // ── Selector section ─────────────────────────────────────────────────────
    outer.append(&section_label("Selector"));
    let selector_box = gtk4::ListBox::new();
    selector_box.set_selection_mode(gtk4::SelectionMode::None);
    selector_box.add_css_class("boxed-list");
    outer.append(&selector_box);

    // Row 1: enable_selector (switch)
    let enable_switch = gtk4::Switch::new();
    enable_switch.set_active(config.borrow().enable_selector);
    enable_switch.set_valign(gtk4::Align::Center);
    let enable_row = switch_row(
        "Enable selector",
        "Show picker when multiple handlers match",
        &enable_switch,
    );
    selector_box.append(&enable_row);

    // Row 2: selector command (entry, full-width below titles)
    let selector_entry = gtk4::Entry::new();
    selector_entry.set_text(&config.borrow().selector);
    selector_entry.set_sensitive(config.borrow().enable_selector);
    let selector_row = entry_row(
        "Selector command",
        "Shell command used as the interactive picker",
        &selector_entry,
    );
    selector_box.append(&selector_row);

    // ── Terminal section ──────────────────────────────────────────────────────
    outer.append(&section_label("Terminal"));
    let terminal_box = gtk4::ListBox::new();
    terminal_box.set_selection_mode(gtk4::SelectionMode::None);
    terminal_box.add_css_class("boxed-list");
    outer.append(&terminal_box);

    let term_entry = gtk4::Entry::new();
    term_entry.set_text(&config.borrow().term_exec_args);
    let term_row = entry_row(
        "Terminal exec args",
        "Flag passed to terminal emulator for terminal handlers (default: -e)",
        &term_entry,
    );
    terminal_box.append(&term_row);

    // ── MIME section ──────────────────────────────────────────────────────────
    outer.append(&section_label("MIME"));
    let mime_box = gtk4::ListBox::new();
    mime_box.set_selection_mode(gtk4::SelectionMode::None);
    mime_box.add_css_class("boxed-list");
    outer.append(&mime_box);

    let wildcard_switch = gtk4::Switch::new();
    wildcard_switch.set_active(config.borrow().expand_wildcards);
    wildcard_switch.set_valign(gtk4::Align::Center);
    let wildcard_row = switch_row(
        "Expand wildcards",
        "Expand video/* to individual MIME types when writing mimeapps.list",
        &wildcard_switch,
    );
    mime_box.append(&wildcard_row);

    outer.append(&err_label);

    // ── Live-apply callbacks ──────────────────────────────────────────────────

    // enable_selector toggle: update config, dim selector entry, save.
    {
        let config = config.clone();
        let selector_entry = selector_entry.clone();
        let err_label = err_label.clone();
        enable_switch.connect_active_notify(move |sw| {
            let active = sw.is_active();
            config.borrow_mut().enable_selector = active;
            selector_entry.set_sensitive(active);
            apply_config_save(&config, &err_label);
        });
    }

    // selector entry: save on every keystroke.
    {
        let config = config.clone();
        let err_label = err_label.clone();
        selector_entry.connect_changed(move |entry| {
            config.borrow_mut().selector = entry.text().to_string();
            apply_config_save(&config, &err_label);
        });
    }

    // term_exec_args entry.
    {
        let config = config.clone();
        let err_label = err_label.clone();
        term_entry.connect_changed(move |entry| {
            config.borrow_mut().term_exec_args = entry.text().to_string();
            apply_config_save(&config, &err_label);
        });
    }

    // expand_wildcards switch.
    {
        let config = config.clone();
        let err_label = err_label.clone();
        wildcard_switch.connect_active_notify(move |sw| {
            config.borrow_mut().expand_wildcards = sw.is_active();
            apply_config_save(&config, &err_label);
        });
    }

    let key_ctrl = gtk4::EventControllerKey::new();
    {
        let dlg = dlg.clone();
        key_ctrl.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                dlg.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }
    dlg.add_controller(key_ctrl);

    dlg
}

fn apply_config_save(
    config: &Rc<RefCell<crate::config::Config>>,
    err_label: &gtk4::Label,
) {
    match crate::config::save(&config.borrow()) {
        Ok(()) => err_label.set_visible(false),
        Err(e) => {
            err_label.set_text(&format!("Save failed: {e}"));
            err_label.set_visible(true);
        }
    }
}

fn section_label(text: &str) -> gtk4::Label {
    let label = gtk4::Label::new(Some(text));
    label.set_halign(gtk4::Align::Start);
    label.add_css_class("heading");
    label
}

fn switch_row(title: &str, subtitle: &str, switch: &gtk4::Switch) -> gtk4::ListBoxRow {
    let row_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    row_box.set_margin_top(8);
    row_box.set_margin_bottom(8);
    row_box.set_margin_start(8);
    row_box.set_margin_end(8);

    let text_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    text_box.set_hexpand(true);
    let title_label = gtk4::Label::new(Some(title));
    title_label.set_halign(gtk4::Align::Start);
    let sub_label = gtk4::Label::new(Some(subtitle));
    sub_label.set_halign(gtk4::Align::Start);
    sub_label.add_css_class("dim-label");
    sub_label.set_wrap(true);
    text_box.append(&title_label);
    text_box.append(&sub_label);

    row_box.append(&text_box);
    row_box.append(switch);

    let row = gtk4::ListBoxRow::new();
    row.set_activatable(false);
    row.set_child(Some(&row_box));
    row
}

fn entry_row(title: &str, subtitle: &str, entry: &gtk4::Entry) -> gtk4::ListBoxRow {
    let row_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    row_box.set_margin_top(8);
    row_box.set_margin_bottom(8);
    row_box.set_margin_start(8);
    row_box.set_margin_end(8);

    let title_label = gtk4::Label::new(Some(title));
    title_label.set_halign(gtk4::Align::Start);
    let sub_label = gtk4::Label::new(Some(subtitle));
    sub_label.set_halign(gtk4::Align::Start);
    sub_label.add_css_class("dim-label");
    sub_label.set_wrap(true);
    row_box.append(&title_label);
    row_box.append(&sub_label);
    entry.set_hexpand(true);
    row_box.append(entry);

    let row = gtk4::ListBoxRow::new();
    row.set_activatable(false);
    row.set_child(Some(&row_box));
    row
}

// --- Regex Handlers tab ---------------------------------------------------------------

pub(crate) fn build_regex_handlers_tab(
    config: Rc<RefCell<crate::config::Config>>,
    state: Rc<RefCell<AppState>>,
) -> (gtk4::Widget, impl Fn() + 'static) {
    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    // Toolbar row.
    let toolbar = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    toolbar.set_margin_top(6);
    toolbar.set_margin_bottom(6);
    toolbar.set_margin_start(10);
    toolbar.set_margin_end(10);

    let add_btn = gtk4::Button::with_label("Add handler");
    add_btn.set_halign(gtk4::Align::End);
    add_btn.set_hexpand(true);
    toolbar.append(&add_btn);
    outer.append(&toolbar);

    let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
    outer.append(&sep);

    // Scrolled card list.
    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_hscrollbar_policy(gtk4::PolicyType::Never);

    let cards_box = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    cards_box.set_margin_top(8);
    cards_box.set_margin_bottom(8);
    cards_box.set_margin_start(10);
    cards_box.set_margin_end(10);
    scrolled.set_child(Some(&cards_box));
    outer.append(&scrolled);

    // Collect apps from AppState.
    let apps: Vec<handlr::App> = state
        .borrow()
        .state()
        .system_apps
        .iter()
        .map(|a| handlr::App { desktop: a.desktop.clone(), name: a.name.clone() })
        .collect();

    rebuild_handler_cards(&cards_box, &config, &apps);

    // Add handler button: append a new empty card.
    {
        let cards_box = cards_box.clone();
        let config = config.clone();
        let apps = apps.clone();
        add_btn.connect_clicked(move |_| {
            let card = build_handler_card(None, config.clone(), cards_box.clone(), apps.clone());
            cards_box.append(&card);
        });
    }

    let css = gtk4::CssProvider::new();
    css.load_from_string(
        ".new-handler-card { border: 1px dashed alpha(currentColor, 0.3); border-radius: 12px; }",
    );
    #[allow(deprecated)]
    gtk4::style_context_add_provider_for_display(
        &gtk4::gdk::Display::default().expect("display"),
        &css,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let refresh = {
        let cards_box = cards_box.clone();
        let config = config.clone();
        let state = state.clone();
        move || {
            let apps: Vec<handlr::App> = state
                .borrow()
                .state()
                .system_apps
                .iter()
                .map(|a| handlr::App { desktop: a.desktop.clone(), name: a.name.clone() })
                .collect();
            rebuild_handler_cards(&cards_box, &config, &apps);
        }
    };

    (outer.upcast(), refresh)
}

fn rebuild_handler_cards(
    cards_box: &gtk4::Box,
    config: &Rc<RefCell<crate::config::Config>>,
    apps: &[handlr::App],
) {
    while let Some(child) = cards_box.first_child() {
        cards_box.remove(&child);
    }
    let n = config.borrow().handlers.len();
    for idx in 0..n {
        let card = build_handler_card(
            Some(idx),
            config.clone(),
            cards_box.clone(),
            apps.to_vec(),
        );
        cards_box.append(&card);
    }
}

fn build_handler_card(
    idx: Option<usize>,
    config: Rc<RefCell<crate::config::Config>>,
    cards_box: gtk4::Box,
    apps: Vec<handlr::App>,
) -> gtk4::Box {
    let is_new = idx.is_none();
    let handler = idx
        .and_then(|i| config.borrow().handlers.get(i).cloned())
        .unwrap_or_default();

    let card = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    card.add_css_class("card");
    if is_new {
        card.add_css_class("new-handler-card");
    }

    // ── Header row ────────────────────────────────────────────────────────────
    let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    header.set_margin_top(6);
    header.set_margin_bottom(6);
    header.set_margin_start(8);
    header.set_margin_end(8);

    let toggle_btn = gtk4::ToggleButton::new();
    toggle_btn.set_icon_name(if is_new { "pan-down-symbolic" } else { "pan-end-symbolic" });
    toggle_btn.add_css_class("flat");
    toggle_btn.set_active(is_new);

    let summary_label = gtk4::Label::new(None);
    summary_label.set_hexpand(true);
    summary_label.set_xalign(0.0);
    summary_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    // App icon (existing handlers only).
    let icon_image = gtk4::Image::new();
    icon_image.set_pixel_size(16);

    if is_new {
        summary_label.set_text("New handler");
        summary_label.add_css_class("dim-label");
    } else {
        let display_name = if handler.exec.is_empty() {
            "(no command)".to_string()
        } else {
            resolve_exec_name(&handler.exec, &apps)
        };
        let count = handler.regexes.len();
        summary_label.set_text(&format!(
            "{} — {} regex{}",
            display_name,
            count,
            if count == 1 { "" } else { "es" }
        ));

        // Look up icon from the matching DesktopAppInfo.
        if let Some(icon) = resolve_app_icon(&handler.exec, &apps) {
            icon_image.set_from_gicon(&icon);
        }
    }

    header.append(&toggle_btn);
    if !is_new {
        header.append(&icon_image);
    }
    header.append(&summary_label);

    // ▲/▼ reorder buttons (existing handlers only).
    if let Some(i) = idx {
        let n = config.borrow().handlers.len();

        let up_btn = gtk4::Button::from_icon_name("go-up-symbolic");
        up_btn.add_css_class("flat");
        up_btn.set_tooltip_text(Some("Move up"));
        up_btn.set_sensitive(i > 0);

        let down_btn = gtk4::Button::from_icon_name("go-down-symbolic");
        down_btn.add_css_class("flat");
        down_btn.set_tooltip_text(Some("Move down"));
        down_btn.set_sensitive(i + 1 < n);

        {
            let config = config.clone();
            let cards_box = cards_box.clone();
            let apps = apps.clone();
            up_btn.connect_clicked(move |_| {
                {
                    let mut cfg = config.borrow_mut();
                    cfg.handlers.swap(i, i - 1);
                }
                let _ = crate::config::save(&config.borrow());
                rebuild_handler_cards(&cards_box, &config, &apps);
            });
        }
        {
            let config = config.clone();
            let cards_box = cards_box.clone();
            let apps = apps.clone();
            down_btn.connect_clicked(move |_| {
                {
                    let mut cfg = config.borrow_mut();
                    if i + 1 < cfg.handlers.len() {
                        cfg.handlers.swap(i, i + 1);
                    } else {
                        return;
                    }
                }
                let _ = crate::config::save(&config.borrow());
                rebuild_handler_cards(&cards_box, &config, &apps);
            });
        }

        header.append(&up_btn);
        header.append(&down_btn);
    }

    card.append(&header);

    let hsep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
    card.append(&hsep);

    // ── Expandable body ───────────────────────────────────────────────────────
    let revealer = gtk4::Revealer::new();
    revealer.set_transition_type(gtk4::RevealerTransitionType::SlideDown);
    revealer.set_reveal_child(is_new);
    card.append(&revealer);

    {
        let revealer = revealer.clone();
        toggle_btn.connect_toggled(move |btn| {
            revealer.set_reveal_child(btn.is_active());
            btn.set_icon_name(if btn.is_active() {
                "pan-down-symbolic"
            } else {
                "pan-end-symbolic"
            });
        });
    }

    let body = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    body.set_margin_top(8);
    body.set_margin_bottom(8);
    body.set_margin_start(12);
    body.set_margin_end(12);
    revealer.set_child(Some(&body));

    // Regexes section.
    let regexes_label = gtk4::Label::new(Some("Regexes"));
    regexes_label.set_halign(gtk4::Align::Start);
    regexes_label.add_css_class("heading");
    body.append(&regexes_label);

    let regex_box = gtk4::Box::new(gtk4::Orientation::Vertical, 3);
    body.append(&regex_box);

    for regex_str in &handler.regexes {
        append_regex_row(&regex_box, regex_str);
    }

    let add_regex_btn = gtk4::Button::with_label("＋ Add regex");
    add_regex_btn.add_css_class("flat");
    add_regex_btn.set_halign(gtk4::Align::Start);
    {
        let regex_box = regex_box.clone();
        add_regex_btn.connect_clicked(move |_| {
            append_regex_row(&regex_box, "");
        });
    }
    body.append(&add_regex_btn);

    // Command section.
    let cmd_label = gtk4::Label::new(Some("Command"));
    cmd_label.set_halign(gtk4::Align::Start);
    cmd_label.add_css_class("heading");
    body.append(&cmd_label);

    let cmd_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let exec_entry = gtk4::Entry::new();
    exec_entry.set_text(&handler.exec);
    exec_entry.set_hexpand(true);
    exec_entry.set_placeholder_text(Some("e.g. mpv %f"));
    cmd_row.append(&exec_entry);

    let browse_btn = gtk4::Button::with_label("Browse apps…");
    {
        let exec_entry = exec_entry.clone();
        let apps = apps.clone();
        browse_btn.connect_clicked(move |btn| {
            show_app_picker(btn.upcast_ref::<gtk4::Widget>(), &apps, {
                let exec_entry = exec_entry.clone();
                move |desktop| {
                    let cmd = gio::DesktopAppInfo::new(&desktop)
                        .and_then(|info| info.commandline())
                        .map(|path| {
                            let s = path.to_string_lossy();
                            let mut parts = s.splitn(2, ' ');
                            let exe_path = parts.next().unwrap_or("");
                            let args = parts.next().unwrap_or("");
                            let basename = std::path::Path::new(exe_path)
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| exe_path.to_string());
                            if args.is_empty() {
                                basename
                            } else {
                                format!("{} {}", basename, args)
                            }
                        })
                        .unwrap_or_else(|| desktop.clone());
                    exec_entry.set_text(&cmd);
                }
            });
        });
    }
    cmd_row.append(&browse_btn);
    body.append(&cmd_row);

    // Terminal checkbox.
    let terminal_check = gtk4::CheckButton::with_label("Open in terminal");
    terminal_check.set_active(handler.terminal);
    body.append(&terminal_check);

    // Error label (hidden until needed).
    let err_label = gtk4::Label::new(None);
    err_label.add_css_class("error");
    err_label.set_halign(gtk4::Align::Start);
    err_label.set_wrap(true);
    err_label.set_visible(false);
    body.append(&err_label);

    // Action buttons.
    let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);

    let left_btn = if is_new {
        let b = gtk4::Button::with_label("Discard");
        {
            let card = card.clone();
            let cards_box = cards_box.clone();
            b.connect_clicked(move |_| {
                cards_box.remove(&card);
            });
        }
        b
    } else {
        let b = gtk4::Button::with_label("Delete");
        b.add_css_class("destructive-action");
        {
            let config = config.clone();
            let cards_box = cards_box.clone();
            let apps = apps.clone();
            let i = idx.unwrap();
            b.connect_clicked(move |_| {
                config.borrow_mut().handlers.remove(i);
                let _ = crate::config::save(&config.borrow());
                rebuild_handler_cards(&cards_box, &config, &apps);
            });
        }
        b
    };
    actions.append(&left_btn);

    let spacer = gtk4::Label::new(None);
    spacer.set_hexpand(true);
    actions.append(&spacer);

    let apply_btn = gtk4::Button::with_label("Apply");
    apply_btn.add_css_class("suggested-action");
    {
        let config = config.clone();
        let cards_box = cards_box.clone();
        let apps = apps.clone();
        let regex_box = regex_box.clone();
        let exec_entry = exec_entry.clone();
        let terminal_check = terminal_check.clone();
        let err_label = err_label.clone();
        let summary_label = summary_label.clone();
        let toggle_btn = toggle_btn.clone();

        apply_btn.connect_clicked(move |_| {
            let regexes = collect_regexes(&regex_box);

            // Check for invalid regex patterns in the entry widgets.
            let has_invalid_regex = {
                let mut invalid = false;
                let mut child = regex_box.first_child();
                while let Some(w) = child {
                    if let Some(row) = w.downcast_ref::<gtk4::Box>()
                        && let Some(entry) = row
                            .first_child()
                            .and_then(|c| c.downcast::<gtk4::Entry>().ok())
                    {
                        let t = entry.text();
                        if !t.is_empty() && regex::Regex::new(t.as_str()).is_err() {
                            invalid = true;
                            break;
                        }
                    }
                    child = w.next_sibling();
                }
                invalid
            };

            let exec = exec_entry.text().trim().to_string();

            let mut errors: Vec<&str> = Vec::new();
            if regexes.is_empty() {
                errors.push("At least one regex is required.");
            }
            if exec.is_empty() {
                errors.push("Command must not be empty.");
            }
            if has_invalid_regex {
                errors.push("One or more regexes are invalid.");
            }
            if !errors.is_empty() {
                err_label.set_text(&errors.join(" "));
                err_label.set_visible(true);
                return;
            }

            let new_handler = crate::config::RegexHandler {
                regexes,
                exec,
                terminal: terminal_check.is_active(),
            };

            if let Some(i) = idx {
                config.borrow_mut().handlers[i] = new_handler;
                let _ = crate::config::save(&config.borrow());
                let cfg = config.borrow();
                let h = &cfg.handlers[i];
                let display_name = if h.exec.is_empty() {
                    "(no command)".to_string()
                } else {
                    resolve_exec_name(&h.exec, &apps)
                };
                let count = h.regexes.len();
                summary_label.set_text(&format!(
                    "{} — {} regex{}",
                    display_name,
                    count,
                    if count == 1 { "" } else { "es" }
                ));
                toggle_btn.set_active(false);
            } else {
                config.borrow_mut().handlers.push(new_handler);
                let _ = crate::config::save(&config.borrow());
                rebuild_handler_cards(&cards_box, &config, &apps);
            }

            err_label.set_visible(false);
        });
    }
    actions.append(&apply_btn);
    body.append(&actions);

    card
}

fn append_regex_row(regex_box: &gtk4::Box, initial: &str) {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);

    let entry = gtk4::Entry::new();
    entry.set_text(initial);
    entry.set_hexpand(true);
    entry.set_placeholder_text(Some("regex pattern"));

    entry.connect_changed(|e| {
        let text = e.text();
        if text.is_empty() || regex::Regex::new(text.as_str()).is_ok() {
            e.remove_css_class("error");
        } else {
            e.add_css_class("error");
        }
    });

    let rm_btn = gtk4::Button::from_icon_name("edit-delete-symbolic");
    rm_btn.add_css_class("flat");
    rm_btn.set_tooltip_text(Some("Remove regex"));
    {
        let row = row.clone();
        let regex_box = regex_box.clone();
        rm_btn.connect_clicked(move |_| {
            regex_box.remove(&row);
        });
    }

    row.append(&entry);
    row.append(&rm_btn);
    regex_box.append(&row);
}

// Try to resolve a human-readable app name from an exec string by matching
// the executable basename against the commandline of known DesktopAppInfo entries.
fn resolve_exec_name(exec: &str, apps: &[handlr::App]) -> String {
    let exec_basename = exec
        .split_whitespace()
        .next()
        .and_then(|s| std::path::Path::new(s).file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| exec.to_string());

    for app in apps {
        if let Some(info) = gio::DesktopAppInfo::new(&app.desktop)
            && let Some(cmdline) = info.commandline()
        {
            let basename = cmdline
                .to_string_lossy()
                .split_whitespace()
                .next()
                .and_then(|s| std::path::Path::new(s).file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if basename == exec_basename {
                return app.name.clone();
            }
        }
    }

    exec_basename
}

fn resolve_app_icon(exec: &str, apps: &[handlr::App]) -> Option<gio::Icon> {
    let exec_basename = exec
        .split_whitespace()
        .next()
        .and_then(|s| std::path::Path::new(s).file_name())
        .map(|n| n.to_string_lossy().into_owned())?;

    for app in apps {
        if let Some(info) = gio::DesktopAppInfo::new(&app.desktop)
            && let Some(cmdline) = info.commandline()
        {
            let basename = cmdline
                .to_string_lossy()
                .split_whitespace()
                .next()
                .and_then(|s| std::path::Path::new(s).file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if basename == exec_basename {
                return info.icon();
            }
        }
    }
    None
}

fn collect_regexes(regex_box: &gtk4::Box) -> Vec<String> {
    let mut out = Vec::new();
    let mut child = regex_box.first_child();
    while let Some(w) = child {
        if let Some(row) = w.downcast_ref::<gtk4::Box>()
            && let Some(entry) = row
                .first_child()
                .and_then(|c| c.downcast::<gtk4::Entry>().ok())
        {
            let t = entry.text().trim().to_string();
            if !t.is_empty() {
                out.push(t);
            }
        }
        child = w.next_sibling();
    }
    out
}
