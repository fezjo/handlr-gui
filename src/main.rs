//! App entry: handlr presence check, state load, window build, drop-target wiring.
//! Startup failures surface as a modal GtkAlertDialog and quit on dismiss (spec §5).

mod config;
mod dnd;
mod handlr;
mod model;
mod ui;
mod undo;

use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

fn main() -> glib::ExitCode {
    let app = gtk4::Application::builder()
        .application_id("com.github.handlrgui")
        .build();
    app.connect_activate(on_activate);
    app.run()
}

fn on_activate(app: &gtk4::Application) {
    // Present existing window if the app is already running.
    if let Some(w) = app.windows().first() {
        w.present();
        return;
    }

    // 1. handlr on PATH?
    if let Err(e) = handlr::check_present() {
        show_startup_error(
            app,
            &format!(
                "handlr CLI not found on PATH.\n\n{e}\n\nInstall handlr-regex and restart."
            ),
        );
        return;
    }

    // 2. Initial state load.
    let state = match model::AppState::load() {
        Ok(s) => s,
        Err(e) => {
            show_startup_error(app, &format!("handlr is installed but failed:\n\n{e}"));
            return;
        }
    };
    let state = Rc::new(RefCell::new(state));

    // 3. Load config (non-fatal: use defaults if loading fails).
    let config = Rc::new(RefCell::new(config::load().unwrap_or_default()));

    // 4. Build window.
    let (window, tree) = ui::build_window(app, state.clone(), config);

    // 5. Install drop target. On drop: resolve mime, then:
    //    - If an exact entry (exception or category) already exists → scroll to it.
    //    - Otherwise → inject a pending empty exception so the user can see the MIME in
    //      context and optionally click "Set handler" to configure it.
    dnd::install(&window, {
        let state = state.clone();
        move |path| match handlr::detect_mime(&path) {
            Ok(mime) => {
                // Try scrolling to an existing row first (exact or wildcard match).
                // If not found, inject a pending exception and retry so the user can
                // see the MIME in context and click "Set handler". If still not found
                // (no category at all), show the banner.
                if !ui::scroll_to_mime(&tree, &mime) {
                    state.borrow_mut().add_pending_exception(mime.clone());
                    ui::rebuild_tree(&tree, &state);
                    if !ui::scroll_to_mime(&tree, &mime) {
                        ui::show_banner(
                            &tree,
                            &format!("MIME {mime} has no matching category."),
                        );
                    }
                }
            }
            Err(e) => ui::show_banner(&tree, &format!("MIME detect failed: {e}")),
        }
    });

    window.present();
}

fn show_startup_error(app: &gtk4::Application, msg: &str) {
    let dialog = gtk4::AlertDialog::builder()
        .message("handlr-gui startup error")
        .detail(msg)
        .modal(true)
        .buttons(["Quit"])
        .build();
    let app = app.clone();
    dialog.choose(
        None::<&gtk4::Window>,
        None::<&gio::Cancellable>,
        move |_| app.quit(),
    );
}
