use anyhow::{Context as _, Result};
use std::path::{Path, PathBuf};
use util::command::{Command, new_command};

pub fn find_system_git() -> Option<PathBuf> {
    which::which("git")
        .ok()
        .filter(|path| is_usable_system_git(path))
}

pub fn is_usable_system_git(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    if path == Path::new("/usr/bin/git") {
        // Apple's Git shim opens a developer-tools installer when no tools are
        // selected. Query the selection without launching the shim itself.
        #[allow(clippy::disallowed_methods)]
        return match util::command::new_std_command("/usr/bin/xcode-select")
            .arg("--print-path")
            .output()
        {
            Ok(output) if output.status.success() => {
                let directory = String::from_utf8_lossy(&output.stdout);
                Path::new(directory.trim()).join("usr/bin/git").is_file()
            }
            _ => false,
        };
    }
    let _ = path;
    true
}

pub fn select_git(system_git: Option<PathBuf>, bundled_git: Option<PathBuf>) -> Result<PathBuf> {
    system_git
        .filter(|path| is_usable_system_git(path))
        .or_else(|| bundled_git.filter(|path| bundled_support_directory(path).is_some()))
        .context("Git is unavailable. Install Git or reinstall Fanta with its complete Git bundle.")
}

pub fn bundled_support_directory(binary: &Path) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        use std::{ffi::OsStr, os::unix::fs::PermissionsExt};

        let executable = |path: &Path| {
            path.metadata().is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        };
        let directory = binary.parent()?;
        if directory.file_name() != Some(OsStr::new("MacOS")) || !executable(binary) {
            return None;
        }
        let contents = directory.parent()?;
        if contents.file_name() != Some(OsStr::new("Contents")) {
            return None;
        }
        let support = contents.join("Resources/git");
        let helpers = support.join("libexec/git-core");
        if [
            "git-remote-http",
            "git-remote-https",
            "git-upload-pack",
            "git-receive-pack",
        ]
        .iter()
        .all(|name| executable(&helpers.join(name)))
            && support.join("share/git-core/templates").is_dir()
        {
            return Some(support);
        }
    }
    let _ = binary;
    None
}

// This is the low-level constructor behind `GitBinary::build_command`; the
// repository layer adds the mandatory security arguments before execution.
#[allow(clippy::disallowed_methods)]
pub fn command(binary: &Path) -> Command {
    let mut command = new_command(binary);
    if let Some(support) = bundled_support_directory(binary) {
        // Dugite is built with an absolute /libexec path. A command-line
        // override also survives a project's inherited GIT_EXEC_PATH value.
        let mut exec_path = std::ffi::OsString::from("--exec-path=");
        exec_path.push(support.join("libexec/git-core"));
        command.arg(exec_path);
        command.env("GIT_TEMPLATE_DIR", support.join("share/git-core/templates"));
    }
    command
}
