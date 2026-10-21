#!/usr/bin/env sh
set -eu

# Downloads a tarball from https://fantaisa.net/releases and unpacks it
# into ~/.local/. If you'd prefer to do this manually, instructions are at
# https://fantaisa.net/docs/linux.

main() {
    platform="$(uname -s)"
    arch="$(uname -m)"
    channel="${FANTA_CHANNEL:-stable}"
    FANTA_VERSION="${FANTA_VERSION:-latest}"
    # Use TMPDIR if available (for environments with non-standard temp directories)
    if [ -n "${TMPDIR:-}" ] && [ -d "${TMPDIR}" ]; then
        temp="$(mktemp -d "$TMPDIR/fanta-XXXXXX")"
    else
        temp="$(mktemp -d "/tmp/fanta-XXXXXX")"
    fi

    if [ "$platform" = "Darwin" ]; then
        platform="macos"
    elif [ "$platform" = "Linux" ]; then
        platform="linux"
    else
        echo "Unsupported platform $platform"
        exit 1
    fi

    case "$platform-$arch" in
        macos-arm64* | linux-arm64* | linux-armhf | linux-aarch64)
            arch="aarch64"
            ;;
        macos-x86* | linux-x86* | linux-i686*)
            arch="x86_64"
            ;;
        *)
            echo "Unsupported platform or architecture"
            exit 1
            ;;
    esac

    if command -v curl >/dev/null 2>&1; then
        curl () {
            command curl -fL "$@"
        }
    elif command -v wget >/dev/null 2>&1; then
        curl () {
            wget -O- "$@"
        }
    else
        echo "Could not find 'curl' or 'wget' in your path"
        exit 1
    fi

    "$platform" "$@"

    if [ "$(command -v fanta)" = "$HOME/.local/bin/fanta" ]; then
        echo "Fanta has been installed. Run with 'fanta'"
    else
        echo "To run Fanta from your terminal, you must add ~/.local/bin to your PATH"
        echo "Run:"

        case "$SHELL" in
            *zsh)
                echo "   echo 'export PATH=\$HOME/.local/bin:\$PATH' >> ~/.zshrc"
                echo "   source ~/.zshrc"
                ;;
            *fish)
                echo "   fish_add_path -U $HOME/.local/bin"
                ;;
            *)
                echo "   echo 'export PATH=\$HOME/.local/bin:\$PATH' >> ~/.bashrc"
                echo "   source ~/.bashrc"
                ;;
        esac

        echo "To run Fanta now, '~/.local/bin/fanta'"
    fi
}

linux() {
    if [ -n "${FANTA_BUNDLE_PATH:-}" ]; then
        cp "$FANTA_BUNDLE_PATH" "$temp/fanta-linux-$arch.tar.gz"
    else
        echo "Downloading Fanta version: $FANTA_VERSION"
        curl "https://api.fantaisa.net/releases/$channel/$FANTA_VERSION/download?asset=fanta&arch=$arch&os=linux&source=install.sh" > "$temp/fanta-linux-$arch.tar.gz"
    fi

    suffix=""
    if [ "$channel" != "stable" ]; then
        suffix="-$channel"
    fi

    appid=""
    case "$channel" in
      stable)
        appid="dev.fanta.Fanta"
        ;;
      nightly)
        appid="dev.fanta.Fanta-Nightly"
        ;;
      preview)
        appid="dev.fanta.Fanta-Preview"
        ;;
      dev)
        appid="dev.fanta.Fanta-Dev"
        ;;
      *)
        echo "Unknown release channel: ${channel}. Using stable app ID."
        appid="dev.fanta.Fanta"
        ;;
    esac

    # Unpack
    rm -rf "$HOME/.local/fanta$suffix.app"
    mkdir -p "$HOME/.local/fanta$suffix.app"
    tar -xzf "$temp/fanta-linux-$arch.tar.gz" -C "$HOME/.local/"

    # Setup ~/.local directories
    mkdir -p "$HOME/.local/bin" "$HOME/.local/share/applications"

    # Link the binary
    if [ -f "$HOME/.local/fanta$suffix.app/bin/fanta" ]; then
        ln -sf "$HOME/.local/fanta$suffix.app/bin/fanta" "$HOME/.local/bin/fanta"
    else
        # support for versions before 0.139.x.
        ln -sf "$HOME/.local/fanta$suffix.app/bin/cli" "$HOME/.local/bin/fanta"
    fi

    # Copy .desktop file
    desktop_file_path="$HOME/.local/share/applications/${appid}.desktop"
    src_dir="$HOME/.local/fanta$suffix.app/share/applications"
    if [ -f "$src_dir/${appid}.desktop" ]; then
        cp "$src_dir/${appid}.desktop" "${desktop_file_path}"
    else
        # Fallback for older tarballs
        cp "$src_dir/fanta$suffix.desktop" "${desktop_file_path}"
    fi
    sed -i "s|Icon=fanta|Icon=$HOME/.local/fanta$suffix.app/share/icons/hicolor/512x512/apps/fanta.png|g" "${desktop_file_path}"
    sed -i "s|Exec=fanta|Exec=$HOME/.local/fanta$suffix.app/bin/fanta|g" "${desktop_file_path}"

    # Register the .fnx and .fig file types, then the desktop entry that opens them
    mime_file="$HOME/.local/fanta$suffix.app/share/mime/packages/${appid}.xml"
    if [ -f "$mime_file" ]; then
        mkdir -p "$HOME/.local/share/mime/packages"
        cp "$mime_file" "$HOME/.local/share/mime/packages/${appid}.xml"
        if command -v update-mime-database >/dev/null 2>&1; then
            update-mime-database "$HOME/.local/share/mime" ||
                echo "Could not update the MIME database, so .fnx and .fig files may not open in Fanta."
        fi
    fi
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "$HOME/.local/share/applications" ||
            echo "Could not update the desktop database, so Fanta may be missing from 'Open With' menus."
    fi
}

macos() {
    echo "Downloading Fanta version: $FANTA_VERSION"
    curl "https://api.fantaisa.net/releases/$channel/$FANTA_VERSION/download?asset=fanta&os=macos&arch=$arch&source=install.sh" > "$temp/Fanta-$arch.dmg"
    hdiutil attach -quiet "$temp/Fanta-$arch.dmg" -mountpoint "$temp/mount"
    app="$(cd "$temp/mount/"; echo *.app)"
    echo "Installing $app"
    if [ -d "/Applications/$app" ]; then
        echo "Removing existing $app"
        rm -rf "/Applications/$app"
    fi
    ditto "$temp/mount/$app" "/Applications/$app"
    hdiutil detach -quiet "$temp/mount"

    mkdir -p "$HOME/.local/bin"
    # Link the binary
    ln -sf "/Applications/$app/Contents/MacOS/cli" "$HOME/.local/bin/fanta"
}

main "$@"
