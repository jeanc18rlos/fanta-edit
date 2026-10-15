use std::{fs, io};

use build_support::{
    binaries_config,
    cargo::{self, Target},
    features, platform, skia, skia_bindgen,
};

mod build_support;

fn main() -> Result<(), io::Error> {
    if env::is_docs_rs_build() {
        println!("DETECTED DOCS_RS BUILD");
        return fake_bindings();
    }

    let skia_debug = env::is_skia_debug();
    let cargo_target = cargo::target();

    let features = {
        let mut features = features::Features::from_cargo_env();
        if features.backend_without_engine() {
            return Err(io::Error::other(
                "the `vulkan` and `metal` features require at least one rendering engine: `ganesh` or `graphite`",
            ));
        }
        let missing_dependencies = features.missing_dependencies();
        if !missing_dependencies.is_empty() {
            return Err(io::Error::other(format!(
                "Missing dependent features: {missing_dependencies}"
            )));
        }

        let redundant_features = platform::redundant_features(&features, &cargo_target);
        if !redundant_features.is_empty() {
            #[cfg(feature = "binary-cache")]
            if build_support::binary_cache::should_export().is_some() {
                return Err(io::Error::other(format!(
                    "Can't produce binaries with redundant features: {redundant_features}"
                )));
            }
            cargo::warning(format!(
                "Redundant features: {redundant_features}. Disabled for the build and binary download."
            ));
            features -= redundant_features;
            cargo::warning(format!("Final features: {features}"));
        }

        features
    };

    if features.graphite() {
        if cfg!(feature = "no-compile") {
            return Err(io::Error::other(
                "Fanta's Graphite image provider requires compiling its local native shim; `no-compile` is incompatible",
            ));
        }
        if cargo::env_var("FORCE_SKIA_BINARIES_DOWNLOAD").is_some() {
            return Err(io::Error::other(
                "Fanta's Graphite image provider is absent from upstream prebuilt binaries; FORCE_SKIA_BINARIES_DOWNLOAD is incompatible",
            ));
        }
        println!("FANTA GRAPHITE IMAGE PROVIDER: UPSTREAM PREBUILT BINARIES DISABLED");
    }

    let binaries_config =
        binaries_config::BinariesConfiguration::from_features(&features, skia_debug);

    //
    // skip attempting to download?
    //
    if let Some(source_dir) = env::source_dir() {
        if let Some(search_path) = env::skia_lib_search_path() {
            println!("STARTING BIND AGAINST SYSTEM SKIA");

            binaries_config.import(&search_path, false).unwrap();

            let definitions = skia_bindgen::definitions::from_env();
            generate_bindings(
                &features,
                definitions,
                &binaries_config,
                &source_dir,
                cargo_target,
                None,
            );
        } else {
            if cfg!(feature = "no-compile") {
                panic!("Refusing to offline-build skia with no-compile feature");
            }

            println!("STARTING OFFLINE BUILD");

            let final_build_configuration = build_from_source(
                features.clone(),
                &binaries_config,
                &source_dir,
                skia_debug,
                true,
            );
            let definitions = skia_bindgen::definitions::from_ninja_features(
                &features,
                final_build_configuration.use_system_libraries,
                &binaries_config.output_directory,
            );
            generate_bindings(
                &features,
                definitions,
                &binaries_config,
                &source_dir,
                final_build_configuration.target,
                final_build_configuration
                    .sysroot
                    .as_ref()
                    .map(AsRef::as_ref),
            );
        }
    } else {
        //
        // is the download of prebuilt binaries possible?
        //

        #[allow(unused_variables)]
        let build_skia = true;

        // Upstream cached shim archives omit the local image provider, so Graphite builds only
        // reuse binaries from the Fanta-keyed local cache.
        #[cfg(feature = "binary-cache")]
        let fanta_cache_entry = if features.graphite() {
            fanta_binary_cache::Entry::from_env(&binaries_config)
        } else {
            None
        };

        #[cfg(feature = "binary-cache")]
        let build_skia = match &fanta_cache_entry {
            Some(entry) => !entry.install(&binaries_config),
            None => {
                features.graphite()
                    || build_support::binary_cache::try_prepare_download(&binaries_config)
            }
        };

        //
        // full build?
        //

        if build_skia {
            if cfg!(feature = "no-compile") {
                panic!("Refusing to full-build skia with no-compile feature");
            }

            println!("STARTING A FULL BUILD");
            println!("HOST: {}", cargo::host());

            let source_dir = std::env::current_dir().unwrap().join("skia");
            let final_build_configuration = build_from_source(
                features.clone(),
                &binaries_config,
                &source_dir,
                skia_debug,
                false,
            );
            let definitions = skia_bindgen::definitions::from_ninja_features(
                &features,
                final_build_configuration.use_system_libraries,
                &binaries_config.output_directory,
            );
            generate_bindings(
                &features,
                definitions,
                &binaries_config,
                &source_dir,
                final_build_configuration.target,
                final_build_configuration
                    .sysroot
                    .as_ref()
                    .map(AsRef::as_ref),
            );

            #[cfg(feature = "binary-cache")]
            if let Some(entry) = &fanta_cache_entry {
                entry.store(&binaries_config);
            }
        }
    };

    binaries_config.commit_to_cargo();

    #[cfg(feature = "binary-cache")]
    if let Some(staging_directory) = build_support::binary_cache::should_export() {
        build_support::binary_cache::publish(&binaries_config, &staging_directory);
    }

    Ok(())
}

