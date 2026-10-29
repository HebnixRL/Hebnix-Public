# `.moon` bundles https://moonpkg.github.io/cli / https://github.com/moonpkg/cli

A `.moon` bundle is one file that carries a whole app: the program (with its
multiplayer helper), its
manifest, its menu entry and its icon. A user unpacks nothing, signs into
nothing and adds no repository:

```
moon install hebnix.moon
```

It is a plain `tar.gz`, so `tar xzf hebnix.moon` still works and the file can be
inspected, diffed or checksummed like any other archive.

## Building one

```
packaging/moon/build-moon-bundle.sh  # from make release output, run in hebnix_rs
packaging/moon/build-moon-bundle.sh --from path/to/hebnix-2.2.4-linux-x86_64.tar.gz
```

Or through the Makefile:

```
make moon-bundle   # run from hebnix_rs
```

## What ends up inside

```
hebnix-2.2.4.moon           # (tar.gz)
├── hebnix.manifest         # moon's own key=value manifest, paths relative
├── app/
│   ├── bin/hebnix          # the binary
│   ├── lib/hebnix/hebnix-tc  # the Workshop multiplayer helper
│   └── share/…             # README and LICENSE
├── desktop/hebnix.desktop  # the menu entry
└── icon/hebnix.png         # the icon
```

The manifest is the same format moon writes for every install, with the
paths the bundle can have:

```
dir=app
main=app/bin/hebnix
version=2.2.4
desktop=desktop/hebnix.desktop
link=hebnix
to=app/bin/hebnix
cmd=hebnix
```
