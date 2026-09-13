use anyhow::{Context as _, Result, bail, ensure};
use db::{kvp::KeyValueStore, sqlez::connection::Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::HashSet, io::Write};
use url::Url;
use uuid::Uuid;

const VERSION: u32 = 1;
const STORAGE_KEY: &str = "journal";
const MAX_FINISHED: usize = 12;
const MAX_UNRESOLVED: usize = 8;
const MAX_UNFINISHED: usize = 32;
const MAX_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct JournalScope {
    endpoint: String,
    user_id: Uuid,
    org_id: Uuid,
}

impl JournalScope {
    pub(super) fn new(endpoint: &str, user_id: &str, org_id: &str) -> Result<Self> {
        let endpoint = normalize_endpoint(endpoint)?;
        let user_id = Uuid::parse_str(user_id).context("The generation account ID is invalid.")?;
        let org_id =
            Uuid::parse_str(org_id).context("The generation organization ID is invalid.")?;
        ensure!(
            !user_id.is_nil() && !org_id.is_nil(),
            "Generation history requires an authenticated account and organization."
        );
        Ok(Self {
            endpoint,
            user_id,
            org_id,
        })
    }

    pub(super) fn endpoint(&self) -> &str {
        &self.endpoint
    }

    fn namespace(&self) -> Result<String> {
        let identity = serde_json::to_vec(self)?;
        Ok(format!(
            "fanta.generation-journal.{:x}",
            Sha256::digest(identity)
        ))
    }
}

