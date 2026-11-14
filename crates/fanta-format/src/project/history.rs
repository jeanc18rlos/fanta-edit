//! Snapshots of a project's files (spec 0009 R6, R7): a safety net under
//! every save, kept in a private git directory at `.fanta/history.git`.
//!
//! The store drives the `git` binary with `GIT_DIR` and `GIT_WORK_TREE` set,
//! so it never reads or writes a repository the user owns, and works the
//! same whether or not the project sits inside one. Each snapshot is a
//! parentless commit under its own ref: any snapshot can be dropped without
//! rewriting the others, and git still stores unchanged files once.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::error::{FormatError, Result};

use super::diagnostic::{SNAPSHOT_UNAVAILABLE, SourceDiagnostic};

/// The directory at the project root that holds Fanta's private state. Every
/// walker of a project directory skips it.
pub const HISTORY_DIR: &str = ".fanta";
const GIT_DIR: &str = ".fanta/history.git";
const NULL_DEVICE: &str = if cfg!(windows) { "NUL" } else { "/dev/null" };
/// What `git init --bare` writes, less what it probes from the file system.
const STORE_CONFIG: &str = if cfg!(windows) {
    "[core]\n\trepositoryformatversion = 0\n\tbare = true\n\tfilemode = false\n\tsymlinks = false\n"
} else {
    "[core]\n\trepositoryformatversion = 0\n\tbare = true\n\tfilemode = true\n"
};
/// Outranks any `.gitattributes` in the project: snapshots hold the bytes on
/// disk, with no line-ending, encoding or filter conversion.
const STORE_ATTRIBUTES: &str = "* -text -filter -ident -working-tree-encoding\n";
const STORE_HEAD: &str = "ref: refs/heads/main\n";
const SNAPSHOT_REFS: &str = "refs/snapshots/";
const KEEP_EVERYTHING_FOR: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const MILLIS_PER_DAY: u128 = 24 * 60 * 60 * 1000;

/// Why a snapshot was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotReason {
    BeforeSave,
    AfterSave,
    BeforeRestore,
}

impl SnapshotReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::BeforeSave => "before-save",
            Self::AfterSave => "after-save",
            Self::BeforeRestore => "before-restore",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        [Self::BeforeSave, Self::AfterSave, Self::BeforeRestore]
            .into_iter()
            .find(|reason| reason.as_str() == text)
    }
}

/// One recorded state of the project files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// The name to restore by: the time it was taken, in milliseconds since
    /// the Unix epoch, nudged forward when two fall in the same millisecond.
    pub id: String,
    pub taken_at: SystemTime,
    pub reason: SnapshotReason,
}

/// The snapshot store of one project directory.
pub struct History {
    dir: PathBuf,
    git: OsString,
}

