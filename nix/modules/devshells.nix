{ inputs, ... }:
{
  perSystem =
    { pkgs, system, ... }:
    let
      # NOTE: Duplicated because this is in a separate flake-parts partition
      # than ./packages.nix
      mkFanta = import ../toolchain.nix { inherit inputs; };
      fanta-editor = mkFanta pkgs;

      # mdBook pinned to 0.4.40 via a dedicated nixpkgs input, because the docs
      # rely on behavior that newer mdBook releases break (see
      # `crates/docs_preprocessor/Cargo.toml`).
      mdbook = (import inputs.nixpkgs-mdbook { inherit system; }).mdbook;

      # Prebuilt docs preprocessor/postprocessor binary. `docs/book.toml`
      # defaults to `cargo run -p docs_preprocessor` so non-Nix contributors are
      # unaffected; in the devshell we point mdBook at this prebuilt binary via
      # the `MDBOOK_*` env vars below so `mdbook build docs` doesn't have to
      # compile the preprocessor on every run.
      #
      # We reuse `fanta-editor`'s crane builder and shared arguments (exposed via
      # `passthru`) rather than `overrideAttrs`, because crane bakes
      # `cargoExtraArgs` into the build command at evaluation time.
      docs-preprocessor = fanta-editor.passthru.craneLib.buildPackage (
        fanta-editor.passthru.commonArgs
        // {
          inherit (fanta-editor.passthru) cargoArtifacts;
          pname = "fanta-docs-preprocessor";
          cargoExtraArgs = "-p docs_preprocessor --locked";
          dontUseCmakeConfigure = true;
          meta = {
            description = "mdBook preprocessor and postprocessor for the Fanta docs";
            mainProgram = "docs_preprocessor";
          };
        }
      );

      rustBin = inputs.rust-overlay.lib.mkRustBin { } pkgs;
      rustToolchain = rustBin.fromRustupToolchainFile ../../rust-toolchain.toml;

      baseEnv =
        (fanta-editor.overrideAttrs (attrs: {
          passthru.env = attrs.env;
        })).env; # exfil `env`; it's not in drvAttrs

      # Musl cross-compiler for building remote_server
      muslCross = pkgs.pkgsCross.musl64;

      # Cargo build timings wrapper script
      wrappedCargo = pkgs.writeShellApplication {
        name = "cargo";
        runtimeInputs = [ pkgs.nodejs ];
        text =
          let
            pathToCargoScript = ./. + "/../../script/cargo";
          in
          ''
            NIX_WRAPPER=1 CARGO=${rustToolchain}/bin/cargo ${pathToCargoScript} "$@"
          '';
      };
    in
    {
      devShells.default = (pkgs.mkShell.override { inherit (fanta-editor) stdenv; }) {
        name = "fanta-editor-dev";
        inputsFrom = [ fanta-editor ];

        packages =
          with pkgs;
          [
            wrappedCargo # must be first, to shadow the `cargo` provided by `rustToolchain`
            rustToolchain # cargo, rustc, and rust-toolchain.toml components included
            cargo-nextest
            cargo-hakari
            cargo-machete
            cargo-zigbuild
            # TODO: package protobuf-language-server for editing fanta.proto
            # TODO: add other tools used in our scripts

            # `build.nix` adds this to the `fanta-editor` wrapper (see `postFixup`)
            # we'll just put it on `$PATH`:
            nodejs_22
            zig

            # Documentation tooling: `nix develop -c mdbook build docs`
            mdbook
            docs-preprocessor

            # A11y testing infra
            gobject-introspection
            at-spi2-core
            (python3.withPackages (ps: [
              ps.pyatspi
              ps.pygobject3
            ]))
          ]
          ++ lib.optionals stdenv.hostPlatform.isLinux [ accerciser ];

        env =
          (removeAttrs baseEnv [
            "LK_CUSTOM_WEBRTC" # download the staticlib during the build as usual
            "FANTA_UPDATE_EXPLANATION" # allow auto-updates
            "CARGO_PROFILE" # let you specify the profile
            "TARGET_DIR"
          ])
          // {
            FONTCONFIG_FILE = pkgs.makeFontsConf {
              fontDirectories = [
                pkgs.lilex
                pkgs.ibm-plex
              ];
            };
            PROTOC = "${pkgs.protobuf}/bin/protoc";

            # Point mdBook at the prebuilt preprocessor/postprocessor binary
            # instead of `cargo run`. mdBook lowercases these keys and turns `_`
            # into `-`, so they map to `preprocessor.fanta-docs-preprocessor.command`
            # and `output.fanta-html.command` in `docs/book.toml`.
            MDBOOK_PREPROCESSOR__FANTA_DOCS_PREPROCESSOR__COMMAND = "${docs-preprocessor}/bin/docs_preprocessor";
            MDBOOK_OUTPUT__FANTA_HTML__COMMAND = "${docs-preprocessor}/bin/docs_preprocessor postprocess";

            FANTA_ZSTD_MUSL_LIB = "${pkgs.pkgsCross.musl64.pkgsStatic.zstd.out}/lib";
            # For aws-lc-sys musl cross-compilation
            CC_x86_64_unknown_linux_musl = "${muslCross.stdenv.cc}/bin/x86_64-unknown-linux-musl-gcc";
          };
      };
    };
}
