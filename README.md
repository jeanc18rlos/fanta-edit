# Fanta Edit

> **Unofficial fork** of [Zed](https://github.com/zed-industries/zed). Not affiliated with or endorsed by Zed Industries.

Fanta Edit is a personal fork of Zed — a high-performance code editor built in Rust — customized for independent development and experimentation.

## Status

This fork is based on upstream Zed `main`. Initial rebranding is in place so Fanta Edit uses its own config/data directories and macOS bundle identity (see [FORK.md](./FORK.md)).

## Build (macOS)

**Prerequisite:** Accept the Xcode license (required for `git` and Rust builds on macOS):

```sh
sudo xcodebuild -license accept
```

Then follow [Building Zed for macOS](./docs/src/development/macos.md):

```sh
cargo run          # debug build
cargo run --release
```

## Upstream

- **Upstream:** https://github.com/zed-industries/zed
- **This fork:** https://github.com/jeanc18rlos/fanta-edit

See [FORK.md](./FORK.md) for syncing with upstream and the full rebranding checklist.

## Licensing

Same as upstream: primarily GPL-3.0-or-later, with Apache-2.0 components where marked. See upstream [README](https://github.com/zed-industries/zed) and `script/licenses/`.