impl History {
    /// The store for the project at `dir`, using the `git` on `PATH`.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            // Git for Windows cannot open the `\\?\` form that `canonicalize` returns there.
            dir: dunce::simplified(&dir.into()).to_path_buf(),
            git: "git".into(),
        }
    }

    /// Use this git binary instead: the editor ships its own.
    pub fn with_git(mut self, git: impl Into<OsString>) -> Self {
        self.git = git.into();
        self
    }

    /// Record the project files as they are now.
    pub fn record(&self, reason: SnapshotReason) -> Result<Snapshot> {
        self.record_at(reason, SystemTime::now())
    }

    /// Call before a save writes. Records the files only when they differ
    /// from the newest snapshot: every save ends with one, so a difference
    /// is a change made outside Fanta, which the save may be about to
    /// rewrite (R6). A snapshot that cannot be taken never stops the save;
    /// it comes back as a finding for the save's report.
    pub fn before_save(&self) -> Option<SourceDiagnostic> {
        self.record_if_changed(SnapshotReason::BeforeSave).err().map(unavailable)
    }

    /// Call after a save has written everything, then drop what retention
    /// no longer keeps.
    pub fn after_save(&self) -> Option<SourceDiagnostic> {
        self.record_if_changed(SnapshotReason::AfterSave)
            .and_then(|_| self.prune(SystemTime::now()))
            .err()
            .map(unavailable)
    }

    fn record_if_changed(&self, reason: SnapshotReason) -> Result<Option<Snapshot>> {
        if let Some(newest) = self.list()?.first() {
            let tree = self.stage()?;
            let name = format!("{SNAPSHOT_REFS}{}^{{tree}}", newest.id);
            if self.run(self.git()?.args(["rev-parse", &name]))?.trim() == tree {
                return Ok(None);
            }
        }
        self.record(reason).map(Some)
    }

    /// Bring the private index up to the files on disk; returns its tree.
    fn stage(&self) -> Result<String> {
        // `-f`: the project's own `.gitignore` must not thin the snapshot, or
        // a restore would not give back what the save changed.
        self.run(self.git()?.args(["add", "-A", "-f", "--", ".", ":(exclude).fanta"]))?;
        Ok(self.run(self.git()?.arg("write-tree"))?.trim().to_owned())
    }

    fn record_at(&self, reason: SnapshotReason, time: SystemTime) -> Result<Snapshot> {
        let tree = self.stage()?;
        let since_epoch = time.duration_since(UNIX_EPOCH).unwrap_or_default();
        let date = format!("@{} +0000", since_epoch.as_secs());
        let commit = self.run(
            self.git()?
                .env("GIT_AUTHOR_DATE", &date)
                .env("GIT_COMMITTER_DATE", &date)
                .args(["commit-tree", "--no-gpg-sign", "-m", reason.as_str()])
                .arg(&tree),
        )?;
        let taken = since_epoch.as_millis();
        // An empty old value makes `update-ref` fail when the ref exists, so
        // two snapshots in one millisecond cannot replace each other.
        for millis in taken..taken + 1000 {
            let id = format!("{millis:013}");
            let name = format!("{SNAPSHOT_REFS}{id}");
            if self.succeeds(self.git()?.args(["update-ref", &name, commit.trim(), ""]))? {
                return Ok(Snapshot {
                    id,
                    taken_at: UNIX_EPOCH + Duration::from_millis(millis as u64),
                    reason,
                });
            }
        }
        Err(FormatError::Snapshot(
            "could not name a new snapshot".to_owned(),
        ))
    }

    /// Every snapshot, newest first. A project that never recorded one has
    /// none.
    pub fn list(&self) -> Result<Vec<Snapshot>> {
        if !self.dir.join(GIT_DIR).is_dir() {
            return Ok(Vec::new());
        }
        let refs = self.run(self.git()?.args([
            "for-each-ref",
            "--format=%(refname:lstrip=2) %(contents:subject)",
            SNAPSHOT_REFS,
        ]))?;
        let mut snapshots: Vec<Snapshot> = refs
            .lines()
            .filter_map(|line| {
                let (id, reason) = line.split_once(' ')?;
                Some(Snapshot {
                    id: id.to_owned(),
                    taken_at: UNIX_EPOCH + Duration::from_millis(id.parse().ok()?),
                    reason: SnapshotReason::parse(reason)?,
                })
            })
            .collect();
        snapshots.sort_by(|a, b| b.id.cmp(&a.id));
        Ok(snapshots)
    }

    /// Make the project files equal to snapshot `id`. The state being
    /// replaced is recorded first and returned, so a restore can be undone.
    pub fn restore(&self, id: &str) -> Result<Snapshot> {
        if !id.bytes().all(|byte| byte.is_ascii_digit()) || id.is_empty() {
            return Err(FormatError::Snapshot(format!("{id:?} is not a snapshot")));
        }
        let target = format!("{SNAPSHOT_REFS}{id}");
        let tree = format!("{target}^{{tree}}");
        if !self.dir.join(GIT_DIR).is_dir()
            || !self.succeeds(self.git()?.args(["rev-parse", "--verify", "--quiet", &tree]))?
        {
            return Err(FormatError::Snapshot(format!("no snapshot {id}")));
        }
        let before = self.record(SnapshotReason::BeforeRestore)?;
        let gone = self.run(self.git()?.args([
            "diff-tree",
            "-r",
            "-z",
            "--name-only",
            "--no-renames",
            "--diff-filter=D",
            &format!("{SNAPSHOT_REFS}{}", before.id),
            &target,
        ]))?;
        for relative in gone.split('\0').filter(|path| !path.is_empty()) {
            match fs::remove_file(self.dir.join(relative)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            // Git does not record empty directories, so leave none behind.
            let mut parent = Path::new(relative).parent();
            while let Some(folder) = parent.filter(|folder| !folder.as_os_str().is_empty()) {
                if fs::remove_dir(self.dir.join(folder)).is_err() {
                    break;
                }
                parent = folder.parent();
            }
        }
        self.run(self.git()?.args(["read-tree", &target]))?;
        self.run(self.git()?.args(["checkout-index", "-a", "-f"]))?;
        Ok(before)
    }

    /// Drop what retention no longer keeps (R6): everything is kept for 30
    /// days, then the last snapshot of each UTC day.
    pub fn prune(&self, now: SystemTime) -> Result<()> {
        let mut kept_days = std::collections::BTreeSet::new();
        let mut dropped = false;
        // Newest first, so the first snapshot seen for a day is its last.
        for snapshot in self.list()? {
            let recent = now
                .duration_since(snapshot.taken_at)
                .is_ok_and(|age| age <= KEEP_EVERYTHING_FOR)
                || snapshot.taken_at > now;
            let day = snapshot
                .taken_at
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                / MILLIS_PER_DAY;
            if recent || kept_days.insert(day) {
                continue;
            }
            self.run(
                self.git()?
                    .args(["update-ref", "-d", &format!("{SNAPSHOT_REFS}{}", snapshot.id)]),
            )?;
            dropped = true;
        }
        if dropped {
            self.run(self.git()?.args(["gc", "--auto", "--quiet"]))?;
        }
        Ok(())
    }

    /// Make the store directory safe to hand to git, creating it when it is
    /// missing. The directory travels with the project folder, so everything
    /// in it is input from whoever supplied the project, and is checked
    /// before every command, not only at creation:
    ///
    /// - A link (on Windows also a junction) at `.fanta`, at the store, or at
    ///   any entry git reads by a fixed name is refused: following one lets
    ///   the project choose which files git reads and which Fanta overwrites.
    ///   A store that is a file is refused too: git follows a `gitdir:` file.
    /// - `HEAD`, `config` and `info/attributes` hold nothing a user sets, so
    ///   they are overwritten with fixed content instead of being parsed.
    ///   That drops every setting that names a command (filters,
    ///   `diff.external`, `core.fsmonitor`, `core.hooksPath`, includes,
    ///   extensions), and a `.gitattributes` in the project then names
    ///   drivers that do not exist.
    /// - `commondir` (moves config, objects and refs elsewhere),
    ///   `objects/info/alternates` (borrows objects from any path) and
    ///   `info/grafts` are deleted: the store never writes them.
    /// - `hooks/` is left in place and never run: `git` points
    ///   `core.hooksPath` at the null device on the command line, which
    ///   outranks every file.
    /// - `index`, `packed-refs`, `refs/`, `objects/` and `worktrees/` are
    ///   data. A crafted one gives wrong or unreadable snapshots, which a
    ///   project could ship anyway, and runs nothing.
    fn secure_store(&self) -> Result<()> {
        let private = self.dir.join(HISTORY_DIR);
        let git_dir = self.dir.join(GIT_DIR);
        refuse_link(&private)?;
        refuse_link(&git_dir)?;
        if fs::symlink_metadata(&git_dir).is_err() {
            fs::create_dir_all(&private)?;
            // The directory ignores itself, so the user's git never sees it
            // and their own `.gitignore` is not edited.
            fs::write(private.join(".gitignore"), "*\n")?;
            fs::create_dir(&git_dir)?;
        }
        if !fs::symlink_metadata(&git_dir)?.is_dir() {
            return Err(refused(&git_dir));
        }
        for name in [
            "HEAD",
            "config",
            "hooks",
            "info",
            "info/attributes",
            "info/grafts",
            "objects",
            "objects/info",
            "objects/info/alternates",
            "objects/info/http-alternates",
            "refs",
            "packed-refs",
            "index",
            "commondir",
            "worktrees",
        ] {
            refuse_link(&git_dir.join(name))?;
        }
        for name in [
            "commondir",
            "info/grafts",
            "objects/info/alternates",
            "objects/info/http-alternates",
        ] {
            match fs::remove_file(git_dir.join(name)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        for folder in ["objects", "refs", "info"] {
            fs::create_dir_all(git_dir.join(folder))?;
        }
        for (name, content) in [
            ("HEAD", STORE_HEAD),
            ("config", STORE_CONFIG),
            ("info/attributes", STORE_ATTRIBUTES),
        ] {
            let path = git_dir.join(name);
            // Rewritten only when it differs: a save runs a dozen git
            // commands, and none should find the file half written.
            if fs::read(&path).ok().as_deref() != Some(content.as_bytes()) {
                fs::write(&path, content)?;
            }
        }
        Ok(())
    }

    /// A git command against the private repository only, once
    /// `secure_store` has made that repository inert. The user's and the
    /// system's configuration are shut out: signing, hooks, filters and
    /// line-ending settings must not shape or block a snapshot.
    fn git(&self) -> Result<Command> {
        self.secure_store()?;
        let mut command = Command::new(&self.git);
        // Set when Fanta is started from inside a git hook or alias, or by a
        // shell profile: they redirect git or name programs for it to start.
        for (name, _) in std::env::vars_os() {
            if is_inherited_git_setting(&name.to_string_lossy()) {
                command.env_remove(name);
            }
        }
        command
            .current_dir(&self.dir)
            .env("GIT_DIR", self.dir.join(GIT_DIR))
            .env("GIT_WORK_TREE", &self.dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", NULL_DEVICE)
            .env("GIT_ATTR_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_AUTHOR_NAME", "Fanta")
            .env("GIT_AUTHOR_EMAIL", "snapshots@fanta.invalid")
            .env("GIT_COMMITTER_NAME", "Fanta")
            .env("GIT_COMMITTER_EMAIL", "snapshots@fanta.invalid")
            .args(["-c", "core.autocrlf=false", "-c", "core.fsmonitor=false"])
            .args(["-c", &format!("core.hooksPath={NULL_DEVICE}")]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            // CREATE_NO_WINDOW: no console flashes up under the editor.
            command.creation_flags(0x0800_0000);
        }
        Ok(command)
    }

    fn succeeds(&self, command: &mut Command) -> Result<bool> {
        Ok(self.output(command)?.status.success())
    }

    #[allow(
        clippy::disallowed_methods,
        reason = "saves are synchronous and run on a background thread; a snapshot is part of the save"
    )]
    fn output(&self, command: &mut Command) -> Result<std::process::Output> {
        command.output().map_err(|error| {
            FormatError::Snapshot(format!("could not run {}: {error}", self.git.to_string_lossy()))
        })
    }

    fn run(&self, command: &mut Command) -> Result<String> {
        let output = self.output(command)?;
        if !output.status.success() {
            return Err(FormatError::Snapshot(format!(
                "git failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        String::from_utf8(output.stdout)
            .map_err(|_| FormatError::Snapshot("git printed a path that is not UTF-8".to_owned()))
    }
}

/// Environment the store must not pass on: every `GIT_*` variable except
/// the one that says where git's own programs are, and the variables naming
/// an editor, pager or password prompt for git to start.
fn is_inherited_git_setting(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    (name.starts_with("GIT_") && name != "GIT_EXEC_PATH")
        || ["EDITOR", "VISUAL", "PAGER", "SSH_ASKPASS"].contains(&name.as_str())
}

fn refuse_link(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(refused(path)),
        _ => Ok(()),
    }
}

fn refused(path: &Path) -> FormatError {
    FormatError::Snapshot(format!(
        "{} is a link or not a folder, so the snapshot store is not used",
        path.display()
    ))
}

fn unavailable(error: FormatError) -> SourceDiagnostic {
    SourceDiagnostic::warning(
        SNAPSHOT_UNAVAILABLE,
        format!("no snapshot was taken around this save, so it cannot be undone from the snapshots: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Spec 0009 R6: a snapshot before a save only when the files changed
    /// outside Fanta, one after every save that changed something, and a
    /// missing git never stops a save.
    #[test]
    fn a_resolved_project_path_takes_snapshots() {
        let scratch = tempfile::tempdir().unwrap();
        let project = scratch.path().canonicalize().unwrap();
        write(&project, "fanta.json", "{}");
        assert_eq!(History::new(&project).before_save(), None);
    }

    #[test]
    fn saves_are_bracketed_by_snapshots_only_when_something_changed() {
        let scratch = tempfile::tempdir().unwrap();
        let project = scratch.path();
        write(project, "fanta.json", "{}");
        let history = History::new(project);
        let reasons = || -> Vec<SnapshotReason> {
            history.list().unwrap().iter().rev().map(|snapshot| snapshot.reason).collect()
        };
        use SnapshotReason::{AfterSave, BeforeSave};

        // The first save of a project records what was there.
        assert_eq!(history.before_save(), None);
        write(project, "pages/home/page.svg", "<svg/>");
        assert_eq!(history.after_save(), None);
        assert_eq!(reasons(), [BeforeSave, AfterSave]);

        // Nothing changed outside, and the save wrote nothing.
        assert_eq!(history.before_save(), None);
        assert_eq!(history.after_save(), None);
        assert_eq!(reasons(), [BeforeSave, AfterSave]);

        // An outside edit, then a save that rewrites it.
        write(project, "pages/home/page.svg", "<svg data-hand-edit=\"\"/>");
        assert_eq!(history.before_save(), None);
        write(project, "pages/home/page.svg", "<svg/>");
        assert_eq!(history.after_save(), None);
        assert_eq!(reasons(), [BeforeSave, AfterSave, BeforeSave, AfterSave]);

        let finding = History::new(project)
            .with_git("no-such-git-binary")
            .before_save()
            .expect("a finding, not a failure");
        assert_eq!(finding.code, SNAPSHOT_UNAVAILABLE);
    }

    fn files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
        let mut found = BTreeMap::new();
        let mut pending = vec![dir.to_path_buf()];
        while let Some(folder) = pending.pop() {
            for entry in fs::read_dir(&folder).unwrap() {
                let path = entry.unwrap().path();
                let name = path.strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/");
                if name == HISTORY_DIR || name == ".git" {
                    continue;
                }
                if path.is_dir() {
                    pending.push(path);
                } else {
                    found.insert(name, fs::read(&path).unwrap());
                }
            }
        }
        found
    }

    fn write(dir: &Path, name: &str, bytes: &str) {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    #[allow(clippy::disallowed_methods)]
    fn user_git(dir: &Path, arguments: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(dir)
            .args(["-c", "user.name=User", "-c", "user.email=user@example.invalid"])
            .args(arguments)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8(output.stdout).unwrap()
    }

    /// Spec 0009 A4: ten snapshots leave the user's repository exactly as it
    /// was, and the same ten are listed outside any repository.
    #[test]
    fn ten_snapshots_do_not_touch_the_users_repository() {
        for inside_a_repository in [true, false] {
            let scratch = tempfile::tempdir().unwrap();
            let project = scratch.path().join("design");
            write(&project, "fanta.json", "{}");
            write(&project, "pages/home/page.svg", "<svg/>");
            let state = |repository: &Path| {
                (
                    user_git(repository, &["status", "--porcelain"]),
                    user_git(repository, &["rev-parse", "HEAD"]),
                    user_git(repository, &["branch", "-a"]),
                    user_git(repository, &["for-each-ref"]),
                    fs::read(repository.join(".git/index")).unwrap(),
                )
            };
            if inside_a_repository {
                user_git(scratch.path(), &["init", "--quiet"]);
                user_git(scratch.path(), &["add", "-A"]);
                user_git(scratch.path(), &["commit", "--quiet", "-m", "start"]);
                // The first snapshot creates the ignored `.fanta` directory;
                // nothing after it may show.
                History::new(&project).record(SnapshotReason::AfterSave).unwrap();
            }
            let before = inside_a_repository.then(|| state(scratch.path()));
            let already = History::new(&project).list().unwrap().len();
            for save in 0..10 {
                write(&project, "pages/home/page.svg", &format!("<svg data-save=\"{save}\"/>"));
                History::new(&project).record(SnapshotReason::AfterSave).unwrap();
            }
            assert_eq!(History::new(&project).list().unwrap().len(), already + 10);
            if let Some(before) = before {
                let mut after = state(scratch.path());
                // The page changed on disk, which the user's status shows;
                // the snapshots themselves must add nothing to it.
                assert_eq!(before.0, "");
                assert_eq!(after.0, " M design/pages/home/page.svg\n");
                after.0.clear();
                assert_eq!(after, before);
            }
        }
    }

    /// Spec 0009 R7: a restore gives back the exact bytes, removes what the
    /// snapshot did not have, and can itself be undone.
    #[test]
    fn a_restore_is_byte_identical_and_can_be_undone() {
        let scratch = tempfile::tempdir().unwrap();
        let project = scratch.path();
        write(project, "fanta.json", "{}");
        write(project, "pages/home/page.svg", "<<<<<<< HEAD\r\n<svg/>\r\n");
        // Neither an ignore rule nor a line-ending rule may change a snapshot.
        write(project, ".gitignore", "previews/\n");
        write(project, ".gitattributes", "* text=auto eol=lf\n");
        write(project, "previews/home.png", "not really a png");
        let hostile = files(project);
        let history = History::new(project);
        let taken = history.record(SnapshotReason::BeforeSave).unwrap();

        write(project, "pages/home/page.svg", "<svg/>\n");
        write(project, "pages/extra/page.svg", "<svg/>\n");
        fs::remove_file(project.join("previews/home.png")).unwrap();
        let saved = files(project);
        assert_ne!(saved, hostile);

        let undo = history.restore(&taken.id).unwrap();
        assert_eq!(files(project), hostile);
        assert!(!project.join("pages/extra").exists());
        assert_eq!(undo.reason, SnapshotReason::BeforeRestore);

        history.restore(&undo.id).unwrap();
        assert_eq!(files(project), saved);
        assert!(history.restore("999").is_err());
        assert!(history.restore("../HEAD").is_err());
    }

    /// Spec 0009 R6: everything for 30 days, then one per day.
    #[test]
    fn pruning_keeps_thirty_days_then_the_last_of_each_day() {
        let scratch = tempfile::tempdir().unwrap();
        write(scratch.path(), "fanta.json", "{}");
        let history = History::new(scratch.path());
        let day = Duration::from_secs(24 * 60 * 60);
        // Midnight UTC, so "the same day" below does not depend on the clock.
        let now = UNIX_EPOCH + day * 20_000;
        let hour = Duration::from_secs(60 * 60);
        let mut last_of_old_day = None;
        for (days_ago, hours) in [(40, 1), (40, 2), (40, 3), (35, 1), (1, 1), (1, 2)] {
            let taken = history
                .record_at(SnapshotReason::AfterSave, now - day * days_ago + hour * hours)
                .unwrap();
            if (days_ago, hours) == (40, 3) {
                last_of_old_day = Some(taken.id);
            }
        }
        history.prune(now).unwrap();
        let kept = history.list().unwrap();
        assert_eq!(kept.len(), 4);
        assert!(kept.iter().any(|snapshot| Some(&snapshot.id) == last_of_old_day.as_ref()));
        // What was kept still restores.
        history.restore(&kept[3].id).unwrap();
    }

    #[test]
    fn two_snapshots_in_the_same_millisecond_are_both_kept() {
        let scratch = tempfile::tempdir().unwrap();
        write(scratch.path(), "fanta.json", "{}");
        let history = History::new(scratch.path());
        let time = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let first = history.record_at(SnapshotReason::BeforeSave, time).unwrap();
        let second = history.record_at(SnapshotReason::AfterSave, time).unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(history.list().unwrap(), [second, first]);
    }

    /// A store as a hostile project would ship it. Every command it names
    /// only creates a file in `markers`, so an empty `markers` proves that
    /// none ran.
    #[cfg(unix)]
    fn plant_hostile_store(store: &Path, markers: &Path) {
        use std::os::unix::fs::PermissionsExt as _;
        fs::create_dir_all(markers).unwrap();
        let touch = |name: &str| format!("touch '{}/{name}'", markers.display());
        for folder in ["objects", "refs", "info"] {
            fs::create_dir_all(store.join(folder)).unwrap();
        }
        fs::write(store.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(
            store.join("config"),
            format!(
                "[core]\n\trepositoryformatversion = 0\n\tbare = true\n\
                 \tfsmonitor = \"{}\"\n\thooksPath = {}\n\
                 [filter \"planted\"]\n\tclean = \"{}; cat\"\n\tsmudge = \"{}; cat\"\n\
                 [diff]\n\texternal = \"{}\"\n\
                 [diff \"planted\"]\n\tcommand = \"{}\"\n\ttextconv = \"{}\"\n\
                 [merge \"planted\"]\n\tdriver = \"{}\"\n",
                touch("fsmonitor"),
                store.join("planted-hooks").display(),
                touch("filter-clean"),
                touch("filter-smudge"),
                touch("diff-external"),
                touch("diff-command"),
                touch("diff-textconv"),
                touch("merge-driver"),
            ),
        )
        .unwrap();
        fs::write(
            store.join("info/attributes"),
            "* filter=planted diff=planted merge=planted\n",
        )
        .unwrap();
        for folder in ["hooks", "planted-hooks"] {
            for hook in [
                "post-index-change",
                "reference-transaction",
                "pre-auto-gc",
                "post-checkout",
            ] {
                let path = store.join(folder).join(hook);
                write(store, &format!("{folder}/{hook}"), &format!("#!/bin/sh\n{}\n", touch(hook)));
                fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
    }

    #[cfg(unix)]
    fn ran(markers: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(markers)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// The store directory arrives with the project. One that names
    /// commands in its config, attributes and hooks must run none of them,
    /// and snapshots must still work.
    #[cfg(unix)]
    #[test]
    fn a_store_shipped_with_the_project_runs_nothing() {
        let scratch = tempfile::tempdir().unwrap();
        let project = scratch.path().join("design");
        let markers = scratch.path().join("markers");
        write(&project, "fanta.json", "{}");
        write(&project, "pages/home/page.svg", "<svg/>\r\n");
        plant_hostile_store(&project.join(GIT_DIR), &markers);
        let before = files(&project);
        let history = History::new(&project);

        assert_eq!(history.before_save(), None);
        write(&project, "pages/home/page.svg", "<svg data-save=\"1\"/>");
        assert_eq!(history.after_save(), None);
        let snapshots = history.list().unwrap();
        assert_eq!(snapshots.len(), 2);
        history.restore(&snapshots[1].id).unwrap();
        assert_eq!(files(&project), before);
        // Far enough ahead that everything but one snapshot a day is
        // dropped, which is what runs `gc`.
        history.prune(SystemTime::now() + KEEP_EVERYTHING_FOR * 2).unwrap();
        assert_eq!(history.list().unwrap().len(), 1);

        assert_eq!(ran(&markers), Vec::<String>::new());
        let store = project.join(GIT_DIR);
        assert_eq!(fs::read_to_string(store.join("config")).unwrap(), STORE_CONFIG);
        assert_eq!(fs::read_to_string(store.join("info/attributes")).unwrap(), STORE_ATTRIBUTES);
    }

    /// A `.gitattributes` in the project may name any driver: the store's
    /// config defines none, even when the store came without its own
    /// attributes file.
    #[cfg(unix)]
    #[test]
    fn drivers_named_by_the_projects_gitattributes_do_not_exist() {
        let scratch = tempfile::tempdir().unwrap();
        let project = scratch.path().join("design");
        let markers = scratch.path().join("markers");
        write(&project, "fanta.json", "{}");
        write(&project, "pages/home/page.svg", "<svg/>\r\n");
        write(&project, ".gitattributes", "* filter=planted diff=planted merge=planted\n");
        plant_hostile_store(&project.join(GIT_DIR), &markers);
        fs::remove_file(project.join(GIT_DIR).join("info/attributes")).unwrap();
        let before = files(&project);
        let history = History::new(&project);

        let taken = history.record(SnapshotReason::BeforeSave).unwrap();
        write(&project, "pages/home/page.svg", "<svg/>");
        history.restore(&taken.id).unwrap();
        assert_eq!(files(&project), before);
        assert_eq!(ran(&markers), Vec::<String>::new());
    }

    /// A link in place of the store, or of a file git reads from it, would
    /// make git read (and Fanta overwrite) files the project chose. Such a
    /// store is not used at all; the save goes on without a snapshot.
    #[cfg(unix)]
    #[test]
    fn a_store_that_is_or_holds_a_link_is_refused() {
        use std::os::unix::fs::symlink;
        for case in ["private folder", "store", "config", "hooks", "attributes", "gitdir file"] {
            let scratch = tempfile::tempdir().unwrap();
            let project = scratch.path().join("design");
            let markers = scratch.path().join("markers");
            let elsewhere = scratch.path().join("elsewhere");
            write(&project, "fanta.json", "{}");
            plant_hostile_store(&elsewhere.join("history.git"), &markers);
            let planted = files(&elsewhere);
            let store = project.join(GIT_DIR);
            let outside = elsewhere.join("history.git");
            if case == "private folder" {
                symlink(&elsewhere, project.join(HISTORY_DIR)).unwrap();
            } else if case == "store" {
                fs::create_dir_all(project.join(HISTORY_DIR)).unwrap();
                symlink(&outside, &store).unwrap();
            } else if case == "gitdir file" {
                write(&project, GIT_DIR, &format!("gitdir: {}\n", outside.display()));
            } else {
                for folder in ["objects", "refs", "info"] {
                    fs::create_dir_all(store.join(folder)).unwrap();
                }
                fs::write(store.join("HEAD"), STORE_HEAD).unwrap();
                let name = if case == "attributes" { "info/attributes" } else { case };
                symlink(outside.join(name), store.join(name)).unwrap();
            }
            let history = History::new(&project);

            assert!(history.record(SnapshotReason::AfterSave).is_err(), "{case}");
            let finding = history.before_save().expect(case);
            assert_eq!(finding.code, SNAPSHOT_UNAVAILABLE, "{case}");
            assert!(history.after_save().is_some(), "{case}");
            assert_eq!(ran(&markers), Vec::<String>::new(), "{case}");
            assert_eq!(files(&elsewhere), planted, "{case}: the link's target was written");
        }
    }

    /// Files inside a git directory that send git to another directory
    /// without any config are removed before git runs.
    #[cfg(unix)]
    #[test]
    fn redirections_inside_the_store_are_removed() {
        let scratch = tempfile::tempdir().unwrap();
        let project = scratch.path().join("design");
        let markers = scratch.path().join("markers");
        let elsewhere = scratch.path().join("elsewhere.git");
        write(&project, "fanta.json", "{}");
        plant_hostile_store(&elsewhere, &markers);
        let planted = files(&elsewhere);
        let store = project.join(GIT_DIR);
        let redirections = [
            ("commondir", format!("{}\n", elsewhere.display())),
            ("objects/info/alternates", format!("{}/objects\n", elsewhere.display())),
            ("info/grafts", String::new()),
        ];
        fs::create_dir_all(&store).unwrap();
        fs::write(store.join("HEAD"), STORE_HEAD).unwrap();
        for (name, content) in &redirections {
            write(&store, name, content);
        }
        let history = History::new(&project);

        let taken = history.record(SnapshotReason::AfterSave).unwrap();
        assert_eq!(history.list().unwrap(), [taken]);
        for (name, _) in &redirections {
            assert!(!store.join(name).exists(), "{name}");
        }
        assert_eq!(ran(&markers), Vec::<String>::new());
        assert_eq!(files(&elsewhere), planted);
    }

    /// Git runs with the store's own settings only: nothing inherited may
    /// redirect it or name a program, and it never asks at a terminal.
    #[test]
    fn git_runs_in_a_controlled_environment() {
        for name in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_CONFIG",
            "GIT_CONFIG_COUNT",
            "GIT_CONFIG_KEY_0",
            "GIT_CONFIG_VALUE_0",
            "GIT_CONFIG_PARAMETERS",
            "GIT_CONFIG_SYSTEM",
            "GIT_EXTERNAL_DIFF",
            "GIT_SSH",
            "GIT_SSH_COMMAND",
            "GIT_ASKPASS",
            "GIT_EDITOR",
            "GIT_PAGER",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "git_proxy_command",
            "SSH_ASKPASS",
            "EDITOR",
            "VISUAL",
            "PAGER",
        ] {
            assert!(is_inherited_git_setting(name), "{name}");
        }
        for name in ["PATH", "HOME", "GIT_EXEC_PATH", "LANG", "TMPDIR", "DIGIT_X"] {
            assert!(!is_inherited_git_setting(name), "{name}");
        }

        let scratch = tempfile::tempdir().unwrap();
        let command = History::new(scratch.path()).git().unwrap();
        let set = |name: &str| {
            command
                .get_envs()
                .find(|(key, _)| *key == std::ffi::OsStr::new(name))
                .and_then(|(_, value)| value)
                .map(|value| value.to_string_lossy().into_owned())
        };
        assert_eq!(set("GIT_CONFIG_NOSYSTEM").as_deref(), Some("1"));
        assert_eq!(set("GIT_CONFIG_GLOBAL").as_deref(), Some(NULL_DEVICE));
        assert_eq!(set("GIT_ATTR_NOSYSTEM").as_deref(), Some("1"));
        assert_eq!(set("GIT_TERMINAL_PROMPT").as_deref(), Some("0"));
        let store = scratch.path().join(GIT_DIR);
        assert_eq!(set("GIT_DIR").as_deref(), Some(store.to_string_lossy().as_ref()));
        let arguments: Vec<_> = command.get_args().map(|argument| argument.to_string_lossy()).collect();
        assert!(arguments.contains(&format!("core.hooksPath={NULL_DEVICE}").into()));
        assert!(arguments.contains(&"core.fsmonitor=false".into()));
    }

    #[test]
    fn a_project_without_snapshots_lists_none() {
        let scratch = tempfile::tempdir().unwrap();
        assert_eq!(History::new(scratch.path()).list().unwrap(), Vec::new());
        assert!(!scratch.path().join(HISTORY_DIR).exists());
    }
}
