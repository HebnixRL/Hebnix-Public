# Linux build helpers for Hebnix (Windows uses ../Makefile.win).

.PHONY: release sidecar moon-bundle install

CARGO_BIN := hebnix

# The Workshop multiplayer helper (Go, see sidecar/hebnix-tc). Hebnix finds it
# in ../lib/hebnix next to its own binary.
sidecar:
	cd sidecar/hebnix-tc && CGO_ENABLED=0 go build -trimpath -ldflags="-s -w" -o hebnix-tc .

release: sidecar
	cargo build --release --bin hebnix

# Package the built release as a single .moon bundle (packaging/moon).
moon-bundle: release
	bash packaging/moon/build-moon-bundle.sh --binary target/release/$(CARGO_BIN)

# Install under $(PREFIX) (default ~/.local, no sudo): the hebnix command, the
# multiplayer helper next to it in lib/hebnix, the menu entry and the icon.
PREFIX ?= $(HOME)/.local

install: release
	install -Dm755 target/release/$(CARGO_BIN) "$(DESTDIR)$(PREFIX)/bin/hebnix"
	install -Dm755 sidecar/hebnix-tc/hebnix-tc "$(DESTDIR)$(PREFIX)/lib/hebnix/hebnix-tc"
	install -Dm644 packaging/hebnix.desktop "$(DESTDIR)$(PREFIX)/share/applications/hebnix.desktop"
	install -Dm644 crates/hebnix-app/assets/hebnix.png "$(DESTDIR)$(PREFIX)/share/icons/hicolor/256x256/apps/hebnix.png"
