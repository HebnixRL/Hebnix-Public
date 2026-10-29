# Linux build helpers for Hebnix (Windows uses ../Makefile.win).

.PHONY: release sidecar moon-bundle

CARGO_BIN := hebnix-app

# The Workshop multiplayer helper (Go, see sidecar/hebnix-tc). Hebnix finds it
# in ../lib/hebnix next to its own binary.
sidecar:
	cd sidecar/hebnix-tc && CGO_ENABLED=0 go build -trimpath -ldflags="-s -w" -o hebnix-tc .

release: sidecar
	cargo build --release

# Package the built release as a single .moon bundle (packaging/moon).
moon-bundle: release
	bash packaging/moon/build-moon-bundle.sh --binary target/release/$(CARGO_BIN)
