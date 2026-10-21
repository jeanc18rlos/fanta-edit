#!/usr/bin/env sh
set -eu

# Uninstalls Fanta that was installed using the install.sh script

check_remaining_installations() {
    platform="$(uname -s)"
    if [ "$platform" = "Darwin" ]; then
        # Check for any Fanta variants in /Applications
        remaining=$(ls -d /Applications/Fanta*.app 2>/dev/null | wc -l)
        [ "$remaining" -eq 0 ]
    else
        # Check for any Fanta variants in ~/.local
        remaining=$(ls -d "$HOME/.local/fanta"*.app 2>/dev/null | wc -l)
        [ "$remaining" -eq 0 ]
    fi
}

prompt_remove_preferences() {
    printf "Do you want to keep your Fanta preferences? [Y/n] "
    read -r response
    case "$response" in
        [nN]|[nN][oO])
            rm -rf "$HOME/.config/fanta"
            echo "Preferences removed."
            ;;
        *)
            echo "Preferences kept."
            ;;
    esac
}

main() {
    platform="$(uname -s)"
    channel="${FANTA_CHANNEL:-stable}"

    if [ "$platform" = "Darwin" ]; then
        platform="macos"
    elif [ "$platform" = "Linux" ]; then
        platform="linux"
    else
        echo "Unsupported platform $platform"
        exit 1
    fi

    "$platform"

    echo "Fanta has been uninstalled"
}

linux() {
    suffix=""
    if [ "$channel" != "stable" ]; then
        suffix="-$channel"
    fi

    appid=""
    db_suffix="stable"
    case "$channel" in
      stable)
        appid="dev.fanta.Fanta"
        db_suffix="stable"
        ;;
      nightly)
        appid="dev.fanta.Fanta-Nightly"
        db_suffix="nightly"
        ;;
      preview)
        appid="dev.fanta.Fanta-Preview"
        db_suffix="preview"
        ;;
      dev)
        appid="dev.fanta.Fanta-Dev"
        db_suffix="dev"
        ;;
      *)
        echo "Unknown release channel: ${channel}. Using stable app ID."
        appid="dev.fanta.Fanta"
        db_suffix="stable"
        ;;
    esac

    # Remove the app directory
    rm -rf "$HOME/.local/fanta$suffix.app"

    # Remove the binary symlink
    rm -f "$HOME/.local/bin/fanta"

    # Remove the .desktop file
    rm -f "$HOME/.local/share/applications/${appid}.desktop"
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "$HOME/.local/share/applications" ||
            echo "Could not update the desktop database."
    fi

    # Remove the file types install.sh registered
    if [ -f "$HOME/.local/share/mime/packages/${appid}.xml" ]; then
        rm -f "$HOME/.local/share/mime/packages/${appid}.xml"
        if command -v update-mime-database >/dev/null 2>&1; then
            update-mime-database "$HOME/.local/share/mime" ||
                echo "Could not update the MIME database."
        fi
    fi

    # Remove the database directory for this channel
    rm -rf "$HOME/.local/share/fanta/db/0-$db_suffix"

    # Remove socket file
    rm -f "$HOME/.local/share/fanta/fanta-$db_suffix.sock"

    # Remove the entire Fanta directory if no installations remain
    if check_remaining_installations; then
        rm -rf "$HOME/.local/share/fanta"
        prompt_remove_preferences
    fi

    rm -rf $HOME/.fanta_server
}

macos() {
    app="Fanta.app"
    db_suffix="stable"
    app_id="dev.fanta.Fanta"
    case "$channel" in
      nightly)
        app="Fanta Nightly.app"
        db_suffix="nightly"
        app_id="dev.fanta.Fanta-Nightly"
        ;;
      preview)
        app="Fanta Preview.app"
        db_suffix="preview"
        app_id="dev.fanta.Fanta-Preview"
        ;;
      dev)
        app="Fanta Dev.app"
        db_suffix="dev"
        app_id="dev.fanta.Fanta-Dev"
        ;;
    esac

    # Remove the app bundle
    if [ -d "/Applications/$app" ]; then
        rm -rf "/Applications/$app"
    fi

    # Remove the binary symlink
    rm -f "$HOME/.local/bin/fanta"

    # Remove the database directory for this channel
    rm -rf "$HOME/Library/Application Support/Fanta/db/0-$db_suffix"

    # Remove app-specific files and directories
    rm -rf "$HOME/Library/Application Support/com.apple.sharedfilelist/com.apple.LSSharedFileList.ApplicationRecentDocuments/$app_id.sfl"*
    rm -rf "$HOME/Library/Caches/$app_id"
    rm -rf "$HOME/Library/HTTPStorages/$app_id"
    rm -rf "$HOME/Library/Preferences/$app_id.plist"
    rm -rf "$HOME/Library/Saved Application State/$app_id.savedState"

    # Remove the entire Fanta directory if no installations remain
    if check_remaining_installations; then
        rm -rf "$HOME/Library/Application Support/Fanta"
        rm -rf "$HOME/Library/Logs/Fanta"

        prompt_remove_preferences
    fi

    rm -rf $HOME/.fanta_server
}

main "$@"
