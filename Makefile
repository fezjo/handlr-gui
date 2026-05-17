PREFIX ?= /usr/local
BINDIR  = $(PREFIX)/bin
DATADIR = $(PREFIX)/share

.PHONY: build install install-user uninstall

build:
	cargo build --release

install: build
	install -Dm755 target/release/handlr-gui $(DESTDIR)$(BINDIR)/handlr-gui
	install -Dm644 handlr-gui.desktop $(DESTDIR)$(DATADIR)/applications/handlr-gui.desktop

install-user: build
	install -Dm755 target/release/handlr-gui $(HOME)/.local/bin/handlr-gui
	install -Dm644 handlr-gui.desktop $(HOME)/.local/share/applications/handlr-gui.desktop

uninstall:
	rm -f $(DESTDIR)$(BINDIR)/handlr-gui
	rm -f $(DESTDIR)$(DATADIR)/applications/handlr-gui.desktop
