# Fork guide — Fanta Edit

## Git remotes

After accepting the Xcode license (`sudo xcodebuild -license accept`), initialize git and connect remotes:

```sh
cd /Users/jeanrojas/fanta-edit

git init
git remote add origin https://github.com/jeanc18rlos/fanta-edit.git
git remote add upstream https://github.com/zed-industries/zed.git

git fetch upstream
git checkout -b main
git reset --soft upstream/main   # keep local fork changes staged
git add -A
git commit -m "Initial Fanta Edit fork with rebranding"
git push -u origin main
```

If you cloned via zip instead of git, the above creates a proper history linked to upstream.

## Syncing with upstream Zed

```sh
git fetch upstream
git merge upstream/main
# resolve conflicts, then:
cargo test --workspace   # optional but recommended
git push origin main
```

## Rebranding checklist

Already done in this fork:

| Item | Location | Value |
|------|----------|-------|
| App data/config name | `crates/paths/src/paths.rs` → `APP_NAME` | `FantaEdit` |
| macOS bundle IDs & display names | `crates/zed/Cargo.toml` → `[package.metadata.bundle-*]` | `dev.fantaedit.*` / `Fanta Edit` |

Still to customize as you build out Fanta Edit:

- [ ] **App icons** — replace `crates/zed/resources/app-icon*.png`
- [ ] **CLI binary name** — `crates/cli` (currently still `cli` / `zed` in places)
- [ ] **Window title / about dialog** — search for `"Zed"` in `crates/zed/src/`
- [ ] **Default settings** — `assets/settings/default.json` (`server_url`, provider IDs)
- [ ] **Deep links / URL schemes** — currently `fantaedit://` in bundle metadata
- [ ] **Project folder** — still `.zed/` in upstream; change in `crates/paths/src/paths.rs` if you want `.fanta-edit/`
- [ ] **Remote server dirs** — `.zed_server` paths in `paths.rs` if you use remote dev
- [ ] **Documentation & marketing** — `docs/`, GitHub repo description
- [ ] **Disable Zed cloud** — point `server_url` away from `zed.dev` or gate features in settings

See [`FANTA.md`](./FANTA.md) for the broader adoption plan, service-safety
baseline, release resources, and first feature substrate.

Zed explicitly documents fork branding in `crates/paths/src/paths.rs`:

```rust
/// Forks should change this to avoid colliding with Zed's user data.
pub const APP_NAME: &str = "FantaEdit";
```

Config paths become `~/.config/fantaedit`, `~/Library/Application Support/FantaEdit`, etc.

## Legal note

GPL-3.0 requires source availability for distributed binaries. If you ship Fanta Edit publicly, keep license notices and document your changes. Do not use Zed's trademarks or imply official affiliation.
