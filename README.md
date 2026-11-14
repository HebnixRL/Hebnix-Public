<div align="center">

<img src="https://hebnix.com/media/hero-desktop.png" />

Hebnix serves as an intelligent gateway to Rocket League's Stats API, Configuration Files, game files and plugins.\
The purpose is to consolidate several Rocket League Quality of Life tools into 1 platform that is simple to work with, user friendly and feature rich.

[Website](https://hebnix.com) | [Plugins](https://hebnix.com/plugins) | [Download](https://hebnix.com/download) | [Developer Documentation](https://docs.hebnix.com) | [Discord](https://discord.gg/yr6xXb5wQd)

<a href="https://moonpkg.github.io/cli/app/?type=github&repo=HebnixRL/Hebnix-Public"><img src="https://github.com/moonpkg/cli/blob/main/docs/assets/install-btn.png?raw=true" width="256" alt="Install with moon" /></a>
</div>

## Access to our Rank API data

2.2.0 will include an additional method of authentication in accessing our Rank API. Although Hebnix will compile without the key, you will require a key to access it.
You can get a key by requesting one on our [Discord Server](https://discord.gg/yr6xXb5wQd) and it will be DMed to you. Open a request in the requests channel.

## Build

Install Rust with the MSVC toolchain, then run:

    cd hebnix_rs
    cargo build --release

RLAPI uses the existing captured Rocket League WebSocket session; no Go build is required.

Set `$env:HEBNIX_BASE_DIR` to use a different data directory while developing.

There is also a more user friendly build.bat file.

## Packaging

    cd hebnix_rs
    ./package.ps1

## Plugins and themes

See the [plugin examples](hebnix_rs/examples/plugins), [theme examples](hebnix_rs/examples/themes), and [documentation](https://hebnix.com/docs).

Plugins can access WebSocket match history through `hebnix.rlapi.enabled` and `hebnix.rlapi.matchhistory()`. See [the RLAPI plugin API](docs/rlapi-plugin-api.md).

## Logs

`hebnix.log` and `crash.txt` are written beside the executable. Set `RUST_LOG=debug` for additional logging.

## Platforms

- **Windows** - Build from `hebnix_rs/` directory
- **Linux** - Build from `hebnix_rs/` directory (supports Hyprland, Wayland compositors, X11)

## Quick Start

### Windows
```batch
build.bat
# or
cd hebnix_rs && cargo build --release
```

### Linux
```bash
cd hebnix_rs
cargo build --release
```

## Linux-Specific Features

- Wayland support (wlr-layer-shell for overlays)
- X11 support (i3, Cinnamon, XFCE, MATE)
- evdev input handling
- Linux path handling (case-sensitive filesystems)
- Heroic/Steam Proton launcher integration
- AppImage packaging

## License

[LICENSE](LICENSE.md)
