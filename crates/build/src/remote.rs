//! Remote cache and execution integration for the build.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use seal_cas::{ActionOutput, ActionRecord, Cas, Digest};
use seal_remote::{
    CacheAction, RemoteCache, RemoteClient, RemoteConfig, RemoteError, RemoteExecutionRequest,
    RemoteExecutionResult,
};

use crate::action::Action;
use crate::error::{BuildError, BuildResult};

/// One connected remote endpoint shared by every worker.
pub(crate) struct RemoteSession {
    client: Arc<RemoteClient>,
    cache: RemoteCache,
    local: Cas,
    execution: bool,
    require_remote: bool,
    disabled: AtomicBool,
}

impl RemoteSession {
    /// Connects and verifies the endpoint before any action starts.
    pub(crate) fn open(config: RemoteConfig, local: Cas) -> Result<Arc<Self>, RemoteError> {
        let client = RemoteClient::connect(config.clone())?;
        client.capabilities()?;
        let execution = config.execution;
        Ok(Arc::new(Self {
            cache: RemoteCache::new(Arc::clone(&client), local.clone()),
            client,
            local,
            execution,
            require_remote: config.require_remote,
            disabled: AtomicBool::new(false),
        }))
    }

    /// Returns whether the session still accepts requests.
    pub(crate) fn available(&self) -> bool {
        !self.disabled.load(Ordering::Relaxed)
    }

    /// Disables the session after an infrastructure failure.
    pub(crate) fn disable(&self) {
        self.disabled.store(true, Ordering::Relaxed);
    }

    /// Returns whether remote execution is enabled.
    pub(crate) fn execution(&self) -> bool {
        self.execution
    }

    /// Returns whether failures must abort the build.
    pub(crate) fn required(&self) -> bool {
        self.require_remote
    }

    /// Returns the redacted message for a remote failure.
    pub(crate) fn message(&self, error: &RemoteError) -> String {
        self.client.redact(error.message())
    }

    /// Looks up one action in the remote cache.
    pub(crate) fn lookup(&self, action: &CacheAction) -> Result<Option<ActionRecord>, RemoteError> {
        self.cache.lookup(action)
    }

    /// Stores one successful local result remotely.
    pub(crate) fn store(
        &self,
        action: &CacheAction,
        record: &ActionRecord,
    ) -> Result<(), RemoteError> {
        self.cache.store(action, record)
    }

    /// Executes one action remotely and turns its files into a record.
    pub(crate) fn execute(
        &self,
        action: &Action,
        request: &RemoteExecutionRequest,
    ) -> Result<ActionRecord, RemoteError> {
        let result = self.client.execute(request)?;
        if result.exit_code != 0 {
            return Err(RemoteError::action_failed(
                result.exit_code,
                format!(
                    "the remote `{}` action failed with exit code {}: {}",
                    action.kind(),
                    result.exit_code,
                    self.client.redact(&String::from_utf8_lossy(&result.stderr))
                ),
            ));
        }
        record_from_result(action, result, &self.local)
    }
}

/// Builds the cache descriptor for one local action.
pub(crate) fn cache_action(action: &Action, key: Digest) -> CacheAction {
    CacheAction {
        key,
        kind: action.kind().to_owned(),
        command: action.command(),
        inputs: action.inputs().to_vec(),
        environment: action.environment(),
        outputs: action.output_names(),
    }
}

fn record_from_result(
    action: &Action,
    result: RemoteExecutionResult,
    local: &Cas,
) -> Result<ActionRecord, RemoteError> {
    let mut outputs = Vec::with_capacity(result.files.len());
    for file in result.files {
        let digest = local.put_bytes(&file.bytes)?;
        outputs.push(ActionOutput {
            name: file.name,
            digest,
            mode: if file.executable { 0o755 } else { 0o644 },
        });
    }
    let mut record = ActionRecord::new(action.kind(), action.inputs().to_vec(), outputs)
        .with_command(action.command());
    record.env = action.environment();
    Ok(record)
}

/// Converts a remote failure into the build diagnostic, redacting secrets.
pub(crate) fn error(session: &RemoteSession, failure: RemoteError) -> BuildError {
    BuildError::new(failure.code(), session.message(&failure))
}

/// Converts a remote failure into a local fallback or a required failure.
pub(crate) fn fallback(session: &RemoteSession, failure: RemoteError) -> BuildResult<()> {
    if session.required() {
        return Err(error(session, failure));
    }
    session.disable();
    Ok(())
}