fn build_from_source(
    features: features::Features,
    binaries_config: &binaries_config::BinariesConfiguration,
    skia_source_dir: &std::path::Path,
    skia_debug: bool,
    offline: bool,
) -> skia::FinalBuildConfiguration {
    let build_config = skia::BuildConfiguration::from_features(features, skia_debug);
    let final_configuration = skia::FinalBuildConfiguration::from_build_configuration(
        &build_config,
        skia::env::use_system_libraries(),
        skia_source_dir,
    );

    skia::build(
        &final_configuration,
        binaries_config,
        skia::env::ninja_command(),
        skia::env::gn_command(),
        offline,
    );
    if final_configuration.target.is_emscripten() {
        binaries_config.copy_emscripten_ninja_archives_for_linking();
    }

    final_configuration
}

fn generate_bindings(
    features: &features::Features,
    definitions: Vec<skia_bindgen::Definition>,
    binaries_config: &binaries_config::BinariesConfiguration,
    skia_source_dir: &std::path::Path,
    target: Target,
    sysroot: Option<&str>,
) {
    // Emit the ninja definitions, to help debug build consistency.
    skia_bindgen::definitions::save_definitions(&definitions, &binaries_config.output_directory)
        .expect("failed to write Skia defines");

    let bindings_config = skia_bindgen::Configuration::new(features, definitions, skia_source_dir);
    skia_bindgen::generate_bindings(
        &bindings_config,
        &binaries_config.output_directory,
        target,
        sysroot,
    );
}

/// On docs.rs, rustdoc runs inside a container with no networking, so copy a pre-generated
/// `bindings.rs` file.
fn fake_bindings() -> Result<(), io::Error> {
    let source = std::path::Path::new("bindings_docs.rs");
    if !source.exists() {
        return Err(io::Error::other(
            "bindings_docs.rs is missing from the published skia-bindings \
             tarball. The release Makefile must run `make publish-bindings-docs` \
             (not `make publish-bindings`) so the documentation bindings end up \
             in the tarball. Without this file, docs.rs builds of skia-safe and \
             every downstream crate fail here. See \
             https://github.com/rust-skia/rust-skia/issues/720",
        ));
    }
    println!("COPYING bindings_docs.rs to OUT_DIR/skia/bindings.rs");
    let bindings_parent = cargo::output_directory().join(binaries_config::SKIA_OUTPUT_DIR);
    fs::create_dir_all(&bindings_parent)?;
    fs::copy(source, bindings_parent.join("bindings.rs")).map(|_| ())
}

