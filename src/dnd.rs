//! Drop target for the main window. Accepts a local file, hands the resolved path to a
//! caller-supplied callback. Visual hover feedback via the `.drop-active` CSS class.

use gtk4::gdk;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use std::path::PathBuf;

const HOVER_CSS: &str = ".drop-active { box-shadow: inset 0 0 0 3px @accent_bg_color; }";

pub(crate) fn install(
    window: &gtk4::ApplicationWindow,
    on_drop: impl Fn(PathBuf) + 'static,
) {
    install_hover_css(window);

    // Accept both FileList (Wayland-friendly: maps from text/uri-list, the standard
    // file-manager drop format) and bare GFile. GTK negotiates whichever the source offers.
    let target = gtk4::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    target.set_types(&[gdk::FileList::static_type(), gio::File::static_type()]);

    {
        let window = window.clone();
        target.connect_enter(move |_, _, _| {
            window.add_css_class("drop-active");
            gdk::DragAction::COPY
        });
    }
    {
        let window = window.clone();
        target.connect_leave(move |_| {
            window.remove_css_class("drop-active");
        });
    }

    {
        let window = window.clone();
        target.connect_drop(move |_, value, _, _| {
            window.remove_css_class("drop-active");
            // Spec §7: process only the first file. We only registered GFile as the
            // accepted type, so GTK hands us a single GFile here.
            let path = extract_first_path(value);
            match path {
                Some(p) => {
                    on_drop(p);
                    true
                }
                None => false,
            }
        });
    }

    window.add_controller(target);
}

fn extract_first_path(value: &glib::Value) -> Option<PathBuf> {
    if let Ok(file_list) = value.get::<gdk::FileList>() {
        return file_list.files().into_iter().next().and_then(|f| f.path());
    }
    if let Ok(file) = value.get::<gio::File>() {
        return file.path();
    }
    None
}

// Inline single-rule provider — no `data/style.css` for one selector.
fn install_hover_css(window: &gtk4::ApplicationWindow) {
    let provider = gtk4::CssProvider::new();
    provider.load_from_string(HOVER_CSS);
    let display = gtk4::prelude::WidgetExt::display(window);
    #[allow(deprecated)]
    gtk4::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}
