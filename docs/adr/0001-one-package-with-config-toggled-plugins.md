# One package, plugins switched on in config

specmint ships as a single npm package, `specmint`, with every plugin built into one native addon. Users switch plugins on or off in the config (`plugins: { ts: true, faker: { … } }`) and never install a plugin package. kubb, which specmint ports, does the opposite: each plugin is its own npm package. We chose one package because installing nothing extra is simpler for users, and because the plugins are Rust code: separate native addons per plugin would mean one binary per plugin per platform, each carrying its own copy of the core.

## Consequences

- Plugins are written in Rust by the specmint maintainers. There is no API for third-party JavaScript plugins in v1.
- Splitting the Rust code into a crate per plugin is fine and planned; that split is internal and does not change what users install.