/// Environment variables used by this build script.
mod env {
    use crate::build_support::cargo;
    use std::path::PathBuf;

    /// The path to the Skia source directory.
    pub fn source_dir() -> Option<PathBuf> {
        cargo::env_var("SKIA_SOURCE_DIR").map(PathBuf::from)
    }

    /// The path to where a pre-built Skia library can be found.
    pub fn skia_lib_search_path() -> Option<PathBuf> {
        cargo::env_var("SKIA_LIBRARY_SEARCH_PATH").map(PathBuf::from)
    }

    pub fn is_skia_debug() -> bool {
        matches!(cargo::env_var("SKIA_DEBUG"), Some(v) if v != "0")
    }

    pub fn is_docs_rs_build() -> bool {
        matches!(cargo::env_var("DOCS_RS"), Some(v) if v != "0")
    }
}

/// A local cache of Graphite builds, which upstream prebuilt binaries cannot provide.
///
/// Entries live in `FANTA_SKIA_BINARIES_DIR/<key>` and hold the files of an upstream binary
/// archive. The key extends the upstream key with a hash of every local input that changes the
/// native libraries or `bindings.rs`: the shim sources, the bindgen and GN configuration, the
/// locked bindgen and cc versions, the compiler environment and the toolchain. Editing
/// `src/graphite.cpp` therefore selects a new entry instead of reusing a stale shim.
#[cfg(feature = "binary-cache")]
mod fanta_binary_cache {
    use std::{
        collections::BTreeMap,
        fs, io,
        path::{Path, PathBuf},
        process::Command,
    };

    use crate::build_support::{binaries_config::BinariesConfiguration, cargo};

    /// Package files that feed the native build or bindgen. `src/*.rs` only feeds the Rust crate.
    const PACKAGE_INPUTS: &[&str] = &[
        "Cargo.toml",
        "build.rs",
        "build_support.rs",
        "build_support",
        "src",
    ];

    /// Environment that changes the Skia, shim or bindgen output and is not part of the upstream
    /// key. Compiler caches (`RUSTC_WRAPPER`, GN's `cc_wrapper`) are left out because they never
    /// change the output.
    const ENVIRONMENT_INPUTS: &[&str] = &[
        "OPT_LEVEL",
        "DEBUG",
        "CLANGCC",
        "CLANGCXX",
        "CRATE_CC_NO_DEFAULTS",
        "SKIA_USE_SYSTEM_LIBRARIES",
        "DEP_JPEG_INCLUDE",
        "SDKROOT",
        "SDKTARGETSYSROOT",
        "DEVELOPER_DIR",
        "MACOSX_DEPLOYMENT_TARGET",
        "LIBCLANG_PATH",
        "LIBCLANG_STATIC_PATH",
        "CLANG_PATH",
    ];

    /// cc-rs and bindgen also read target-qualified spellings of these variables.
    const TARGETED_ENVIRONMENT_INPUTS: &[&str] = &[
        "CC",
        "CXX",
        "CFLAGS",
        "CXXFLAGS",
        "BINDGEN_EXTRA_CLANG_ARGS",
    ];

    /// Build dependencies whose versions change `bindings.rs` or the shim objects. Cargo rebuilds
    /// and reruns this script when their locked versions change, so the key only needs to read
    /// those versions.
    const BUILD_DEPENDENCY_INPUTS: &[&str] = &["bindgen", "cc", "clang-sys"];

    /// The `-C`/`-Z` rustflags that cc-rs turns into C++ flags for the shim (`cc/src/flags.rs`).
    /// No other rustflag reaches the native outputs. Hashing all of them would split the cache by
    /// location: cargo joins `build.rustflags` from every ancestor `.cargo/config.toml`, so a
    /// worktree nested in another checkout sees the flags twice.
    const INHERITED_RUSTFLAGS: &[&str] = &[
        "branch-protection",
        "code-model",
        "control-flow-guard",
        "dwarf-version",
        "embed-bitcode",
        "force-frame-pointers",
        "linker-plugin-lto",
        "lto",
        "no-redzone",
        "no-vectorize-loops",
        "no-vectorize-slp",
        "profile-generate",
        "profile-use",
        "relocation-model",
        "soft-float",
        "stack-protector",
    ];