pub(super) fn normalize_endpoint(endpoint: &str) -> Result<String> {
    let mut endpoint = Url::parse(endpoint).context("The generation server URL is invalid.")?;
    ensure!(
        matches!(endpoint.scheme(), "http" | "https") && endpoint.host_str().is_some(),
        "Generation history requires an HTTP or HTTPS server URL."
    );
    ensure!(
        endpoint.username().is_empty()
            && endpoint.password().is_none()
            && endpoint.query().is_none()
            && endpoint.fragment().is_none(),
        "The generation server URL cannot contain credentials, a query, or a fragment."
    );
    let path = endpoint.path().trim_end_matches('/').to_owned();
    endpoint.set_path(&path);
    Ok(endpoint.as_str().trim_end_matches('/').to_owned())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct SavedSource {
    pub reference: Value,
    pub name: String,
    pub preview_png: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct SavedSubmission {
    pub key: String,
    pub request: Value,
    pub model: String,
    pub prompt: String,
    pub source: Option<SavedSource>,
    pub mode: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) enum SavedDispatch {
    #[default]
    Unknown,
    Unclaimed {
        retry_requires_input: bool,
    },
    Claimed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) enum SavedRunResult {
    Generation {
        id: String,
        #[serde(default)]
        finished: bool,
        #[serde(default)]
        dispatch: SavedDispatch,
    },
    VectorMessage {
        message_id: Option<String>,
        svg: String,
    },
}

impl SavedRunResult {
    pub(super) fn requires_input(&self) -> bool {
        matches!(
            self,
            Self::Generation {
                finished: false,
                dispatch: SavedDispatch::Unclaimed {
                    retry_requires_input: true
                },
                ..
            }
        )
    }

    fn releases_request(&self) -> bool {
        result_finished(self)
            || matches!(
                self,
                Self::Generation {
                    dispatch: SavedDispatch::Claimed,
                    ..
                }
            )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct JournalRecord {
    pub key: String,
    pub request: Option<Value>,
    pub model: String,
    pub prompt: String,
    pub source: Option<SavedSource>,
    pub mode: String,
    pub result: Option<SavedRunResult>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct JournalSnapshot {
    pub records: Vec<JournalRecord>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredRecord {
    record: JournalRecord,
    // Keep the fingerprint after a claim releases the body so an old UI retry
    // cannot reuse the key for a different payload.
    submission_fingerprint: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredJournal {
    version: u32,
    records: Vec<StoredRecord>,
}

impl Default for StoredJournal {
    fn default() -> Self {
        Self {
            version: VERSION,
            records: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub(super) struct GenerationJournal {
    store: KeyValueStore,
    namespace: String,
}

impl GenerationJournal {
    pub(super) fn new(store: KeyValueStore, scope: JournalScope) -> Result<Self> {
        ensure!(
            store.persistent(),
            "Generation recovery needs persistent local storage. No request was submitted."
        );
        let canonical = JournalScope::new(
            scope.endpoint(),
            &scope.user_id.to_string(),
            &scope.org_id.to_string(),
        )?;
        Ok(Self {
            store,
            namespace: canonical.namespace()?,
        })
    }

    pub(super) async fn load(&self) -> Result<JournalSnapshot> {
        let namespace = self.namespace.clone();
        self.store
            .write(move |connection| {
                connection.with_savepoint("fanta_generation_journal", || {
                    Ok(JournalSnapshot {
                        records: read_journal(connection, &namespace)?
                            .records
                            .into_iter()
                            .map(|stored| stored.record)
                            .collect(),
                    })
                })
            })
            .await
            .context("Could not load local generation recovery. Existing history was preserved.")
    }

    pub(super) async fn prepare(&self, submission: SavedSubmission) -> Result<JournalRecord> {
        self.change(move |journal| {
            validate_submission(&submission)?;
            let fingerprint = request_fingerprint(&submission.request)?;
            if let Some(stored) = journal.records.iter().find(|stored| stored.record.key == submission.key) {
                ensure!(
                    stored.submission_fingerprint == fingerprint,
                    "This generation recovery key already belongs to a different submission."
                );
                return Ok(stored.record.clone());
            }
            ensure!(
                journal.records.iter().filter(|stored| stored.record.result.is_none()).count() < MAX_UNRESOLVED,
                "Eight generations still need recovery. Resolve them before submitting another request."
            );
            ensure!(unfinished_count(journal) < MAX_UNFINISHED,
                "Thirty-two generations are unfinished. Check their status before submitting another request.");
            let record = JournalRecord {
                key: submission.key,
                request: Some(submission.request),
                model: submission.model,
                prompt: submission.prompt,
                source: submission.source,
                mode: submission.mode,
                result: None,
            };
            journal.records.insert(0, StoredRecord { record: record.clone(), submission_fingerprint: fingerprint });
            Ok(record)
        }).await
    }

    pub(super) async fn accept(&self, key: &str, result: SavedRunResult) -> Result<JournalRecord> {
        let key = key.to_owned();
        self.change(move |journal| {
            validate_result(&result)?;
            let stored = journal
                .records
                .iter_mut()
                .find(|stored| stored.record.key == key)
                .context("The generation has no saved submission to resolve.")?;
            accept_result(&mut stored.record, result)?;
            let record = stored.record.clone();
            prune_finished(journal);
            Ok(record)
        })
        .await
    }

    pub(super) async fn record_completed(
        &self,
        submission: SavedSubmission,
        result: SavedRunResult,
    ) -> Result<()> {
        self.change(move |journal| {
            validate_submission(&submission)?;
            validate_result(&result)?;
            ensure!(
                result_finished(&result),
                "Only completed results can be recorded without a prepared submission."
            );
            let fingerprint = request_fingerprint(&submission.request)?;
            if let Some(stored) = journal
                .records
                .iter_mut()
                .find(|stored| stored.record.key == submission.key)
            {
                ensure!(
                    stored.submission_fingerprint == fingerprint,
                    "This generation recovery key already belongs to a different submission."
                );
                accept_result(&mut stored.record, result)?;
            } else {
                journal.records.insert(
                    0,
                    StoredRecord {
                        record: JournalRecord {
                            key: submission.key,
                            request: None,
                            model: submission.model,
                            prompt: submission.prompt,
                            source: submission.source,
                            mode: submission.mode,
                            result: Some(result),
                        },
                        submission_fingerprint: fingerprint,
                    },
                );
            }
            prune_finished(journal);
            Ok(())
        })
        .await
    }

    pub(super) async fn observe_generation(
        &self,
        result: SavedRunResult,
    ) -> Result<(bool, Option<SavedRunResult>)> {
        let SavedRunResult::Generation { id, .. } = &result else {
            bail!("A generation status cannot replace vector artwork.");
        };
        validate_result(&result)?;
        if result_finished(&result) {
            self.mark_finished(id).await?;
            return Ok((true, None));
        }
        let id = id.clone();
        let namespace = self.namespace.clone();
        self.store.write(move |connection| {
            connection.with_savepoint("fanta_generation_journal", || {
                let mut journal = read_journal(connection, &namespace)?;
                let mut changed = false;
                let mut observed = None;
                for stored in &mut journal.records {
                    if matches!(&stored.record.result, Some(SavedRunResult::Generation { id: stored_id, .. }) if *stored_id == id) {
                        let previous = stored.record.result.clone();
                        let retained_request = stored.record.request.is_some();
                        accept_result(&mut stored.record, result.clone())?;
                        changed |= previous != stored.record.result
                            || retained_request != stored.record.request.is_some();
                        observed = stored.record.result.clone();
                    }
                }
                if changed {
                    write_journal(connection, &namespace, &journal)?;
                }
                Ok((changed, observed))
            })
        }).await.context("Could not update saved generation recovery. The previous record was preserved.")
    }

    pub(super) async fn mark_finished(&self, id: &str) -> Result<()> {
        ensure!(!id.trim().is_empty(), "The generation ID is missing.");
        let id = id.to_owned();
        self.change(move |journal| {
            for stored in &mut journal.records {
                if let Some(SavedRunResult::Generation {
                    id: stored_id,
                    finished,
                    ..
                }) = &mut stored.record.result
                {
                    if *stored_id == id {
                        *finished = true;
                        stored.record.request = None;
                    }
                }
            }
            prune_finished(journal);
            Ok(())
        })
        .await
    }

    pub(super) async fn reject(&self, key: &str) -> Result<()> {
        let key = key.to_owned();
        self.change(move |journal| {
            if let Some(index) = journal
                .records
                .iter()
                .position(|stored| stored.record.key == key)
            {
                ensure!(
                    journal
                        .records
                        .get(index)
                        .is_some_and(|stored| stored.record.result.is_none()),
                    "An accepted generation cannot be removed as a rejected submission."
                );
                journal.records.remove(index);
            }
            Ok(())
        })
        .await
    }

    async fn change<T: Send + Sync + 'static>(
        &self,
        change: impl FnOnce(&mut StoredJournal) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let namespace = self.namespace.clone();
        self.store
            .write(move |connection| {
                connection.with_savepoint("fanta_generation_journal", || {
                    let mut journal = read_journal(connection, &namespace)?;
                    let result = change(&mut journal)?;
                    write_journal(connection, &namespace, &journal)?;
                    Ok(result)
                })
            })
            .await
            .context(
                "Could not save local generation recovery. No new request should be submitted.",
            )
    }
}

fn write_journal(connection: &Connection, namespace: &str, journal: &StoredJournal) -> Result<()> {
    validate_journal(journal)?;
    let encoded = String::from_utf8(bounded_json(journal)?)?;
    connection.exec_bound::<(&str, &str, &str)>(
        "INSERT OR REPLACE INTO scoped_kv_store(namespace, key, value) VALUES (?, ?, ?)",
    )?((namespace, STORAGE_KEY, &encoded))?;
    Ok(())
}

fn result_finished(result: &SavedRunResult) -> bool {
    match result {
        SavedRunResult::Generation { finished, .. } => *finished,
        SavedRunResult::VectorMessage { .. } => true,
    }
}

fn record_finished(record: &JournalRecord) -> bool {
    record.result.as_ref().is_some_and(result_finished)
}

fn unfinished_count(journal: &StoredJournal) -> usize {
    journal
        .records
        .iter()
        .filter(|stored| !record_finished(&stored.record))
        .count()
}

fn merge_result(previous: &mut SavedRunResult, result: SavedRunResult) -> Result<()> {
    match (previous, result) {
        (
            SavedRunResult::Generation {
                id: previous_id,
                finished: previous_finished,
                dispatch: previous_dispatch,
            },
            SavedRunResult::Generation {
                id,
                finished,
                dispatch,
            },
        ) => {
            ensure!(
                *previous_id == id,
                "This generation already has a different saved result."
            );
            *previous_finished |= finished;
            if *previous_dispatch != SavedDispatch::Claimed {
                *previous_dispatch = dispatch;
            }
        }
        (previous, result) => ensure!(
            *previous == result,
            "This generation already has a different saved result."
        ),
    }
    Ok(())
}

fn accept_result(record: &mut JournalRecord, result: SavedRunResult) -> Result<()> {
    if let Some(previous) = &mut record.result {
        merge_result(previous, result)?;
    } else {
        record.result = Some(result);
    }
    if record
        .result
        .as_ref()
        .is_some_and(SavedRunResult::releases_request)
    {
        record.request = None;
    }
    Ok(())
}

fn prune_finished(journal: &mut StoredJournal) {
    let mut finished = 0;
    journal.records.retain(|stored| {
        if !record_finished(&stored.record) {
            return true;
        }
        finished += 1;
        finished <= MAX_FINISHED
    });
}

fn read_journal(connection: &Connection, namespace: &str) -> Result<StoredJournal> {
    let size = connection.select_row_bound::<(&str, &str), i64>(
        "SELECT length(CAST(value AS BLOB)) FROM scoped_kv_store WHERE namespace = ? AND key = ?",
    )?((namespace, STORAGE_KEY))?;
    match size {
        None => return Ok(StoredJournal::default()),
        Some(size) => ensure!(
            (0..=MAX_BYTES as i64).contains(&size),
            "Generation history exceeds the 64 MiB storage limit."
        ),
    }
    let encoded = connection.select_row_bound::<(&str, &str), String>(
        "SELECT value FROM scoped_kv_store WHERE namespace = ? AND key = ?",
    )?((namespace, STORAGE_KEY))?
    .context("Generation history changed while it was being read.")?;
    let journal: StoredJournal = serde_json::from_str(&encoded)
        .context("Generation history is unreadable. It has not been reset.")?;
    validate_journal(&journal)?;
    Ok(journal)
}

fn validate_journal(journal: &StoredJournal) -> Result<()> {
    ensure!(
        journal.version == VERSION,
        "This generation history version is not supported. It has not been reset."
    );
    let unresolved = journal
        .records
        .iter()
        .filter(|stored| stored.record.result.is_none())
        .count();
    let unfinished = unfinished_count(journal);
    ensure!(
        unresolved <= MAX_UNRESOLVED
            && unfinished <= MAX_UNFINISHED
            && journal.records.len() - unfinished <= MAX_FINISHED,
        "Generation history has too many records. It has not been reset."
    );
    let mut keys = HashSet::new();
    for stored in &journal.records {
        let record = &stored.record;
        ensure!(
            keys.insert(&record.key),
            "Generation history contains duplicate recovery keys."
        );
        validate_fields(
            &record.key,
            &record.model,
            &record.mode,
            record.source.as_ref(),
        )?;
        ensure!(
            stored.submission_fingerprint.len() == 64
                && stored
                    .submission_fingerprint
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit()),
            "Generation history has an invalid submission fingerprint."
        );
        match (&record.request, &record.result) {
            (Some(request), result) => {
                if let Some(result) = result {
                    validate_result(result)?;
                    ensure!(
                        !result.releases_request(),
                        "A claimed or finished generation still contains a recovery request."
                    );
                }
                let submission = SavedSubmission {
                    key: record.key.clone(),
                    request: request.clone(),
                    model: record.model.clone(),
                    prompt: record.prompt.clone(),
                    source: record.source.clone(),
                    mode: record.mode.clone(),
                };
                validate_submission(&submission)?;
                ensure!(
                    request_fingerprint(&submission.request)? == stored.submission_fingerprint,
                    "Generation history contains a changed recovery payload."
                );
            }
            (None, Some(result)) => validate_result(result)?,
            _ => bail!("Generation history has an invalid recovery state. It has not been reset."),
        }
    }
    Ok(())
}

fn validate_fields(key: &str, model: &str, mode: &str, source: Option<&SavedSource>) -> Result<()> {
    ensure!(
        !key.trim().is_empty() && key.len() <= 256 && !key.chars().any(char::is_control),
        "The generation recovery key is invalid."
    );
    ensure!(
        !model.trim().is_empty() && !mode.trim().is_empty(),
        "The saved generation model or mode is missing."
    );
    if let Some(source) = source {
        ensure!(
            source.reference.is_object() && source.width > 0 && source.height > 0,
            "The saved generation source is invalid."
        );
        reject_transport_fields(&source.reference)?;
    }
    Ok(())
}

fn reject_transport_fields(value: &Value) -> Result<()> {
    if let Some(object) = value.as_object() {
        ensure!(
            !object.keys().any(|key| matches!(
                key.to_ascii_lowercase().as_str(),
                "authorization" | "headers" | "access_token" | "api_key" | "x-api-key"
            )),
            "Generation history accepts request bodies and asset references, not authentication headers or credentials."
        );
    }
    Ok(())
}

fn validate_submission(submission: &SavedSubmission) -> Result<()> {
    validate_fields(
        &submission.key,
        &submission.model,
        &submission.mode,
        submission.source.as_ref(),
    )?;
    ensure!(
        submission.request.is_object(),
        "The saved generation request must be an object."
    );
    reject_transport_fields(&submission.request)?;
    bounded_json(submission)?;
    Ok(())
}

fn validate_result(result: &SavedRunResult) -> Result<()> {
    match result {
        SavedRunResult::Generation { id, .. } => {
            ensure!(!id.trim().is_empty(), "The saved generation ID is missing.")
        }
        SavedRunResult::VectorMessage { message_id, svg } => {
            ensure!(!svg.trim().is_empty(), "The saved vector result is empty.");
            ensure!(
                message_id.as_ref().is_none_or(|id| !id.trim().is_empty()),
                "The saved message ID is invalid."
            );
        }
    }
    bounded_json(result)?;
    Ok(())
}

fn request_fingerprint(request: &Value) -> Result<String> {
    let mut canonical = request.clone();
    canonical.sort_all_objects();
    Ok(format!("{:x}", Sha256::digest(bounded_json(&canonical)?)))
}

fn bounded_json(value: &impl Serialize) -> Result<Vec<u8>> {
    struct LimitedBuffer(Vec<u8>);
    impl Write for LimitedBuffer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > MAX_BYTES.saturating_sub(self.0.len()) {
                return Err(std::io::Error::other(
                    "Generation history exceeds the 64 MiB storage limit.",
                ));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut buffer = LimitedBuffer(Vec::new());
    serde_json::to_writer(&mut buffer, value)
        .context("The generation history could not be serialized within its storage limit.")?;
    Ok(buffer.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use db::sqlez::thread_safe_connection::{ThreadSafeConnection, locking_queue};
    use serde_json::json;
    use std::path::Path;

    const USER: &str = "a1111111-1111-4111-8111-111111111111";
    const ORGANIZATION: &str = "22222222-2222-4222-8222-222222222222";

    fn scope() -> JournalScope {
        JournalScope::new("https://api.example.test", USER, ORGANIZATION).expect("scope")
    }

    async fn store(path: &Path) -> Result<KeyValueStore> {
        let connection = ThreadSafeConnection::builder::<KeyValueStore>(
            path.to_str().context("test database path")?,
            true,
        )
        .with_write_queue_constructor(locking_queue())
        .build()
        .await?;
        Ok(KeyValueStore::from_app_db(&db::AppDatabase(connection)))
    }

    fn submission(key: &str) -> SavedSubmission {
        SavedSubmission {
            key: key.into(),
            request: json!({"model":"image-test", "prompt":"red circle", "seed":"18446744073709551615"}),
            model: "image-test".into(),
            prompt: "red circle".into(),
            source: None,
            mode: "Image".into(),
        }
    }

    fn generated(id: &str) -> SavedRunResult {
        SavedRunResult::Generation {
            id: id.into(),
            finished: true,
            dispatch: SavedDispatch::Unknown,
        }
    }

    fn pending(id: &str) -> SavedRunResult {
        SavedRunResult::Generation {
            id: id.into(),
            finished: false,
            dispatch: SavedDispatch::Unknown,
        }
    }

    #[test]
    fn generation_journal_scope_canonicalizes_identity_and_rejects_credentials() -> Result<()> {
        let first = JournalScope::new("HTTPS://API.EXAMPLE.TEST:443/prefix/", USER, ORGANIZATION)?;
        let second = JournalScope::new(
            "https://api.example.test/prefix",
            &USER.to_uppercase(),
            ORGANIZATION,
        )?;
        assert_eq!(first, second);
        assert_eq!(first.endpoint(), "https://api.example.test/prefix");
        for (input, expected) in [
            (
                "https://API.EXAMPLE.TEST:443/",
                "https://api.example.test/v1/models",
            ),
            (
                "https://api.example.test",
                "https://api.example.test/v1/models",
            ),
            (
                "https://api.example.test/prefix/",
                "https://api.example.test/prefix/v1/models",
            ),
            (
                "http://localhost:4321/prefix%20name/",
                "http://localhost:4321/prefix%20name/v1/models",
            ),
        ] {
            assert_eq!(
                format!("{}/v1/models", normalize_endpoint(input)?),
                expected
            );
        }
        assert_ne!(first, scope());
        assert_ne!(
            first,
            JournalScope::new("https://api.example.test:8443/prefix", USER, ORGANIZATION)?
        );
        for endpoint in [
            "file:///tmp/data",
            "https://name:secret@api.example.test",
            "https://api.example.test?token=secret",
            "https://api.example.test#token",
        ] {
            assert!(JournalScope::new(endpoint, USER, ORGANIZATION).is_err());
        }
        assert!(JournalScope::new("https://api.example.test", "not-a-user", ORGANIZATION).is_err());
        assert!(
            JournalScope::new("https://api.example.test", USER, &Uuid::nil().to_string()).is_err()
        );
        assert_eq!(
            serde_json::from_value::<SavedRunResult>(json!({"Generation":{"id":"legacy"}}))?,
            pending("legacy")
        );
        Ok(())
    }

    #[gpui::test]
    async fn generation_journal_retains_running_jobs_and_releases_terminal_capacity() {
        async {
            let directory = tempfile::tempdir()?;
            let path = directory.path().join("history.sqlite");
            let journal = GenerationJournal::new(store(&path).await?, scope())?;
            for index in 0..32 {
                let key = format!("job-{index}");
                journal.prepare(submission(&key)).await?;
                journal.accept(&key, pending(&key)).await?;
            }
            let records = journal.load().await?.records;
            assert_eq!(
                records.len(),
                32,
                "accepted running jobs are never pruned as completed history"
            );
            assert!(
                records
                    .iter()
                    .all(|record| !record_finished(record) && record.request.is_some())
            );
            assert!(records.iter().any(|record| record.key == "job-0"));
            assert!(journal.prepare(submission("overflow")).await.is_err());
            journal.prepare(submission("job-0")).await?;
            journal.mark_finished("job-0").await?;
            journal.accept("job-0", pending("job-0")).await?;
            let first = journal.prepare(submission("job-0")).await?;
            assert_eq!(
                first.result,
                Some(generated("job-0")),
                "stale pending replies cannot undo terminal status"
            );
            journal.prepare(submission("new-unconfirmed")).await?;
            assert!(
                journal
                    .prepare(submission("another-overflow"))
                    .await
                    .is_err()
            );
            journal.reject("new-unconfirmed").await?;
            for index in 1..16 {
                journal
                    .accept(&format!("job-{index}"), generated(&format!("job-{index}")))
                    .await?;
            }
            let records = journal.load().await?.records;
            assert_eq!(
                records
                    .iter()
                    .filter(|record| record_finished(record))
                    .count(),
                12
            );
            assert_eq!(
                records
                    .iter()
                    .filter(|record| !record_finished(record))
                    .count(),
                16
            );
            assert!(records.iter().any(|record| record.key == "job-16"));
            assert!(records.iter().any(|record| record.key == "job-31"));
            assert!(!records.iter().any(|record| record.key == "job-0"));
            journal.mark_finished("job-0").await?;
            let reopened = GenerationJournal::new(store(&path).await?, scope())?;
            assert_eq!(reopened.load().await?.records, records);
            Ok::<_, anyhow::Error>(())
        }
        .await
        .expect("unfinished jobs survive pruning and terminal status only advances");
    }

    #[gpui::test]
    async fn generation_journal_reopens_exact_recovery_and_vector_results() {
        async {
            let directory = tempfile::tempdir()?;
            let path = directory.path().join("history.sqlite");
            let mut saved = submission("unresolved");
            saved.source = Some(SavedSource { reference: json!({"asset_id":"original-image"}), name: "Source".into(),
                preview_png: "iVBORw0KGgo=".into(), width: 32, height: 16 });
            {
                let journal = GenerationJournal::new(store(&path).await?, scope())?;
                assert_eq!(journal.prepare(saved.clone()).await?.request, Some(saved.request.clone()));
                journal.record_completed(submission("vector"), SavedRunResult::VectorMessage { message_id: Some("message-one".into()), svg: "<svg><path d='M0 0L2 2'/></svg>".into() }).await?;
            }
            let journal = GenerationJournal::new(store(&path).await?, scope())?;
            let restored = journal.load().await?;
            assert_eq!(restored.records.len(), 2);
            assert!(matches!(&restored.records[0].result, Some(SavedRunResult::VectorMessage { svg, .. }) if svg.contains("M0 0L2 2")));
            assert!(restored.records[0].request.is_none());
            assert_eq!(restored.records[1].request, Some(saved.request));
            assert_eq!(restored.records[1].source, saved.source);
            Ok::<_, anyhow::Error>(())
        }.await.expect("persistent recovery and SVG bytes survive database reopen");
    }

    #[gpui::test]
    async fn generation_journal_completed_messages_never_persist_a_replay_request() {
        async {
            let directory = tempfile::tempdir()?;
            let database = store(&directory.path().join("history.sqlite")).await?;
            let journal = GenerationJournal::new(database.clone(), scope())?;
            // Observe every SQL publication, including any intermediate state
            // that an implementation might incorrectly publish then replace.
            database.write(|connection| connection.exec(
                "CREATE TRIGGER observe_generation_writes AFTER INSERT ON scoped_kv_store BEGIN
                 INSERT OR REPLACE INTO kv_store(key,value) VALUES ('last-generation-write',NEW.value); END;"
            )?()).await?;
            let result = SavedRunResult::VectorMessage { message_id: None, svg: "<svg/>".into() };
            for index in 0..8 { journal.prepare(submission(&format!("pending-{index}"))).await?; }
            database.write(|connection| connection.exec(
                "CREATE TRIGGER forbid_message_replay BEFORE INSERT ON scoped_kv_store
                 WHEN json_extract(NEW.value,'$.records[0].record.key') = 'message'
                      AND json_type(NEW.value,'$.records[0].record.request') != 'null'
                 BEGIN SELECT RAISE(FAIL,'message request must never be recoverable'); END;"
            )?()).await?;
            journal.record_completed(submission("message"), result.clone()).await?;
            journal.record_completed(submission("message"), result.clone()).await?;
            let published: StoredJournal = serde_json::from_str(&database.read_kvp("last-generation-write")?.context("published journal")?)?;
            assert!(published.records[0].record.request.is_none());
            assert_eq!(published.records[0].record.result, Some(result.clone()));
            assert_eq!(published.records.len(), 9, "completed messages do not consume unresolved capacity");
            assert!(journal.prepare(submission("message")).await?.request.is_none());
            let mut changed = submission("message");
            changed.request["prompt"] = json!("changed");
            assert!(journal.record_completed(changed, result).await.is_err());
            assert_eq!(journal.load().await?.records.len(), 9);
            Ok::<_, anyhow::Error>(())
        }.await.expect("message completion is atomic and never exposes media retry state");
    }

    #[gpui::test]
    async fn generation_journal_concurrent_prepare_and_key_replay_are_safe() {
        async {
            let directory = tempfile::tempdir()?;
            let database = store(&directory.path().join("history.sqlite")).await?;
            let first = GenerationJournal::new(database.clone(), scope())?;
            let second = GenerationJournal::new(database, scope())?;
            let (one, two) = futures::join!(
                first.prepare(submission("one")),
                second.prepare(submission("two"))
            );
            one?;
            two?;
            assert_eq!(first.load().await?.records.len(), 2);
            let original = first.prepare(submission("one")).await?;
            let mut reordered = submission("one");
            reordered.request =
                json!({"seed":"18446744073709551615", "prompt":"red circle", "model":"image-test"});
            assert_eq!(first.prepare(reordered).await?, original);
            let mut display_only = submission("one");
            display_only.model = "Renamed model label".into();
            display_only.prompt = "Updated display prompt".into();
            display_only.mode = "Image editor".into();
            display_only.source = Some(SavedSource {
                reference: json!({"asset_id":"same-input"}),
                name: "Renamed source".into(),
                preview_png: "Reencoded thumbnail bytes".into(),
                width: 32,
                height: 16,
            });
            assert_eq!(
                first.prepare(display_only.clone()).await?,
                original,
                "metadata-only changes retain the first unresolved record and its replay body"
            );
            first.accept("one", generated("accepted-one")).await?;
            let replay = first.prepare(submission("one")).await?;
            assert_eq!(replay.result, Some(generated("accepted-one")));
            assert_eq!(
                first.prepare(display_only).await?,
                replay,
                "accepted replay uses request identity and retains the first display metadata"
            );
            assert!(
                replay.request.is_none(),
                "accepted replay must not authorize another POST"
            );
            let mut changed = submission("one");
            changed.request["prompt"] = json!("different");
            assert!(first.prepare(changed).await.is_err());
            first.accept("one", generated("accepted-one")).await?;
            assert!(
                first
                    .accept("one", generated("different-result"))
                    .await
                    .is_err()
            );
            assert!(first.reject("one").await.is_err());
            first.reject("two").await?;
            first.reject("two").await?;
            assert_eq!(first.load().await?.records.len(), 1);
            Ok::<_, anyhow::Error>(())
        }
        .await
        .expect("concurrent callers preserve recovery and accepted keys are never resubmitted");
    }

    #[gpui::test]
    async fn generation_journal_prunes_only_accepted_and_refuses_unresolved_overflow() {
        async {
            let directory = tempfile::tempdir()?;
            let journal = GenerationJournal::new(
                store(&directory.path().join("history.sqlite")).await?,
                scope(),
            )?;
            journal.prepare(submission("old-unresolved")).await?;
            for index in 0..15 {
                let key = format!("accepted-{index}");
                journal.prepare(submission(&key)).await?;
                journal.accept(&key, generated(&key)).await?;
            }
            let records = journal.load().await?.records;
            assert_eq!(records.len(), 13);
            assert_eq!(
                records.first().map(|record| record.key.as_str()),
                Some("accepted-14")
            );
            assert_eq!(
                records.last().map(|record| record.key.as_str()),
                Some("old-unresolved")
            );
            assert!(!records.iter().any(|record| record.key == "accepted-2"));
            for index in 0..7 {
                journal
                    .prepare(submission(&format!("pending-{index}")))
                    .await?;
            }
            let before = journal.load().await?;
            assert!(journal.prepare(submission("overflow")).await.is_err());
            assert_eq!(journal.load().await?, before);
            journal.prepare(submission("old-unresolved")).await?;
            journal
                .accept("old-unresolved", generated("old-completed"))
                .await?;
            journal.prepare(submission("now-room")).await?;
            assert_eq!(
                journal
                    .load()
                    .await?
                    .records
                    .iter()
                    .filter(|record| record.result.is_none())
                    .count(),
                8
            );
            Ok::<_, anyhow::Error>(())
        }
        .await
        .expect("accepted pruning never evicts unresolved requests");
    }

    #[gpui::test]
    async fn generation_journal_corruption_version_and_write_failure_preserve_storage() {
        async {
            let directory = tempfile::tempdir()?;
            let database = store(&directory.path().join("history.sqlite")).await?;
            let journal = GenerationJournal::new(database.clone(), scope())?;
            for invalid in ["{not-json", "{\"version\":999,\"records\":[]}", "{\"version\":1,\"records\":[],\"unknown\":true}"] {
                database.scoped(&journal.namespace).write(STORAGE_KEY.into(), invalid.into()).await?;
                assert!(journal.load().await.is_err());
                assert!(journal.prepare(submission("new")).await.is_err());
                assert!(journal.reject("new").await.is_err());
                assert_eq!(database.scoped(&journal.namespace).read(STORAGE_KEY)?.as_deref(), Some(invalid));
            }
            database.scoped(&journal.namespace).delete(STORAGE_KEY.into()).await?;
            journal.prepare(submission("pending")).await?;
            let before = database.scoped(&journal.namespace).read(STORAGE_KEY)?;
            database.write(|connection| connection.exec(
                "CREATE TRIGGER fail_generation_write AFTER INSERT ON scoped_kv_store BEGIN
                 INSERT OR REPLACE INTO kv_store(key,value) VALUES ('failure-side-effect','must roll back');
                 SELECT RAISE(FAIL,'injected generation storage failure'); END;"
            )?()).await?;
            assert!(journal.accept("pending", generated("done")).await.is_err());
            assert_eq!(database.scoped(&journal.namespace).read(STORAGE_KEY)?, before);
            assert_eq!(database.read_kvp("failure-side-effect")?, None, "savepoint must undo earlier trigger writes too");
            database.write(|connection| connection.exec("DROP TRIGGER fail_generation_write")?()).await?;
            assert_eq!(journal.load().await?.records.len(), 1);
            journal.accept("pending", generated("done")).await?;
            Ok::<_, anyhow::Error>(())
        }.await.expect("corruption and real SQL failure cannot reset unresolved history");
    }

    #[gpui::test]
    async fn generation_journal_isolates_account_organization_and_server() {
        async {
            let directory = tempfile::tempdir()?;
            let database = store(&directory.path().join("history.sqlite")).await?;
            let owner = GenerationJournal::new(database.clone(), scope())?;
            owner.prepare(submission("same-key")).await?;
            for other in [
                JournalScope::new("https://api.example.test", ORGANIZATION, USER)?,
                JournalScope::new("https://api.example.test", USER, USER)?,
                JournalScope::new("http://api.example.test", USER, ORGANIZATION)?,
                JournalScope::new("https://api.example.test/prefix", USER, ORGANIZATION)?,
                JournalScope::new("https://api.example.test:8443", USER, ORGANIZATION)?,
            ] {
                let isolated = GenerationJournal::new(database.clone(), other)?;
                assert!(isolated.load().await?.records.is_empty());
                isolated.prepare(submission("same-key")).await?;
                isolated
                    .accept("same-key", generated("other-account"))
                    .await?;
            }
            assert!(owner.load().await?.records[0].result.is_none());
            let connection =
                ThreadSafeConnection::builder::<KeyValueStore>(&Uuid::new_v4().to_string(), false)
                    .with_write_queue_constructor(locking_queue())
                    .build()
                    .await?;
            let memory = KeyValueStore::from_app_db(&db::AppDatabase(connection));
            assert!(GenerationJournal::new(memory, scope()).is_err());
            Ok::<_, anyhow::Error>(())
        }
        .await
        .expect("history is scoped to stable identity and refuses memory-only fallback");
    }

    #[gpui::test]
    async fn generation_journal_byte_limit_and_transport_credentials_fail_before_mutation() {
        async {
            let directory = tempfile::tempdir()?;
            let journal = GenerationJournal::new(
                store(&directory.path().join("history.sqlite")).await?,
                scope(),
            )?;
            journal.prepare(submission("existing")).await?;
            journal.accept("existing", pending("existing")).await?;
            let before = journal.load().await?;
            let mut too_large = submission("large");
            too_large.prompt = "x".repeat(MAX_BYTES);
            assert!(journal.prepare(too_large).await.is_err());
            assert_eq!(journal.load().await?, before);
            let mut credentials = submission("credentials");
            credentials.request["headers"] = json!({"Authorization":"must-not-store"});
            assert!(journal.prepare(credentials).await.is_err());
            assert_eq!(journal.load().await?, before);
            let mut half = submission("half");
            half.prompt = "x".repeat(MAX_BYTES / 2);
            journal.prepare(half.clone()).await?;
            half.key = "other-half".into();
            assert!(
                journal.prepare(half).await.is_err(),
                "the cap applies to the whole journal, not each record"
            );
            assert_eq!(journal.load().await?.records.len(), 2);
            let oversized = " ".repeat(MAX_BYTES + 1);
            journal
                .store
                .scoped(&journal.namespace)
                .write(STORAGE_KEY.into(), oversized.clone())
                .await?;
            assert!(journal.load().await.is_err());
            assert!(journal.prepare(submission("no-reset")).await.is_err());
            assert_eq!(
                journal
                    .store
                    .scoped(&journal.namespace)
                    .read(STORAGE_KEY)?
                    .as_deref(),
                Some(oversized.as_str())
            );
            Ok::<_, anyhow::Error>(())
        }
        .await
        .expect("storage limits and authentication material cannot replace recoverable data");
    }
    fn dispatching(id: &str, dispatch: SavedDispatch) -> SavedRunResult {
        SavedRunResult::Generation {
            id: id.into(),
            finished: false,
            dispatch,
        }
    }

    #[gpui::test]
    async fn generation_journal_unclaimed_reopens_exact_input_and_claim_is_permanent() {
        async {
            let directory = tempfile::tempdir()?;
            let path = directory.path().join("dispatch.sqlite");
            let mut saved = submission("unclaimed-input");
            saved.request["input"] = json!({"source":{"asset_id":"original"}, "mask":"AA==", "points":[{"x":123,"y":45}], "seed":"18446744073709551615"});
            saved.source = Some(SavedSource { reference: json!({"asset_id":"original"}), name:"Original".into(), preview_png:"AA==".into(), width:400, height:200 });
            let retryable = dispatching("accepted-id", SavedDispatch::Unclaimed { retry_requires_input:true });
            {
                let journal = GenerationJournal::new(store(&path).await?, scope())?;
                journal.prepare(saved.clone()).await?;
                let accepted = journal.accept(&saved.key, retryable.clone()).await?;
                assert_eq!(accepted.request, Some(saved.request.clone()));
                assert!(accepted.result.as_ref().is_some_and(SavedRunResult::requires_input));
            }
            let journal = GenerationJournal::new(store(&path).await?, scope())?;
            let restored = journal.prepare(saved.clone()).await?;
            assert_eq!(restored.request, Some(saved.request.clone()));
            assert_eq!(restored.source, saved.source);
            journal.observe_generation(pending("accepted-id")).await?;
            let unknown = journal.prepare(saved.clone()).await?;
            assert_eq!(unknown.request, restored.request, "missing rollout metadata cannot destroy the only retry input");
            assert!(!unknown.result.as_ref().is_some_and(SavedRunResult::requires_input));
            journal.observe_generation(retryable.clone()).await?;
            assert!(journal.prepare(saved.clone()).await?.result.as_ref().is_some_and(SavedRunResult::requires_input));
            journal.observe_generation(dispatching("accepted-id", SavedDispatch::Claimed)).await?;
            let claimed = journal.prepare(saved.clone()).await?;
            assert!(claimed.request.is_none());
            assert_eq!(claimed.source, saved.source, "history keeps the bounded source thumbnail");
            journal.accept(&saved.key, retryable).await?;
            journal.observe_generation(pending("accepted-id")).await?;
            assert_eq!(journal.prepare(saved.clone()).await?, claimed, "late unclaimed or legacy replies cannot undo a permanent worker claim");
            let mut changed = saved.clone();
            changed.request["input"]["mask"] = json!("different");
            assert!(journal.prepare(changed).await.is_err());
            journal.mark_finished("accepted-id").await?;
            assert!(journal.prepare(saved).await?.request.is_none());
            Ok::<_, anyhow::Error>(())
        }.await.expect("accepted input survives restart until permanent worker claim or terminal status");
    }

    #[gpui::test]
    async fn generation_journal_claim_write_failure_preserves_recoverable_input() {
        async {
            let directory = tempfile::tempdir()?;
            let path = directory.path().join("dispatch.sqlite");
            let database = store(&path).await?;
            let journal = GenerationJournal::new(database.clone(), scope())?;
            let saved = submission("claim-write-failure");
            journal.prepare(saved.clone()).await?;
            journal.accept(&saved.key, dispatching("accepted-id", SavedDispatch::Unclaimed { retry_requires_input:true })).await?;
            let before = database.scoped(&journal.namespace).read(STORAGE_KEY)?;
            database.write(|connection| connection.exec(
                "CREATE TRIGGER fail_claim_write AFTER INSERT ON scoped_kv_store BEGIN
                 INSERT OR REPLACE INTO kv_store(key,value) VALUES ('claim-side-effect','must roll back');
                 SELECT RAISE(FAIL,'injected claim persistence failure'); END;"
            )?()).await?;
            assert!(!journal.observe_generation(dispatching("accepted-id", SavedDispatch::Unclaimed { retry_requires_input:true })).await?.0, "unchanged polling must not rewrite the journal");
            assert!(journal.observe_generation(dispatching("accepted-id", SavedDispatch::Claimed)).await.is_err());
            assert_eq!(database.scoped(&journal.namespace).read(STORAGE_KEY)?, before);
            assert_eq!(database.read_kvp("claim-side-effect")?, None);
            let reopened = GenerationJournal::new(store(&path).await?, scope())?;
            let record = reopened.load().await?.records.into_iter().next().context("saved record")?;
            assert_eq!(record.request, Some(saved.request.clone()));
            assert!(record.result.as_ref().is_some_and(SavedRunResult::requires_input));
            database.write(|connection| connection.exec("DROP TRIGGER fail_claim_write")?()).await?;
            journal.observe_generation(dispatching("accepted-id", SavedDispatch::Claimed)).await?;
            assert!(reopened.prepare(saved).await?.request.is_none());
            Ok::<_, anyhow::Error>(())
        }.await.expect("a failed claim transaction cannot erase the durable input or publish a partial state");
    }

    #[gpui::test]
    async fn generation_journal_legacy_accepted_record_never_recreates_discarded_body() {
        async {
            let directory = tempfile::tempdir()?;
            let database = store(&directory.path().join("dispatch.sqlite")).await?;
            let journal = GenerationJournal::new(database.clone(), scope())?;
            let saved = submission("legacy-key");
            let legacy = json!({"version":1,"records":[{
                "submission_fingerprint":request_fingerprint(&saved.request)?,
                "record":{"key":saved.key,"request":null,"model":saved.model,"prompt":saved.prompt,
                    "source":null,"mode":saved.mode,"result":{"Generation":{"id":"legacy-id","finished":false}}}
            }]});
            database.scoped(&journal.namespace).write(STORAGE_KEY.into(), legacy.to_string()).await?;
            let record = journal.prepare(saved.clone()).await?;
            assert!(record.request.is_none());
            journal.observe_generation(dispatching("legacy-id", SavedDispatch::Unclaimed { retry_requires_input:true })).await?;
            let record = journal.prepare(saved).await?;
            assert!(record.request.is_none(), "a new response cannot reconstruct or authorize replay of discarded input");
            Ok::<_, anyhow::Error>(())
        }.await.expect("legacy accepted history remains readable without retroactive replay");
    }
    #[gpui::test]
    async fn generation_journal_accepted_pending_keeps_exact_input_after_reopen() {
        async {
            let directory = tempfile::tempdir()?;
            let path = directory.path().join("pending.sqlite");
            let mut saved = submission("pending-before-claim");
            saved.request["input"] = json!({"source":{"asset_id":"original"},"mask":"AA=="});
            {
                let journal = GenerationJournal::new(store(&path).await?,scope())?;
                journal.prepare(saved.clone()).await?;
                journal.accept(&saved.key,pending("accepted-before-claim")).await?;
            }
            let reopened = GenerationJournal::new(store(&path).await?,scope())?;
            let record = reopened.prepare(saved.clone()).await?;
            assert_eq!(record.request,Some(saved.request),"an accepted ID alone cannot prove that a worker permanently owns the only replay input");
            Ok::<_,anyhow::Error>(())
        }.await.expect("pending accepted input survives database reopen");
    }
}
