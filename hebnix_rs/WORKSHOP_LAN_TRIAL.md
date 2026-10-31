# Workshop LAN trial

This tests custom-map LAN joining without replacing Utopia Retro. It is an
experiment, not a confirmed Error 40 fix. Both players should run the new Hebnix
build so the host shares the original package filename as well as its title.

The joining player enables the tests under Workshop > In-game menu
(experimental) > LAN custom map test. Rocket League must be closed when changing
test mode. Enable the in-game Workshop menu first.

1. Select **1. Test map titles**, then start Rocket League and connect both
   players to Hebnix multiplayer. Host the map from the game's Workshop menu.
   Keep Hebnix on its Workshop tab and allow about 15 seconds for peer metadata
   to arrive. Reopen/refresh **Join LAN Match**. Its map column should show the
   Workshop title, or the host's filename if no title metadata exists.
2. On the joining player, download the hosting player's map to the Workshop
   folder through Hebnix if offered. An older install with a different filename
   will be offered again. Close Rocket League and select **2. Test custom map
   joining**, restart, and join through **Join LAN Match**. There is no need to
   load freeplay or select Utopia Retro first.
3. Record whether the title appears and whether joining succeeds independently.
   The joining client's log now contains `HBX LAN join preparation:` followed by
   its selected server ID. If a matching local file is available, it also logs
   `HBX LAN preload result:`. `None` indicates the experimental loader did not
   resolve the world object. A resolved object followed by Error 40 means this
   loading step was insufficient; it does not prove network package registration.
4. Close Rocket League and select **Disable test** to restore the two original
   LAN functions while preserving other package patches.

The title bridge uses the sidecar's IPv4 aliases and port 7777. It writes bounded
peer titles and locally resolved map paths to `TAGame/HbxLanTrial`. It does not
execute peer-supplied commands or paths. Test state and original function bytes
are saved in `TAGame/CookedPCConsole/Backups/WorkshopLanTrial.json`.

The preload experiment calls `DynamicLoadObject` for the local package's
`TheWorld` before the normal LAN join. Whether this engine build accepts the
absolute object path and retains/registers the package for network travel needs
the two-client test. It does not reproduce Rocket Plugin's unpublished native
preloader. The separate Utopia Retro fallback still has an unverified internal
package identity risk.

The second build searches the selected Workshop folder, Hebnix's fallback folder,
and mods by original package filename, so a local catalog ID may differ from the
host's ID. Multiple matches outside the exact-ID folder are treated as ambiguous.
For older hosts without a package filename, the last matching join error supplies
the package name for the next attempt. A title without a preload-result line
means the package loading attempt has not been established.

Developer tool: `tools/avatar-proof/target/release/workshop-lan-proof.exe` supports
staging on a package copy, switching live modes with the game closed, and writing
a manual bridge entry. Run it without arguments for usage. Local validation
covers bridge updates, original filename selection, package save/reload, and
restoring all exports; it cannot establish that LAN joining succeeds.

Changed existing functions:

- `fetch_peer_info` on Windows and Linux: accept and validate the package stem.
- `HostedMap::new`, `HostedMap::current`, `describe`: read the loader's title and
  announce the original filename separately.
- `MapManager::install_to_workshop_folder`: validate the supplied package name.
- `WorkshopState::start_relay`, its caller `render_multiplayer`,
  `render_join_prompt`, `install_peer_map`, and `render_player_maps`: carry the
  loader metadata, publish local bridge files, and preserve the host's filename.
- `InGameMenuState::render`: expose the two test modes and restoration.
- `workshop_menu::restore`, `workshop_menu::reapply`: restore the trial before
  removing Workshop hooks, and retain it when another feature rebuilds the package.

New functions include `peer_workshop_file`, `valid_package_stem`, and the
`workshop_lan_trial` module's mode switching, bridge writing, and bytecode builders.
