//! App entry: handlr presence check, state load, window build, drop-target wiring.
//! Startup failures surface as a modal GtkAlertDialog and quit on dismiss (spec §5).

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

    // 3. Build window.
    let (window, tree) = ui::build_window(app, state);

    // 4. Install drop target. On drop: resolve mime, scroll to row. Errors and "not in
    //    any category" both surface in the info bar (spec §7 step 4).
    dnd::install(&window, move |path| match handlr::detect_mime(&path) {
        Ok(mime) => {
            if !ui::scroll_to_mime(&tree, &mime) {
                ui::show_banner(
                    &tree,
                    &format!("MIME {mime} not found in any configured category."),
                );
            }
        }
        Err(e) => ui::show_banner(&tree, &format!("MIME detect failed: {e}")),
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
