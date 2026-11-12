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

/// The directory at the project root that holds Fanta's private state. Every
/// walker of a project directory skips it.
pub const HISTORY_DIR: &str = ".fanta";
const GIT_DIR: &str = ".fanta/history.git";
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
            dir: dir.into(),
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

    fn record_at(&self, reason: SnapshotReason, time: SystemTime) -> Result<Snapshot> {
        self.ensure_repository()?;
        // `-f`: the project's own `.gitignore` must not thin the snapshot, or
        // a restore would not give back what the save changed.
        self.run(self.git().args(["add", "-A", "-f", "--", ".", ":(exclude).fanta"]))?;
        let tree = self.run(self.git().arg("write-tree"))?;
        let since_epoch = time.duration_since(UNIX_EPOCH).unwrap_or_default();
        let date = format!("@{} +0000", since_epoch.as_secs());
        let commit = self.run(
            self.git()
                .env("GIT_AUTHOR_DATE", &date)
                .env("GIT_COMMITTER_DATE", &date)
                .args(["commit-tree", "--no-gpg-sign", "-m", reason.as_str()])
                .arg(tree.trim()),
        )?;
        let taken = since_epoch.as_millis();
        // An empty old value makes `update-ref` fail when the ref exists, so
        // two snapshots in one millisecond cannot replace each other.
        for millis in taken..taken + 1000 {
            let id = format!("{millis:013}");
            let name = format!("{SNAPSHOT_REFS}{id}");
            if self.succeeds(self.git().args(["update-ref", &name, commit.trim(), ""]))? {
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
        let refs = self.run(self.git().args([
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
            || !self.succeeds(self.git().args(["rev-parse", "--verify", "--quiet", &tree]))?
        {
            return Err(FormatError::Snapshot(format!("no snapshot {id}")));
        }
        let before = self.record(SnapshotReason::BeforeRestore)?;
        let gone = self.run(self.git().args([
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
        self.run(self.git().args(["read-tree", &target]))?;
        self.run(self.git().args(["checkout-index", "-a", "-f"]))?;
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
                self.git()
                    .args(["update-ref", "-d", &format!("{SNAPSHOT_REFS}{}", snapshot.id)]),
            )?;
            dropped = true;
        }
        if dropped {
            self.run(self.git().args(["gc", "--auto", "--quiet"]))?;
        }
        Ok(())
    }

    fn ensure_repository(&self) -> Result<()> {
        let git_dir = self.dir.join(GIT_DIR);
        if git_dir.join("HEAD").is_file() {
            return Ok(());
        }
        let private = self.dir.join(HISTORY_DIR);
        fs::create_dir_all(&private)?;
        // The directory ignores itself, so the user's git never sees it and
        // their own `.gitignore` is not edited.
        fs::write(private.join(".gitignore"), "*\n")?;
        self.run(
            Command::new(&self.git)
                .args(["init", "--quiet", "--bare", "--template="])
                .arg(&git_dir),
        )?;
        fs::create_dir_all(git_dir.join("info"))?;
        // Outranks any `.gitattributes` in the project: snapshots hold the
        // bytes on disk, with no line-ending or filter conversion.
        fs::write(git_dir.join("info/attributes"), "* -text -filter -ident\n")?;
        Ok(())
    }

    /// A git command against the private repository only. The user's and the
    /// system's configuration are shut out: signing, hooks, filters and
    /// line-ending settings must not shape or block a snapshot.
    fn git(&self) -> Command {
        let mut command = Command::new(&self.git);
        command
            .current_dir(&self.dir)
            .env("GIT_DIR", self.dir.join(GIT_DIR))
            .env("GIT_WORK_TREE", &self.dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", if cfg!(windows) { "NUL" } else { "/dev/null" })
            .env("GIT_AUTHOR_NAME", "Fanta")
            .env("GIT_AUTHOR_EMAIL", "snapshots@fanta.invalid")
            .env("GIT_COMMITTER_NAME", "Fanta")
            .env("GIT_COMMITTER_EMAIL", "snapshots@fanta.invalid")
            .args(["-c", "core.autocrlf=false", "-c", "core.fsmonitor=false"]);
        // Set when Fanta is started from inside a git hook or alias.
        for inherited in [
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_COMMON_DIR",
            "GIT_NAMESPACE",
            "GIT_PREFIX",
        ] {
            command.env_remove(inherited);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            // CREATE_NO_WINDOW: no console flashes up under the editor.
            command.creation_flags(0x0800_0000);
        }
        command
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

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

    #[test]
    fn a_project_without_snapshots_lists_none() {
        let scratch = tempfile::tempdir().unwrap();
        assert_eq!(History::new(scratch.path()).list().unwrap(), Vec::new());
        assert!(!scratch.path().join(HISTORY_DIR).exists());
    }
}