    pub struct Entry {
        directory: PathBuf,
    }

    impl Entry {
        /// Returns `None` when `FANTA_SKIA_BINARIES_DIR` is unset or the key cannot be computed.
        pub fn from_env(binaries_config: &BinariesConfiguration) -> Option<Self> {
            let cache_directory = PathBuf::from(cargo::env_var("FANTA_SKIA_BINARIES_DIR")?);
            if !cache_directory.is_absolute() {
                cargo::warning(format!(
                    "Ignoring FANTA_SKIA_BINARIES_DIR, it must be an absolute path: {}",
                    cache_directory.display()
                ));
                return None;
            }
            match key(binaries_config) {
                Ok(key) => Some(Self {
                    directory: cache_directory.join(key),
                }),
                Err(error) => {
                    cargo::warning(format!("Fanta Skia binary cache disabled: {error}"));
                    None
                }
            }
        }

        /// Copies the cached build into the output directory. Returns `false` when Skia must be
        /// built from source instead.
        pub fn install(&self, binaries_config: &BinariesConfiguration) -> bool {
            if cargo::env_var("FORCE_SKIA_BUILD").is_some() {
                return false;
            }
            if !self.directory.is_dir() {
                println!("FANTA SKIA BINARY CACHE MISS: {}", self.directory.display());
                return false;
            }
            match binaries_config.import(&self.directory, true) {
                Ok(()) => {
                    println!("FANTA SKIA BINARY CACHE HIT: {}", self.directory.display());
                    true
                }
                Err(error) => {
                    cargo::warning(format!(
                        "Building Skia from source, the Fanta Skia binary cache entry {} is unreadable: {error}",
                        self.directory.display()
                    ));
                    false
                }
            }
        }

        /// A failed store only costs later builds a rebuild, so it is reported as a warning.
        pub fn store(&self, binaries_config: &BinariesConfiguration) {
            match self.try_store(binaries_config) {
                Ok(()) => println!(
                    "FANTA SKIA BINARY CACHE STORED: {}",
                    self.directory.display()
                ),
                Err(error) => cargo::warning(format!(
                    "Could not store the Fanta Skia binary cache entry {}: {error}",
                    self.directory.display()
                )),
            }
        }

        fn try_store(&self, binaries_config: &BinariesConfiguration) -> io::Result<()> {
            if self.directory.is_dir() {
                return Ok(());
            }
            let (Some(parent), Some(name)) = (self.directory.parent(), self.directory.file_name())
            else {
                return Err(io::Error::other("the cache entry path has no parent"));
            };
            fs::create_dir_all(parent)?;
            // An entry only appears, by rename, once it is complete, so concurrent builds never
            // import a partially written one.
            let staging = parent.join(format!(
                ".{}.{}",
                name.to_string_lossy(),
                std::process::id()
            ));
            binaries_config.export(&staging)?;
            if let Err(error) = fs::rename(&staging, &self.directory) {
                fs::remove_dir_all(&staging)?;
                // Losing the race to a concurrent build that stored the same entry is fine.
                if !self.directory.is_dir() {
                    return Err(error);
                }
            }
            Ok(())
        }
    }

    fn key(binaries_config: &BinariesConfiguration) -> io::Result<String> {
        let repository_hash = cargo::crate_repository_hash()?;
        let repository_short_hash = repository_hash
            .get(..20)
            .ok_or_else(|| io::Error::other("the packaged repository hash is too short"))?;

        let mut hasher = KeyHasher::new();

        let mut files = Vec::new();
        for input in PACKAGE_INPUTS {
            collect_package_files(Path::new(input), &mut files)?;
        }
        files.sort();
        for file in &files {
            hasher.write(file.to_string_lossy().as_bytes());
            hasher.write(&fs::read(file)?);
            // A cache hit runs no compiler that would report these inputs to cargo.
            cargo::rerun_if_file_changed(file);
        }

        for locked_package in locked_build_dependencies()? {
            hasher.write(locked_package.as_bytes());
        }

        let target_triple = std::env::var("TARGET").map_err(io::Error::other)?;
        let mut variables: Vec<String> = ENVIRONMENT_INPUTS
            .iter()
            .map(|name| name.to_string())
            .collect();
        for name in TARGETED_ENVIRONMENT_INPUTS {
            variables.push(name.to_string());
            variables.push(format!("{name}_{target_triple}"));
            variables.push(format!("{name}_{}", target_triple.replace('-', "_")));
            variables.push(format!("TARGET_{name}"));
        }
        for name in &variables {
            if let Some(value) = cargo::env_var(name) {
                hasher.write(name.as_bytes());
                hasher.write(value.as_bytes());
            }
        }
        if let Some(encoded_rustflags) = cargo::env_var("CARGO_ENCODED_RUSTFLAGS") {
            let flags = inherited_rustflags(&encoded_rustflags);
            if !flags.is_empty() {
                hasher.write(b"CARGO_ENCODED_RUSTFLAGS");
                for flag in flags {
                    hasher.write(flag.as_bytes());
                }
            }
        }
        if let Some(gn_args) = cargo::env_var("SKIA_GN_ARGS") {
            let gn_args = gn_args
                .split_whitespace()
                .filter(|arg| !arg.starts_with("cc_wrapper="))
                .collect::<Vec<_>>()
                .join(" ");
            // Builds that only add a compiler cache share entries with builds that set nothing.
            if !gn_args.is_empty() {
                hasher.write(b"SKIA_GN_ARGS");
                hasher.write(gn_args.as_bytes());
            }
        }

        // Tool output identifies the toolchain, so updating Xcode selects a new entry. The
        // compiler selection mirrors `skia::BuildConfiguration`.
        let cc = cargo::env_var("CLANGCC")
            .or_else(|| cargo::env_var("CC"))
            .unwrap_or_else(|| "clang".to_string());
        let cxx = cargo::env_var("CLANGCXX")
            .or_else(|| cargo::env_var("CXX"))
            .unwrap_or_else(|| "clang++".to_string());
        for compiler in [&cc, &cxx] {
            hasher.write(&command_output(compiler, &["--version"])?);
        }
        let target = cargo::target();
        if target.vendor == "apple" {
            if target.system != "darwin" {
                return Err(io::Error::other(format!(
                    "{target} SDKs are not part of the cache key"
                )));
            }
            hasher.write(&command_output(
                "xcrun",
                &["--sdk", "macosx", "--show-sdk-path"],
            )?);
            hasher.write(&command_output(
                "xcrun",
                &["--sdk", "macosx", "--show-sdk-build-version"],
            )?);
        }

        Ok(format!(
            "{}-fanta-{}",
            binaries_config.key(repository_short_hash),
            hasher.finish()
        ))
    }

    /// The effective `INHERITED_RUSTFLAGS` in `CARGO_ENCODED_RUSTFLAGS`, tokenized like cc-rs. As
    /// in cc-rs, a later occurrence of a flag overrides an earlier one, so flags that cargo
    /// duplicated give the same result.
    fn inherited_rustflags(encoded_rustflags: &str) -> Vec<String> {
        // Prefixes that take the next argument as their value, as cc-rs lists them.
        const SEPARATE_PREFIXES: &[&str] = &[
            "-C",
            "--codegen",
            "-Z",
            "-L",
            "-l",
            "-o",
            "-W",
            "--warn",
            "-A",
            "--allow",
            "-D",
            "--deny",
            "-F",
            "--forbid",
        ];

        let mut effective_values = BTreeMap::new();
        let mut separate_prefix = None;
        for argument in encoded_rustflags.split('\u{1f}') {
            let option = match separate_prefix.take() {
                Some("-C" | "--codegen" | "-Z") => Some(argument),
                Some(_) => None,
                None if SEPARATE_PREFIXES.contains(&argument) => {
                    separate_prefix = Some(argument);
                    continue;
                }
                None => argument
                    .strip_prefix("--codegen=")
                    .or_else(|| argument.strip_prefix("-C"))
                    .or_else(|| argument.strip_prefix("-Z")),
            };
            let Some(option) = option else {
                continue;
            };
            let (name, value) = option.split_once('=').unwrap_or((option, ""));
            if INHERITED_RUSTFLAGS.contains(&name) {
                effective_values.insert(name, value);
            }
        }
        effective_values
            .into_iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect()
    }

    /// Every locked version of `BUILD_DEPENDENCY_INPUTS`, read from the lockfile of the workspace
    /// that vendors this package. The package's own `Cargo.lock` is upstream's and unused there.
    /// A lockfile can hold several versions of one package; all of them are included.
    fn locked_build_dependencies() -> io::Result<Vec<String>> {
        let manifest_directory =
            PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").map_err(io::Error::other)?);
        let lockfile = manifest_directory
            .ancestors()
            .skip(1)
            .map(|directory| directory.join("Cargo.lock"))
            .find(|lockfile| lockfile.is_file())
            .ok_or_else(|| io::Error::other("no workspace Cargo.lock above the package"))?;
        let lock = toml::de::from_str::<toml::value::Table>(&fs::read_to_string(&lockfile)?)
            .map_err(io::Error::other)?;

        let mut locked_packages: Vec<String> = lock
            .get("package")
            .and_then(|packages| packages.as_array())
            .into_iter()
            .flatten()
            .filter_map(|package| {
                let name = package.get("name")?.as_str()?;
                let version = package.get("version")?.as_str()?;
                BUILD_DEPENDENCY_INPUTS
                    .contains(&name)
                    .then(|| format!("{name} {version}"))
            })
            .collect();
        locked_packages.sort();

        for name in BUILD_DEPENDENCY_INPUTS {
            let is_locked = locked_packages
                .iter()
                .any(|locked_package| locked_package.split(' ').next() == Some(*name));
            if !is_locked {
                return Err(io::Error::other(format!(
                    "{} does not lock {name}",
                    lockfile.display()
                )));
            }
        }
        Ok(locked_packages)
    }

    fn collect_package_files(path: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
        if path.is_dir() {
            for entry in fs::read_dir(path)? {
                collect_package_files(&entry?.path(), files)?;
            }
            return Ok(());
        }
        let hidden = path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with('.'));
        let rust_crate_source =
            path.starts_with("src") && path.extension().is_some_and(|extension| extension == "rs");
        if !hidden && !rust_crate_source {
            files.push(path.to_path_buf());
        }
        Ok(())
    }

    /// `command_line` may carry arguments, like the `CC` and `CXX` values passed to GN.
    fn command_output(command_line: &str, args: &[&str]) -> io::Result<Vec<u8>> {
        let mut words = command_line.split_whitespace();
        let program = words
            .next()
            .ok_or_else(|| io::Error::other("empty tool command"))?;
        let output = Command::new(program).args(words).args(args).output()?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "`{command_line} {}` failed with {}",
                args.join(" "),
                output.status
            )));
        }
        Ok(output.stdout)
    }

    /// 128-bit FNV-1a, which unlike `DefaultHasher` is stable across Rust releases.
    struct KeyHasher(u128);

    impl KeyHasher {
        fn new() -> Self {
            Self(0x6c62272e07bb014262b821756295c58d)
        }

        /// Length-prefixes `bytes` so that adjacent fields cannot run together.
        fn write(&mut self, bytes: &[u8]) {
            for byte in (bytes.len() as u64).to_le_bytes().iter().chain(bytes) {
                self.0 ^= u128::from(*byte);
                self.0 = self.0.wrapping_mul(0x0000000001000000000000000000013b);
            }
        }

        fn finish(&self) -> String {
            format!("{:032x}", self.0)
        }
    }
}
