use crate::{AgentMessage, AgentMessageContent, UserMessage, UserMessageContent};
use acp_thread::ClientUserMessageId;
use agent_client_protocol::v2::schema as acp_v2;
use agent_settings::AgentProfileId;
use anyhow::Result;
use chrono::{DateTime, Utc};
use collections::{HashMap, IndexMap};
use futures::{FutureExt, future::Shared};
use gpui::{App, AppContext, BackgroundExecutor, Global, Task};
use indoc::indoc;
use language_model::Speed;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sqlez::{
    bindable::{Bind, Column},
    connection::Connection,
    statement::Statement,
};
use std::{
    io::{BufWriter, ErrorKind, Write as _},
    path::PathBuf,
    sync::Arc,
};
use ui::SharedString;
use util::{ResultExt, path_list::PathList};
use zed_env_vars::ZED_STATELESS;

pub type DbMessage = crate::Message;
pub type DbLanguageModel = crate::legacy_thread::SerializedLanguageModel;

#[derive(Debug, Clone)]
pub struct DbThreadMetadata {
    pub id: acp_v2::SessionId,
    pub parent_session_id: Option<acp_v2::SessionId>,
    pub title: SharedString,
    pub updated_at: DateTime<Utc>,
    pub created_at: Option<DateTime<Utc>>,
    /// The workspace folder paths this thread was created against, sorted
    /// lexicographically. Used for grouping threads by project in the sidebar.
    pub folder_paths: PathList,
}

impl From<&DbThreadMetadata> for acp_thread::AgentSessionInfo {
    fn from(meta: &DbThreadMetadata) -> Self {
        Self {
            session_id: meta.id.clone(),
            work_dirs: Some(meta.folder_paths.clone()),
            title: Some(meta.title.clone()),
            updated_at: Some(meta.updated_at),
            created_at: meta.created_at,
            meta: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DbThread {
    pub title: SharedString,
    pub messages: Vec<Arc<DbMessage>>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub detailed_summary: Option<SharedString>,
    #[serde(default)]
    pub initial_project_snapshot: Option<Arc<crate::ProjectSnapshot>>,
    #[serde(default)]
    pub cumulative_token_usage: language_model::TokenUsage,
    #[serde(default)]
    pub request_token_usage: HashMap<acp_thread::ClientUserMessageId, language_model::TokenUsage>,
    #[serde(default)]
    pub model: Option<DbLanguageModel>,
    #[serde(default)]
    pub profile: Option<AgentProfileId>,
    #[serde(default)]
    pub subagent_context: Option<crate::SubagentContext>,
    #[serde(default)]
    pub speed: Option<Speed>,
    #[serde(default)]
    pub thinking_enabled: bool,
    #[serde(default)]
    pub thinking_effort: Option<String>,
    #[serde(default)]
    pub draft_prompt: Option<Vec<acp_v2::ContentBlock>>,
    #[serde(default)]
    pub ui_scroll_position: Option<SerializedScrollPosition>,
    #[serde(default)]
    pub sandboxed_terminal_temp_dir: Option<PathBuf>,
    /// Sandbox escalations the user approved "for the rest of this thread".
    /// Persisted so reopening a thread keeps its grants. See
    /// [`crate::sandboxing::ThreadSandboxGrants`].
    #[serde(default)]
    pub sandbox_grants: DbSandboxGrants,
}

/// Serialized form of the sandbox permissions the user granted "for the rest of
/// this thread" (the "Allow for this thread" prompt option). Stored inside the
/// thread blob; round-trips with [`crate::sandboxing::ThreadSandboxGrants`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DbSandboxGrants {
    /// Paths granted write access, each paired with the canonical
    /// (symlink-resolved) target established when the grant was approved; each
    /// covers its whole subtree. Legacy rows stored a bare path string per
    /// entry, which still deserializes (as a grant with no resolved canonical)
    /// via [`settings::GrantedWritePath`]'s string-or-object format.
    #[serde(default)]
    pub write_paths: Vec<settings::GrantedWritePath>,
    /// Host patterns granted network access, in canonical string form (e.g.
    /// `github.com`, `*.npmjs.org`). Parsed back into patterns on load.
    #[serde(default)]
    pub network_hosts: Vec<String>,
    /// Whether arbitrary-host network access was granted.
    #[serde(default)]
    pub network_any_host: bool,
    /// Whether unrestricted filesystem writes (the broad escape hatch) were
    /// granted.
    #[serde(default)]
    pub allow_fs_write_all: bool,

    /// Whether the model-requested fully-unsandboxed escape was granted.
    #[serde(default)]
    pub unsandboxed: bool,
    /// Whether running commands unsandboxed was allowed because the OS sandbox
    /// could not be created (the fallback prompt's "for this thread" option).
    #[serde(default)]
    pub sandbox_fallback: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SerializedScrollPosition {
    pub item_ix: usize,
    pub offset_in_item: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedThread {
    pub title: SharedString,
    pub messages: Vec<Arc<DbMessage>>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub model: Option<DbLanguageModel>,
    pub version: String,
}

impl SharedThread {
    pub const VERSION: &'static str = "1.0.0";

    pub fn from_db_thread(thread: &DbThread) -> Self {
        Self {
            title: thread.title.clone(),
            messages: thread.messages.clone(),
            updated_at: thread.updated_at,
            model: thread.model.clone(),
            version: Self::VERSION.to_string(),
        }
    }

    pub fn to_db_thread(self) -> DbThread {
        DbThread {
            title: format!("🔗 {}", self.title).into(),
            messages: self.messages,
            updated_at: self.updated_at,
            detailed_summary: None,
            initial_project_snapshot: None,
            cumulative_token_usage: Default::default(),
            request_token_usage: Default::default(),
            model: self.model,
            profile: None,
            subagent_context: None,
            speed: None,
            thinking_enabled: false,
            thinking_effort: None,
            draft_prompt: None,
            ui_scroll_position: None,
            sandboxed_terminal_temp_dir: None,
            sandbox_grants: DbSandboxGrants::default(),
        }
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        const COMPRESSION_LEVEL: i32 = 3;
        let json = serde_json::to_vec(self)?;
        let compressed = zstd::encode_all(json.as_slice(), COMPRESSION_LEVEL)?;
        Ok(compressed)
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let decompressed = zstd::decode_all(data)?;
        Ok(serde_json::from_slice(&decompressed)?)
    }
}

impl DbThread {
    pub const VERSION: &'static str = "0.3.0";

    pub fn to_markdown(&self) -> String {
        crate::messages_to_markdown(&self.messages)
    }

    pub fn from_json(json: &[u8]) -> Result<Self> {
        let saved_thread_json = serde_json::from_slice::<serde_json::Value>(json)?;
        match saved_thread_json.get("version") {
            Some(serde_json::Value::String(version)) => match version.as_str() {
                Self::VERSION => Ok(serde_json::from_value(saved_thread_json)?),
                _ => Self::upgrade_from_agent_1(crate::legacy_thread::SerializedThread::from_json(
                    json,
                )?),
            },
            _ => {
                Self::upgrade_from_agent_1(crate::legacy_thread::SerializedThread::from_json(json)?)
            }
        }
    }

    fn upgrade_from_agent_1(thread: crate::legacy_thread::SerializedThread) -> Result<Self> {
        let mut messages = Vec::new();
        let mut request_token_usage = HashMap::default();

        let mut last_user_message_id = None;
        for (ix, msg) in thread.messages.into_iter().enumerate() {
            let message = match msg.role {
                language_model::Role::User => {
                    let mut content = Vec::new();

                    // Convert segments to content
                    for segment in msg.segments {
                        match segment {
                            crate::legacy_thread::SerializedMessageSegment::Text { text } => {
                                content.push(UserMessageContent::Text(text));
                            }
                            crate::legacy_thread::SerializedMessageSegment::Thinking {
                                text,
                                ..
                            } => {
                                // User messages don't have thinking segments, but handle gracefully
                                content.push(UserMessageContent::Text(text));
                            }
                            crate::legacy_thread::SerializedMessageSegment::RedactedThinking {
                                ..
                            } => {
                                // User messages don't have redacted thinking, skip.
                            }
                        }
                    }

                    // If no content was added, add context as text if available
                    if content.is_empty() && !msg.context.is_empty() {
                        content.push(UserMessageContent::Text(msg.context));
                    }

                    let id = ClientUserMessageId::new();
                    last_user_message_id = Some(id.clone());

                    crate::Message::User(UserMessage {
                        // MessageId from old format can't be meaningfully converted, so generate a new one
                        id,
                        content: Arc::from(content),
                    })
                }
                language_model::Role::Assistant => {
                    let mut content = Vec::new();

                    // Convert segments to content
                    for segment in msg.segments {
                        match segment {
                            crate::legacy_thread::SerializedMessageSegment::Text { text } => {
                                content.push(AgentMessageContent::Text(text));
                            }
                            crate::legacy_thread::SerializedMessageSegment::Thinking {
                                text,
                                signature,
                            } => {
                                content.push(AgentMessageContent::Thinking { text, signature });
                            }
                            crate::legacy_thread::SerializedMessageSegment::RedactedThinking {
                                data,
                            } => {
                                content.push(AgentMessageContent::RedactedThinking(data));
                            }
                        }
                    }

                    // Convert tool uses
                    let mut tool_names_by_id = HashMap::default();
                    for tool_use in msg.tool_uses {
                        tool_names_by_id.insert(tool_use.id.clone(), tool_use.name.clone());
                        content.push(AgentMessageContent::ToolUse(
                            language_model::LanguageModelToolUse {
                                id: tool_use.id,
                                name: tool_use.name.into(),
                                raw_input: serde_json::to_string(&tool_use.input)
                                    .unwrap_or_default(),
                                input: language_model::LanguageModelToolUseInput::Json(
                                    tool_use.input,
                                ),
                                is_input_complete: true,
                                thought_signature: None,
                            },
                        ));
                    }

                    // Convert tool results
                    let mut tool_results = IndexMap::default();
                    for tool_result in msg.tool_results {
                        let name = tool_names_by_id
                            .remove(&tool_result.tool_use_id)
                            .unwrap_or_else(|| SharedString::from("unknown"));
                        tool_results.insert(
                            tool_result.tool_use_id.clone(),
                            language_model::LanguageModelToolResult {
                                tool_use_id: tool_result.tool_use_id,
                                tool_name: name.into(),
                                is_error: tool_result.is_error,
                                content: vec![tool_result.content],
                                output: tool_result.output,
                            },
                        );
                    }

                    if let Some(last_user_message_id) = &last_user_message_id
                        && let Some(token_usage) = thread.request_token_usage.get(ix).copied()
                    {
                        request_token_usage.insert(last_user_message_id.clone(), token_usage);
                    }

                    crate::Message::Agent(AgentMessage {
                        content,
                        tool_results,
                        reasoning_details: None,
                    })
                }
                language_model::Role::System => {
                    // Skip system messages as they're not supported in the new format
                    continue;
                }
            };

            messages.push(Arc::new(message));
        }

        Ok(Self {
            title: thread.summary,
            messages,
            updated_at: thread.updated_at,
            detailed_summary: match thread.detailed_summary_state {
                crate::legacy_thread::DetailedSummaryState::NotGenerated
                | crate::legacy_thread::DetailedSummaryState::Generating => None,
                crate::legacy_thread::DetailedSummaryState::Generated { text, .. } => Some(text),
            },
            initial_project_snapshot: thread.initial_project_snapshot,
            cumulative_token_usage: thread.cumulative_token_usage,
            request_token_usage,
            model: thread.model,
            profile: thread.profile,
            subagent_context: None,
            speed: None,
            thinking_enabled: false,
            thinking_effort: None,
            draft_prompt: None,
            ui_scroll_position: None,
            sandboxed_terminal_temp_dir: None,
            sandbox_grants: DbSandboxGrants::default(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataType {
    #[serde(rename = "json")]
    Json,
    #[serde(rename = "zstd")]
    Zstd,
}

impl Bind for DataType {
    fn bind(&self, statement: &Statement, start_index: i32) -> Result<i32> {
        let value = match self {
            DataType::Json => "json",
            DataType::Zstd => "zstd",
        };
        value.bind(statement, start_index)
    }
}

impl Column for DataType {
    fn column(statement: &mut Statement, start_index: i32) -> Result<(Self, i32)> {
        let (value, next_index) = String::column(statement, start_index)?;
        let data_type = match value.as_str() {
            "json" => DataType::Json,
            "zstd" => DataType::Zstd,
            _ => anyhow::bail!("Unknown data type: {}", value),
        };
        Ok((data_type, next_index))
    }
}

#[derive(Clone, Debug, Default)]
pub struct TokenUsageEvent {
    pub request_id: String,
    pub created_at: i64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub saved_tokens: u64,
    pub model: Option<String>,
    /// Thread the request belongs to, used for per-chat and per-project views.
    pub thread_id: Option<String>,
    /// Provider (e.g. `opencode`, `command-code`) that served the request.
    pub provider: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TokenUsageBucket {
    pub start: i64,
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub saved_tokens: u64,
}

impl TokenUsageBucket {
    /// Prompt-side tokens: everything sent to the provider, cached or not.
    pub fn total(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.cache_read_tokens)
            .saturating_add(self.cache_write_tokens)
    }

    /// Prompt plus completion tokens, i.e. everything billed for this bucket.
    pub fn total_tokens(&self) -> u64 {
        self.total().saturating_add(self.output_tokens)
    }
}

type TokenUsageEventParams = (
    String,
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
    Option<String>,
    Option<String>,
    Option<String>,
);

pub(crate) struct ThreadsDatabase {
    executor: BackgroundExecutor,
    connection: Arc<Mutex<Connection>>,
    /// In production, saves take real time (serialization, zstd, disk I/O) while
    /// the user keeps typing, so new save requests routinely arrive mid-write.
    /// The test executor completes writes instantly, so tests use this gate to
    /// hold a write in flight and interleave more save requests with it.
    #[cfg(test)]
    write_gate: Mutex<Option<Shared<futures::channel::oneshot::Receiver<()>>>>,
    #[cfg(test)]
    save_count: std::sync::atomic::AtomicUsize,
}

const LIST_THREADS_QUERY: &str = "SELECT id, parent_id, folder_paths, folder_paths_order, summary, updated_at, created_at FROM threads ORDER BY updated_at DESC, created_at DESC";

struct GlobalThreadsDatabase(Shared<Task<Result<Arc<ThreadsDatabase>, Arc<anyhow::Error>>>>);

impl Global for GlobalThreadsDatabase {}

impl ThreadsDatabase {
    pub fn connect(cx: &mut App) -> Shared<Task<Result<Arc<ThreadsDatabase>, Arc<anyhow::Error>>>> {
        if cx.has_global::<GlobalThreadsDatabase>() {
            return cx.global::<GlobalThreadsDatabase>().0.clone();
        }
        let executor = cx.background_executor().clone();
        let task = executor
            .spawn({
                let executor = executor.clone();
                async move {
                    match ThreadsDatabase::new(executor) {
                        Ok(db) => Ok(Arc::new(db)),
                        Err(err) => Err(Arc::new(err)),
                    }
                }
            })
            .shared();

        cx.set_global(GlobalThreadsDatabase(task.clone()));
        task
    }

    pub fn new(executor: BackgroundExecutor) -> Result<Self> {
        let connection = if *ZED_STATELESS {
            Connection::open_memory(Some("THREAD_FALLBACK_DB"))
        } else if cfg!(any(feature = "test-support", test)) {
            // rust stores the name of the test on the current thread.
            // We use this to automatically create a database that will
            // be shared within the test (for the test_retrieve_old_thread)
            // but not with concurrent tests.
            let thread = std::thread::current();
            let test_name = thread.name();
            Connection::open_memory(Some(&format!(
                "THREAD_FALLBACK_{}",
                test_name.unwrap_or_default()
            )))
        } else {
            let threads_dir = paths::data_dir().join("threads");
            std::fs::create_dir_all(&threads_dir)?;
            let sqlite_path = threads_dir.join("threads.db");
            Connection::open_file(&sqlite_path.to_string_lossy())
        };

        connection.exec(indoc! {"
            CREATE TABLE IF NOT EXISTS threads (
                id TEXT PRIMARY KEY,
                summary TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                data_type TEXT NOT NULL,
                data BLOB NOT NULL
            )
        "})?()
        .map_err(|e| e.context("Failed to create threads table"))?;

        if let Ok(mut s) = connection.exec(indoc! {"
            ALTER TABLE threads ADD COLUMN parent_id TEXT
        "})
        {
            s().ok();
        }

        if let Ok(mut s) = connection.exec(indoc! {"
            ALTER TABLE threads ADD COLUMN folder_paths TEXT;
            ALTER TABLE threads ADD COLUMN folder_paths_order TEXT;
        "})
        {
            s().ok();
        }

        if let Ok(mut s) = connection.exec(indoc! {"
            ALTER TABLE threads ADD COLUMN created_at TEXT;
        "})
        {
            if s().is_ok() {
                connection.exec(indoc! {"
                    UPDATE threads SET created_at = updated_at WHERE created_at IS NULL
                "})?()?;
            }
        }

        // The columns added above are stored after the `data` blob, so reading
        // them from the table walks every thread's overflow pages. This index
        // covers `LIST_THREADS_QUERY`, so listing threads never touches the blobs.
        connection
            .exec(indoc! {"
                CREATE INDEX IF NOT EXISTS threads_list_metadata ON threads (
                    updated_at, created_at, id, parent_id, folder_paths, folder_paths_order, summary
                )
            "})
            .and_then(|mut create_index| create_index())
            .map_err(|e| e.context("Failed to create threads list index"))
            .log_err();

        connection
            .exec(indoc! {"
                CREATE TABLE IF NOT EXISTS token_usage_events (
                    request_id TEXT PRIMARY KEY,
                    created_at INTEGER NOT NULL,
                    input_tokens INTEGER NOT NULL,
                    output_tokens INTEGER NOT NULL,
                    cache_read_tokens INTEGER NOT NULL,
                    cache_write_tokens INTEGER NOT NULL,
                    saved_tokens INTEGER NOT NULL,
                    model TEXT,
                    thread_id TEXT,
                    provider TEXT
                )
            "})
            .and_then(|mut create_table| create_table())
            .map_err(|e| e.context("Failed to create token_usage_events table"))
            .log_err();

        connection
            .exec(indoc! {"
                CREATE INDEX IF NOT EXISTS token_usage_events_created_at ON token_usage_events (created_at)
            "})
            .and_then(|mut create_index| create_index())
            .map_err(|e| e.context("Failed to create token_usage_events index"))
            .log_err();

        // Older databases predate the per-thread and per-provider breakdown, so
        // add the columns in place. SQLite has no `ADD COLUMN IF NOT EXISTS`, and
        // the error for a duplicate column is expected.
        for column in ["thread_id", "provider"] {
            let statement = format!(
                "ALTER TABLE token_usage_events ADD COLUMN {} TEXT",
                column
            );
            connection
                .exec(&statement)
                .and_then(|mut alter| alter())
                .ok();
        }

        const TOKEN_USAGE_RETENTION_SECONDS: i64 = 90 * 24 * 60 * 60;
        let cutoff = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or_default()
            - TOKEN_USAGE_RETENTION_SECONDS;
        connection
            .exec_bound::<i64>("DELETE FROM token_usage_events WHERE created_at < ?")
            .and_then(|mut delete| delete(cutoff))
            .map_err(|e| e.context("Failed to prune token_usage_events"))
            .log_err();

        let db = Self {
            executor,
            connection: Arc::new(Mutex::new(connection)),
            #[cfg(test)]
            write_gate: Mutex::new(None),
            #[cfg(test)]
            save_count: Default::default(),
        };

        Ok(db)
    }

    fn save_thread_sync(
        connection: &Arc<Mutex<Connection>>,
        id: acp_v2::SessionId,
        thread: DbThread,
        folder_paths: &PathList,
    ) -> Result<()> {
        const COMPRESSION_LEVEL: i32 = 3;

        #[derive(Serialize)]
        struct SerializedThread {
            #[serde(flatten)]
            thread: DbThread,
            version: &'static str,
        }

        let title = thread.title.to_string();
        let updated_at = thread.updated_at.to_rfc3339();
        let parent_id = thread
            .subagent_context
            .as_ref()
            .map(|ctx| ctx.parent_thread_id.0.clone());
        let serialized_folder_paths = folder_paths.serialize();
        let (folder_paths_str, folder_paths_order_str): (Option<String>, Option<String>) =
            if folder_paths.is_empty() {
                (None, None)
            } else {
                (
                    Some(serialized_folder_paths.paths),
                    Some(serialized_folder_paths.order),
                )
            };
        let mut encoder = zstd::stream::Encoder::new(Vec::new(), COMPRESSION_LEVEL)?;
        {
            let mut writer = BufWriter::new(&mut encoder);
            serde_json::to_writer(
                &mut writer,
                &SerializedThread {
                    thread,
                    version: DbThread::VERSION,
                },
            )?;
            writer.flush()?;
        }
        let compressed = encoder.finish()?;

        let connection = connection.lock();

        let data_type = DataType::Zstd;
        let data = compressed;

        // Use the thread's updated_at as created_at for new threads.
        // This ensures the creation time reflects when the thread was conceptually
        // created, not when it was saved to the database.
        let created_at = updated_at.clone();

        let mut insert = connection.exec_bound::<(Arc<str>, Option<Arc<str>>, Option<String>, Option<String>, String, String, DataType, Vec<u8>, String)>(indoc! {"
            INSERT INTO threads (id, parent_id, folder_paths, folder_paths_order, summary, updated_at, data_type, data, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(id) DO UPDATE SET
                parent_id = excluded.parent_id,
                folder_paths = excluded.folder_paths,
                folder_paths_order = excluded.folder_paths_order,
                summary = excluded.summary,
                updated_at = excluded.updated_at,
                data_type = excluded.data_type,
                data = excluded.data
        "})?;

        insert((
            id.0,
            parent_id,
            folder_paths_str,
            folder_paths_order_str,
            title,
            updated_at,
            data_type,
            data,
            created_at,
        ))?;

        Ok(())
    }

    pub fn list_threads(&self) -> Task<Result<Vec<DbThreadMetadata>>> {
        let connection = self.connection.clone();

        self.executor.spawn(async move {
            let connection = connection.lock();

            let mut select = connection.select_bound::<(), (
                Arc<str>,
                Option<Arc<str>>,
                Option<String>,
                Option<String>,
                String,
                String,
                Option<String>,
            )>(LIST_THREADS_QUERY)?;

            let rows = select(())?;
            let mut threads = Vec::new();

            for (
                id,
                parent_id,
                folder_paths,
                folder_paths_order,
                summary,
                updated_at,
                created_at,
            ) in rows
            {
                let folder_paths = folder_paths
                    .map(|paths| {
                        PathList::deserialize(&util::path_list::SerializedPathList {
                            paths,
                            order: folder_paths_order.unwrap_or_default(),
                        })
                    })
                    .unwrap_or_default();
                let created_at = created_at
                    .as_deref()
                    .map(DateTime::parse_from_rfc3339)
                    .transpose()?
                    .map(|dt| dt.with_timezone(&Utc));

                threads.push(DbThreadMetadata {
                    id: acp_v2::SessionId::new(id),
                    parent_session_id: parent_id.map(acp_v2::SessionId::new),
                    title: summary.into(),
                    updated_at: DateTime::parse_from_rfc3339(&updated_at)?.with_timezone(&Utc),
                    created_at,
                    folder_paths,
                });
            }

            Ok(threads)
        })
    }

    pub fn load_thread(&self, id: acp_v2::SessionId) -> Task<Result<Option<DbThread>>> {
        let connection = self.connection.clone();

        self.executor.spawn(async move {
            let connection = connection.lock();
            let mut select = connection.select_bound::<Arc<str>, (DataType, Vec<u8>)>(indoc! {"
                SELECT data_type, data FROM threads WHERE id = ? LIMIT 1
            "})?;

            let rows = select(id.0)?;
            if let Some((data_type, data)) = rows.into_iter().next() {
                Ok(Some(Self::deserialize_thread(data_type, data)?))
            } else {
                Ok(None)
            }
        })
    }

    pub fn save_thread(
        &self,
        id: acp_v2::SessionId,
        thread: DbThread,
        folder_paths: PathList,
    ) -> Task<Result<()>> {
        let connection = self.connection.clone();
        #[cfg(test)]
        let write_gate = self.write_gate.lock().clone();
        #[cfg(test)]
        self.save_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        self.executor.spawn(async move {
            #[cfg(test)]
            if let Some(write_gate) = write_gate {
                write_gate.await.ok();
            }
            Self::save_thread_sync(&connection, id, thread, &folder_paths)
        })
    }

    #[cfg(test)]
    pub fn save_count(&self) -> usize {
        self.save_count.load(std::sync::atomic::Ordering::SeqCst)
    }

    #[cfg(test)]
    pub fn set_write_gate(&self, gate: futures::channel::oneshot::Receiver<()>) {
        *self.write_gate.lock() = Some(gate.shared());
    }

    fn deserialize_thread(data_type: DataType, data: Vec<u8>) -> Result<DbThread> {
        let json_data = match data_type {
            DataType::Zstd => {
                let decompressed = zstd::decode_all(&data[..])?;
                String::from_utf8(decompressed)?
            }
            DataType::Json => String::from_utf8(data)?,
        };
        DbThread::from_json(json_data.as_bytes())
    }

    fn sandboxed_terminal_temp_dir(data_type: DataType, data: Vec<u8>) -> Option<PathBuf> {
        match Self::deserialize_thread(data_type, data) {
            Ok(thread) => thread.sandboxed_terminal_temp_dir,
            Err(error) => {
                log::warn!("failed to deserialize thread before deleting it: {error:#}");
                None
            }
        }
    }

    fn remove_sandboxed_terminal_temp_dir(temp_dir: PathBuf) {
        match std::fs::remove_dir_all(&temp_dir) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                log::warn!(
                    "failed to remove sandboxed terminal temp directory {}: {error}",
                    temp_dir.display()
                );
            }
        }
    }

    pub fn delete_thread(&self, id: acp_v2::SessionId) -> Task<Result<()>> {
        let connection = self.connection.clone();

        self.executor.spawn(async move {
            let sandboxed_terminal_temp_dirs = {
                let connection = connection.lock();

                let mut select_children =
                    connection.select_bound::<Arc<str>, Arc<str>>(indoc! {"
                    SELECT id FROM threads WHERE parent_id = ?
                "})?;

                // Collect target thread together with all of its transitive
                // subagent threads
                let mut ids_to_delete = vec![id.0.clone()];
                let mut frontier = vec![id.0.clone()];
                while let Some(parent) = frontier.pop() {
                    for child in select_children(parent)? {
                        ids_to_delete.push(child.clone());
                        frontier.push(child);
                    }
                }

                let mut select =
                    connection.select_bound::<Arc<str>, (DataType, Vec<u8>)>(indoc! {"
                    SELECT data_type, data FROM threads WHERE id = ? LIMIT 1
                "})?;

                let mut delete = connection.exec_bound::<Arc<str>>(indoc! {"
                    DELETE FROM threads WHERE id = ?
                "})?;

                let mut sandboxed_terminal_temp_dirs = Vec::new();
                for thread_id in ids_to_delete {
                    if let Some(temp_dir) = select(thread_id.clone())?.into_iter().next().and_then(
                        |(data_type, data)| Self::sandboxed_terminal_temp_dir(data_type, data),
                    ) {
                        sandboxed_terminal_temp_dirs.push(temp_dir);
                    }
                    delete(thread_id)?;
                }

                sandboxed_terminal_temp_dirs
            };

            for temp_dir in sandboxed_terminal_temp_dirs {
                Self::remove_sandboxed_terminal_temp_dir(temp_dir);
            }

            Ok(())
        })
    }

    pub fn record_token_usage_event(&self, event: TokenUsageEvent) -> Task<Result<()>> {
        let connection = self.connection.clone();

        self.executor.spawn(async move {
            let connection = connection.lock();

            let mut upsert = connection.exec_bound::<TokenUsageEventParams>(indoc! {"
                    INSERT INTO token_usage_events (
                        request_id, created_at, input_tokens, output_tokens,
                        cache_read_tokens, cache_write_tokens, saved_tokens, model,
                        thread_id, provider
                    )
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                    ON CONFLICT(request_id) DO UPDATE SET
                        input_tokens = MAX(input_tokens, excluded.input_tokens),
                        output_tokens = MAX(output_tokens, excluded.output_tokens),
                        cache_read_tokens = MAX(cache_read_tokens, excluded.cache_read_tokens),
                        cache_write_tokens = MAX(cache_write_tokens, excluded.cache_write_tokens),
                        saved_tokens = MAX(saved_tokens, excluded.saved_tokens),
                        model = COALESCE(excluded.model, token_usage_events.model)
                "})?;

            upsert((
                event.request_id,
                event.created_at,
                event.input_tokens as i64,
                event.output_tokens as i64,
                event.cache_read_tokens as i64,
                event.cache_write_tokens as i64,
                event.saved_tokens as i64,
                event.model,
                event.thread_id,
                event.provider,
            ))?;

            Ok(())
        })
    }

    pub fn token_usage_buckets(
        &self,
        since: i64,
        bucket_seconds: i64,
    ) -> Task<Result<Vec<TokenUsageBucket>>> {
        let bucket_seconds = bucket_seconds.max(1);
        let connection = self.connection.clone();

        self.executor.spawn(async move {
            let connection = connection.lock();

            let mut select = connection
                .select_bound::<(i64, i64), (i64, i64, i64, i64, i64, i64, i64)>(indoc! {"
                SELECT
                    (created_at / ?1) * ?1 AS bucket_start,
                    COUNT(*),
                    SUM(input_tokens),
                    SUM(output_tokens),
                    SUM(cache_read_tokens),
                    SUM(cache_write_tokens),
                    SUM(saved_tokens)
                FROM token_usage_events
                WHERE created_at >= ?2
                GROUP BY bucket_start
                ORDER BY bucket_start
            "})?;

            let rows = select((bucket_seconds, since))?;

            Ok(rows
                .into_iter()
                .map(
                    |(
                        start,
                        requests,
                        input_tokens,
                        output_tokens,
                        cache_read_tokens,
                        cache_write_tokens,
                        saved_tokens,
                    )| TokenUsageBucket {
                        start,
                        requests: requests.max(0) as u64,
                        input_tokens: input_tokens.max(0) as u64,
                        output_tokens: output_tokens.max(0) as u64,
                        cache_read_tokens: cache_read_tokens.max(0) as u64,
                        cache_write_tokens: cache_write_tokens.max(0) as u64,
                        saved_tokens: saved_tokens.max(0) as u64,
                    },
                )
                .collect())
        })
    }

    /// Aggregates recorded usage by one dimension (model, provider or thread).
    ///
    /// Project-level totals are computed by the caller from the per-thread
    /// breakdown joined with the thread list metadata, so that a thread opened
    /// from several folders is attributed to each of them.
    pub fn token_usage_breakdown(
        &self,
        since: i64,
        group_by: TokenUsageGroupBy,
    ) -> Task<Result<Vec<TokenUsageBreakdownRow>>> {
        let connection = self.connection.clone();
        let column = group_by.column();

        self.executor.spawn(async move {
            let connection = connection.lock();

            let query = format!(
                "SELECT
                    {column} AS key,
                    COUNT(*),
                    SUM(input_tokens),
                    SUM(output_tokens),
                    SUM(cache_read_tokens),
                    SUM(cache_write_tokens),
                    SUM(saved_tokens)
                FROM token_usage_events
                WHERE created_at >= ?1
                GROUP BY key
                ORDER BY SUM(input_tokens + cache_read_tokens + cache_write_tokens) DESC"
            );

            let mut select = connection.select_bound::<i64, (
                Option<String>,
                i64,
                i64,
                i64,
                i64,
                i64,
                i64,
            )>(&query)?;

            let rows = select(since)?;

            Ok(rows
                .into_iter()
                .map(
                    |(
                        key,
                        requests,
                        input_tokens,
                        output_tokens,
                        cache_read_tokens,
                        cache_write_tokens,
                        saved_tokens,
                    )| TokenUsageBreakdownRow {
                        key,
                        requests: requests.max(0) as u64,
                        input_tokens: input_tokens.max(0) as u64,
                        output_tokens: output_tokens.max(0) as u64,
                        cache_read_tokens: cache_read_tokens.max(0) as u64,
                        cache_write_tokens: cache_write_tokens.max(0) as u64,
                        saved_tokens: saved_tokens.max(0) as u64,
                    },
                )
                .collect())
        })
    }

    /// Deletes every recorded usage event, resetting all statistics.
    pub fn clear_token_usage_events(&self) -> Task<Result<()>> {
        let connection = self.connection.clone();

        self.executor.spawn(async move {
            let connection = connection.lock();
            let mut delete = connection.exec_bound::<()>("DELETE FROM token_usage_events")?;
            delete(())?;
            Ok(())
        })
    }

    pub fn delete_threads(&self) -> Task<Result<()>> {
        let connection = self.connection.clone();

        self.executor.spawn(async move {
            let sandboxed_terminal_temp_dirs = {
                let connection = connection.lock();

                let mut select = connection.select_bound::<(), (DataType, Vec<u8>)>(indoc! {"
                    SELECT data_type, data FROM threads
                "})?;

                let sandboxed_terminal_temp_dirs = select(())?
                    .into_iter()
                    .filter_map(|(data_type, data)| {
                        Self::sandboxed_terminal_temp_dir(data_type, data)
                    })
                    .collect::<Vec<_>>();

                let mut delete = connection.exec_bound::<()>(indoc! {"
                    DELETE FROM threads
                "})?;

                delete(())?;

                sandboxed_terminal_temp_dirs
            };

            for temp_dir in sandboxed_terminal_temp_dirs {
                Self::remove_sandboxed_terminal_temp_dir(temp_dir);
            }

            Ok(())
        })
    }
}

/// Records a completed request's token usage into the statistics store.
pub fn record_token_usage(cx: &mut App, event: TokenUsageEvent) -> Task<Result<()>> {
    let database = ThreadsDatabase::connect(cx);
    cx.background_spawn(async move {
        let database = database.await.map_err(|error| anyhow::anyhow!("{error}"))?;
        database.record_token_usage_event(event).await
    })
}

/// Grouping dimensions available for the statistics breakdowns.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TokenUsageGroupBy {
    Model,
    Provider,
    Thread,
}

impl TokenUsageGroupBy {
    fn column(self) -> &'static str {
        match self {
            TokenUsageGroupBy::Model => "model",
            TokenUsageGroupBy::Provider => "provider",
            TokenUsageGroupBy::Thread => "thread_id",
        }
    }
}

/// One aggregated row of a statistics breakdown.
#[derive(Clone, Debug)]
pub struct TokenUsageBreakdownRow {
    /// Grouping key; `None` groups rows recorded before the column existed.
    pub key: Option<String>,
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub saved_tokens: u64,
}

impl TokenUsageBreakdownRow {
    pub fn prompt_tokens(&self) -> u64 {
        self.input_tokens + self.cache_read_tokens + self.cache_write_tokens
    }

    pub fn total_tokens(&self) -> u64 {
        self.prompt_tokens() + self.output_tokens
    }
}

/// Returns usage aggregated by one dimension since `since` (unix seconds).
pub fn token_usage_breakdown(
    cx: &mut App,
    since: i64,
    group_by: TokenUsageGroupBy,
) -> Task<Result<Vec<TokenUsageBreakdownRow>>> {
    let database = ThreadsDatabase::connect(cx);
    cx.background_spawn(async move {
        let database = database.await.map_err(|error| anyhow::anyhow!("{error}"))?;
        database.token_usage_breakdown(since, group_by).await
    })
}

/// Lists thread metadata (id and title) so usage can be attributed to chats.
pub fn list_thread_titles(cx: &mut App) -> Task<Result<Vec<DbThreadMetadata>>> {
    let database = ThreadsDatabase::connect(cx);
    cx.background_spawn(async move {
        let database = database.await.map_err(|error| anyhow::anyhow!("{error}"))?;
        database.list_threads().await
    })
}

/// Deletes every recorded token usage event.
pub fn clear_token_usage(cx: &mut App) -> Task<Result<()>> {
    let database = ThreadsDatabase::connect(cx);
    cx.background_spawn(async move {
        let database = database.await.map_err(|error| anyhow::anyhow!("{error}"))?;
        database.clear_token_usage_events().await
    })
}

/// Returns token usage aggregated into fixed-width time buckets, newest last.
pub fn token_usage_buckets(
    cx: &mut App,
    since: i64,
    bucket_seconds: i64,
) -> Task<Result<Vec<TokenUsageBucket>>> {
    let database = ThreadsDatabase::connect(cx);
    cx.background_spawn(async move {
        let database = database.await.map_err(|error| anyhow::anyhow!("{error}"))?;
        database.token_usage_buckets(since, bucket_seconds).await
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::v1::schema as acp;
    use chrono::{DateTime, TimeZone, Utc};
    use collections::HashMap;
    use gpui::TestAppContext;
    use std::sync::Arc;

    #[test]
    fn test_shared_thread_roundtrip() {
        let original = SharedThread {
            title: "Test Thread".into(),
            messages: vec![],
            updated_at: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            model: None,
            version: SharedThread::VERSION.to_string(),
        };

        let bytes = original.to_bytes().expect("Failed to serialize");
        let restored = SharedThread::from_bytes(&bytes).expect("Failed to deserialize");

        assert_eq!(restored.title, original.title);
        assert_eq!(restored.version, original.version);
        assert_eq!(restored.updated_at, original.updated_at);
    }

    fn session_id(value: &str) -> acp_v2::SessionId {
        acp_v2::SessionId::new(Arc::<str>::from(value))
    }

    #[test]
    fn test_session_metadata_preserves_backing_storage() {
        let storage: Arc<str> = Arc::from("  session/雪:\"quoted\"\\opaque  ");
        let metadata = DbThreadMetadata {
            id: acp_v2::SessionId::new(storage.clone()),
            parent_session_id: None,
            title: "Opaque session".into(),
            updated_at: Utc::now(),
            created_at: None,
            folder_paths: PathList::default(),
        };
        let session_info = acp_thread::AgentSessionInfo::from(&metadata);

        assert_eq!(session_info.session_id.0.as_ref(), storage.as_ref());
        assert!(Arc::ptr_eq(&session_info.session_id.0, &storage));
        assert!(Arc::ptr_eq(&metadata.id.0, &storage));
    }

    #[gpui::test]
    async fn test_session_ids_preserve_legacy_persistence(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).expect("create database");
        let thread_id = session_id("  session/雪:\"quoted\"\\opaque  ");
        let parent_id = session_id("  parent/雪:\"quoted\"\\opaque  ");
        let legacy_json = serde_json::json!({
            "version": "0.3.0",
            "title": "Legacy subagent",
            "messages": [],
            "updated_at": "2024-01-01T00:00:00Z",
            "subagent_context": {
                "parent_thread_id": parent_id.0.as_ref(),
                "depth": 1,
            },
        });
        {
            let connection = database.connection.lock();
            let mut insert = connection
                .exec_bound::<(Arc<str>, Arc<str>, Vec<u8>)>(indoc! {"
                    INSERT INTO threads (id, parent_id, summary, updated_at, data_type, data)
                    VALUES (?1, ?2, 'Legacy subagent', '2024-01-01T00:00:00Z', 'json', ?3)
                "})
                .expect("prepare legacy row");
            insert((
                thread_id.0.clone(),
                parent_id.0.clone(),
                serde_json::to_vec(&legacy_json).expect("encode legacy JSON"),
            ))
            .expect("insert legacy row");
        }

        let restored = database
            .load_thread(thread_id.clone())
            .await
            .expect("load legacy row")
            .expect("legacy row exists");
        assert_eq!(
            serde_json::to_value(&restored.subagent_context).expect("encode restored context"),
            legacy_json["subagent_context"]
        );
        let entries = database.list_threads().await.expect("list legacy row");
        let metadata = entries.first().expect("legacy metadata exists");
        assert_eq!(metadata.id, thread_id);
        assert_eq!(metadata.parent_session_id.as_ref(), Some(&parent_id));

        database
            .save_thread(thread_id.clone(), restored, PathList::default())
            .await
            .expect("save with v2 session IDs");
        {
            let connection = database.connection.lock();
            let mut select = connection
                .select_bound::<Arc<str>, (String, String, DataType, Vec<u8>)>(
                    "SELECT id, parent_id, data_type, data FROM threads WHERE id = ?",
                )
                .expect("prepare saved row query");
            let rows = select(thread_id.0.clone()).expect("query saved row");
            let (stored_id, stored_parent_id, data_type, data) =
                rows.first().expect("saved row exists");
            assert_eq!(stored_id, thread_id.0.as_ref());
            assert_eq!(stored_parent_id, parent_id.0.as_ref());
            assert_eq!(*data_type, DataType::Zstd);
            let json = zstd::decode_all(data.as_slice()).expect("decompress saved JSON");
            let saved_json: serde_json::Value =
                serde_json::from_slice(&json).expect("decode saved JSON");
            assert_eq!(saved_json["version"], legacy_json["version"]);
            assert_eq!(
                saved_json["subagent_context"],
                legacy_json["subagent_context"]
            );
        }

        database
            .delete_thread(thread_id.clone())
            .await
            .expect("delete using v2 ID");
        assert!(
            database
                .load_thread(thread_id)
                .await
                .expect("load deleted row")
                .is_none()
        );
    }

    fn make_thread(title: &str, updated_at: DateTime<Utc>) -> DbThread {
        DbThread {
            title: title.to_string().into(),
            messages: Vec::new(),
            updated_at,
            detailed_summary: None,
            initial_project_snapshot: None,
            cumulative_token_usage: Default::default(),
            request_token_usage: HashMap::default(),
            model: None,
            profile: None,
            subagent_context: None,
            speed: None,
            thinking_enabled: false,
            thinking_effort: None,
            draft_prompt: None,
            ui_scroll_position: None,
            sandboxed_terminal_temp_dir: None,
            sandbox_grants: DbSandboxGrants::default(),
        }
    }

    #[gpui::test]
    async fn test_save_thread_stores_versioned_json(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();
        let thread_id = session_id("versioned-thread");
        let mut thread = make_thread(
            "Thread \"雪\"",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        thread.detailed_summary = Some(
            "A long summary with \"quotes\", \\ and 雪\n"
                .repeat(8192)
                .into(),
        );
        let expected = serde_json::to_value(&thread).unwrap();

        database
            .save_thread(thread_id.clone(), thread, PathList::default())
            .await
            .unwrap();

        let (data_type, data) = {
            let connection = database.connection.lock();
            let mut select = connection
                .select_bound::<Arc<str>, (DataType, Vec<u8>)>(
                    "SELECT data_type, data FROM threads WHERE id = ?",
                )
                .unwrap();
            select(thread_id.0).unwrap().into_iter().next().unwrap()
        };
        assert_eq!(data_type, DataType::Zstd);
        let json = zstd::decode_all(data.as_slice()).unwrap();
        let saved_json: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(saved_json["version"], DbThread::VERSION);
        let restored = DbThread::from_json(&json).unwrap();
        assert_eq!(serde_json::to_value(restored).unwrap(), expected);
    }

    #[gpui::test]
    async fn test_list_threads_orders_by_created_at(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();

        let older_id = session_id("thread-a");
        let newer_id = session_id("thread-b");

        let older_thread = make_thread(
            "Thread A",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        let newer_thread = make_thread(
            "Thread B",
            Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap(),
        );

        database
            .save_thread(older_id.clone(), older_thread, PathList::default())
            .await
            .unwrap();
        database
            .save_thread(newer_id.clone(), newer_thread, PathList::default())
            .await
            .unwrap();

        let entries = database.list_threads().await.unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, newer_id);
        assert_eq!(entries[1].id, older_id);
    }

    #[gpui::test]
    async fn test_list_threads_reads_only_the_index(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();

        let plan = database
            .connection
            .lock()
            .select::<(i32, i32, i32, String)>(&format!("EXPLAIN QUERY PLAN {LIST_THREADS_QUERY}"))
            .unwrap()()
        .unwrap();
        let details = plan
            .into_iter()
            .map(|(_, _, _, detail)| detail)
            .collect::<Vec<_>>();
        assert_eq!(
            details,
            ["SCAN threads USING COVERING INDEX threads_list_metadata"]
        );
    }

    #[gpui::test]
    async fn test_save_thread_replaces_metadata(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();

        let thread_id = session_id("thread-a");
        let original_thread = make_thread(
            "Thread A",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        let updated_thread = make_thread(
            "Thread B",
            Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap(),
        );

        database
            .save_thread(thread_id.clone(), original_thread, PathList::default())
            .await
            .unwrap();
        database
            .save_thread(thread_id.clone(), updated_thread, PathList::default())
            .await
            .unwrap();

        let entries = database.list_threads().await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, thread_id);
        assert_eq!(entries[0].title.as_ref(), "Thread B");
        assert_eq!(
            entries[0].updated_at,
            Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()
        );
        assert!(
            entries[0].created_at.is_some(),
            "created_at should be populated"
        );
    }

    #[test]
    fn test_subagent_context_defaults_to_none() {
        let json = r#"{
            "title": "Old Thread",
            "messages": [],
            "updated_at": "2024-01-01T00:00:00Z"
        }"#;

        let db_thread: DbThread = serde_json::from_str(json).expect("Failed to deserialize");

        assert!(
            db_thread.subagent_context.is_none(),
            "Legacy threads without subagent_context should default to None"
        );
    }

    #[test]
    fn test_draft_prompt_defaults_to_none() {
        let json = r#"{
            "title": "Old Thread",
            "messages": [],
            "updated_at": "2024-01-01T00:00:00Z"
        }"#;

        let db_thread: DbThread = serde_json::from_str(json).expect("Failed to deserialize");

        assert!(
            db_thread.draft_prompt.is_none(),
            "Legacy threads without draft_prompt field should default to None"
        );
    }

    #[gpui::test]
    async fn test_draft_prompt_preserves_legacy_and_v2_content(cx: &mut TestAppContext) {
        let legacy_draft = serde_json::to_value(vec![
            acp::ContentBlock::Text(acp::TextContent::new("legacy draft")),
            acp::ContentBlock::ResourceLink(acp::ResourceLink::new("file", "file:///a.md")),
        ])
        .expect("serialize v1 draft");
        let mut thread: DbThread = serde_json::from_value(serde_json::json!({
            "title": "Draft Thread",
            "messages": [],
            "updated_at": "2024-01-01T00:00:00Z",
            "draft_prompt": legacy_draft,
        }))
        .expect("decode legacy thread with v2 draft blocks");
        assert_eq!(
            serde_json::to_value(&thread.draft_prompt).expect("serialize decoded draft"),
            legacy_draft
        );

        let extension = serde_json::json!({
            "type": "_draft_card",
            "payload": {"items": [1, {"enabled": true}], "optional": null},
            "_meta": {"source": "draft", "nested": {"version": 2}},
        });
        let extension_block: acp_v2::ContentBlock =
            serde_json::from_value(extension.clone()).expect("decode v2-only draft block");
        assert!(matches!(extension_block, acp_v2::ContentBlock::Other(_)));
        thread
            .draft_prompt
            .as_mut()
            .expect("legacy draft exists")
            .push(extension_block);
        let mut expected = legacy_draft.as_array().expect("draft is an array").clone();
        expected.push(extension);

        let database = ThreadsDatabase::new(cx.executor()).expect("create database");
        let thread_id = session_id("draft-thread");
        database
            .save_thread(thread_id.clone(), thread, PathList::default())
            .await
            .expect("save mixed-version draft");
        let restored = database
            .load_thread(thread_id)
            .await
            .expect("load draft")
            .expect("saved thread exists");
        assert_eq!(
            serde_json::to_value(restored.draft_prompt).expect("serialize restored draft"),
            serde_json::Value::Array(expected)
        );
    }

    #[test]
    fn test_sandboxed_terminal_temp_dir_defaults_to_none() {
        let json = r#"{
            "title": "Old Thread",
            "messages": [],
            "updated_at": "2024-01-01T00:00:00Z"
        }"#;

        let db_thread: DbThread = serde_json::from_str(json).expect("Failed to deserialize");

        assert!(
            db_thread.sandboxed_terminal_temp_dir.is_none(),
            "Legacy threads without sandboxed_terminal_temp_dir should default to None"
        );
    }

    #[test]
    fn test_sandbox_grants_default_when_absent() {
        let json = r#"{
            "title": "Old Thread",
            "messages": [],
            "updated_at": "2024-01-01T00:00:00Z"
        }"#;

        let db_thread: DbThread = serde_json::from_str(json).expect("Failed to deserialize");

        assert_eq!(
            db_thread.sandbox_grants,
            DbSandboxGrants::default(),
            "Legacy threads without sandbox_grants should default to empty grants"
        );
    }

    #[gpui::test]
    async fn test_sandbox_grants_roundtrip_through_save_load(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();
        let thread_id = session_id("sandbox-grants-thread");
        let mut thread = make_thread(
            "Sandbox Grants Thread",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        let grants = DbSandboxGrants {
            write_paths: vec![
                // A legacy bare-string grant (no resolved canonical) and a grant
                // carrying its resolved canonical, to exercise both forms of the
                // string-or-object round-trip.
                settings::GrantedWritePath::from_requested(PathBuf::from("/tmp/build")),
                settings::GrantedWritePath::resolved(
                    PathBuf::from("/tmp/link"),
                    PathBuf::from("/tmp/real"),
                ),
            ],
            network_hosts: vec!["github.com".to_string(), "*.npmjs.org".to_string()],
            network_any_host: false,
            allow_fs_write_all: false,
            unsandboxed: true,
            sandbox_fallback: true,
        };
        thread.sandbox_grants = grants.clone();

        database
            .save_thread(thread_id.clone(), thread, PathList::default())
            .await
            .unwrap();

        let loaded = database
            .load_thread(thread_id)
            .await
            .unwrap()
            .expect("thread should exist");
        assert_eq!(loaded.sandbox_grants, grants);
    }

    #[gpui::test]
    async fn test_sandboxed_terminal_temp_dir_roundtrips_through_save_load(
        cx: &mut TestAppContext,
    ) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();
        let thread_id = session_id("sandbox-temp-dir-thread");
        let temp_dir = tempfile::Builder::new()
            .prefix("zed-agent-terminal-test-")
            .tempdir()
            .unwrap()
            .keep();
        let mut thread = make_thread(
            "Sandbox Temp Dir Thread",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        thread.sandboxed_terminal_temp_dir = Some(temp_dir.clone());

        database
            .save_thread(thread_id.clone(), thread, PathList::default())
            .await
            .unwrap();

        let loaded = database
            .load_thread(thread_id)
            .await
            .unwrap()
            .expect("thread should exist");
        assert_eq!(loaded.sandboxed_terminal_temp_dir, Some(temp_dir.clone()));
        std::fs::remove_dir_all(temp_dir).unwrap();
    }

    #[gpui::test]
    async fn test_delete_thread_removes_sandboxed_terminal_temp_dir(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();
        let thread_id = session_id("sandbox-temp-dir-delete-thread");
        let temp_dir = tempfile::Builder::new()
            .prefix("zed-agent-terminal-test-")
            .tempdir()
            .unwrap()
            .keep();
        std::fs::write(temp_dir.join("sentinel"), b"content").unwrap();
        let mut thread = make_thread(
            "Sandbox Temp Dir Delete Thread",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        thread.sandboxed_terminal_temp_dir = Some(temp_dir.clone());

        database
            .save_thread(thread_id.clone(), thread, PathList::default())
            .await
            .unwrap();
        database.delete_thread(thread_id).await.unwrap();

        assert!(!temp_dir.exists());
    }

    #[gpui::test]
    async fn test_delete_thread_deletes_subagent_threads(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();

        let parent_id = session_id("parent-thread");
        let child_id = session_id("child-thread");
        let grandchild_id = session_id("grandchild-thread");
        let unrelated_id = session_id("unrelated-thread");

        let parent_thread = make_thread(
            "Parent Thread",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );

        let mut child_thread = make_thread(
            "Child Subagent Thread",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        child_thread.subagent_context = Some(crate::SubagentContext {
            parent_thread_id: parent_id.clone(),
            depth: 1,
            outcomes: Vec::new(),
        });

        let mut grandchild_thread = make_thread(
            "Grandchild Subagent Thread",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        grandchild_thread.subagent_context = Some(crate::SubagentContext {
            parent_thread_id: child_id.clone(),
            depth: 2,
            outcomes: Vec::new(),
        });

        let unrelated_thread = make_thread(
            "Unrelated Thread",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );

        for (id, thread) in [
            (parent_id.clone(), parent_thread),
            (child_id.clone(), child_thread),
            (grandchild_id.clone(), grandchild_thread),
            (unrelated_id.clone(), unrelated_thread),
        ] {
            database
                .save_thread(id, thread, PathList::default())
                .await
                .unwrap();
        }

        database.delete_thread(parent_id.clone()).await.unwrap();

        let remaining = database.list_threads().await.unwrap();
        let remaining_ids: Vec<_> = remaining.iter().map(|thread| thread.id.clone()).collect();
        assert_eq!(remaining_ids, vec![unrelated_id]);
    }

    #[gpui::test]
    async fn test_subagent_context_roundtrips_through_save_load(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();

        let parent_id = session_id("parent-thread");
        let child_id = session_id("child-thread");

        let mut child_thread = make_thread(
            "Subagent Thread",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        child_thread.subagent_context = Some(crate::SubagentContext {
            parent_thread_id: parent_id.clone(),
            depth: 2,
            outcomes: Vec::new(),
        });

        database
            .save_thread(child_id.clone(), child_thread, PathList::default())
            .await
            .unwrap();

        let loaded = database
            .load_thread(child_id)
            .await
            .unwrap()
            .expect("thread should exist");

        let context = loaded
            .subagent_context
            .expect("subagent_context should be restored");
        assert_eq!(context.parent_thread_id, parent_id);
        assert_eq!(context.depth, 2);
    }

    #[gpui::test]
    async fn test_non_subagent_thread_has_no_subagent_context(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();

        let thread_id = session_id("regular-thread");
        let thread = make_thread(
            "Regular Thread",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );

        database
            .save_thread(thread_id.clone(), thread, PathList::default())
            .await
            .unwrap();

        let loaded = database
            .load_thread(thread_id)
            .await
            .unwrap()
            .expect("thread should exist");

        assert!(
            loaded.subagent_context.is_none(),
            "Regular threads should have no subagent_context"
        );
    }

    #[gpui::test]
    async fn test_folder_paths_roundtrip(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();

        let thread_id = session_id("folder-thread");
        let thread = make_thread(
            "Folder Thread",
            Utc.with_ymd_and_hms(2024, 6, 15, 12, 0, 0).unwrap(),
        );

        let folder_paths = PathList::new(&[
            std::path::PathBuf::from("/home/user/project-a"),
            std::path::PathBuf::from("/home/user/project-b"),
        ]);

        database
            .save_thread(thread_id.clone(), thread, folder_paths.clone())
            .await
            .unwrap();

        let threads = database.list_threads().await.unwrap();
        assert_eq!(threads.len(), 1);
    }

    #[gpui::test]
    async fn test_folder_paths_empty_when_not_set(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();

        let thread_id = session_id("no-folder-thread");
        let thread = make_thread(
            "No Folder Thread",
            Utc.with_ymd_and_hms(2024, 6, 15, 12, 0, 0).unwrap(),
        );

        database
            .save_thread(thread_id.clone(), thread, PathList::default())
            .await
            .unwrap();

        let threads = database.list_threads().await.unwrap();
        assert_eq!(threads.len(), 1);
    }

    #[test]
    fn test_scroll_position_defaults_to_none() {
        let json = r#"{
            "title": "Old Thread",
            "messages": [],
            "updated_at": "2024-01-01T00:00:00Z"
        }"#;

        let db_thread: DbThread = serde_json::from_str(json).expect("Failed to deserialize");

        assert!(
            db_thread.ui_scroll_position.is_none(),
            "Legacy threads without scroll_position field should default to None"
        );
    }

    #[gpui::test]
    async fn test_scroll_position_roundtrips_through_save_load(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();

        let thread_id = session_id("thread-with-scroll");

        let mut thread = make_thread(
            "Thread With Scroll",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        thread.ui_scroll_position = Some(SerializedScrollPosition {
            item_ix: 42,
            offset_in_item: 13.5,
        });

        database
            .save_thread(thread_id.clone(), thread, PathList::default())
            .await
            .unwrap();

        let loaded = database
            .load_thread(thread_id)
            .await
            .unwrap()
            .expect("thread should exist");

        let scroll = loaded
            .ui_scroll_position
            .expect("scroll_position should be restored");
        assert_eq!(scroll.item_ix, 42);
        assert!((scroll.offset_in_item - 13.5).abs() < f32::EPSILON);
    }

    fn usage_event(request_id: &str) -> TokenUsageEvent {
        TokenUsageEvent {
            request_id: request_id.into(),
            created_at: 1_000,
            input_tokens: 10,
            output_tokens: 5,
            cache_read_tokens: 100,
            cache_write_tokens: 1,
            saved_tokens: 0,
            model: Some("model-a".into()),
            thread_id: Some("thread-1".into()),
            provider: Some("provider-a".into()),
        }
    }

    #[gpui::test]
    async fn test_token_usage_events_are_idempotent_per_request(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();

        // A streamed request reports usage on many chunks; the same request id
        // must collapse into a single row instead of inflating the totals.
        database
            .record_token_usage_event(usage_event("t1:p1:1"))
            .await
            .unwrap();
        let mut growing = usage_event("t1:p1:1");
        growing.input_tokens = 12;
        growing.output_tokens = 7;
        database
            .record_token_usage_event(growing)
            .await
            .unwrap();

        let rows = database
            .token_usage_breakdown(0, TokenUsageGroupBy::Thread)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "one request id must yield one row");
        assert_eq!(rows[0].requests, 1);
        assert_eq!(rows[0].input_tokens, 12, "retries keep the largest value");
        assert_eq!(rows[0].output_tokens, 7);
    }

    #[gpui::test]
    async fn test_token_usage_breakdown_groups_by_model_and_provider(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();

        let mut second = usage_event("t1:p1:2");
        second.model = Some("model-b".into());
        second.provider = Some("provider-b".into());
        second.thread_id = Some("thread-2".into());

        database
            .record_token_usage_event(usage_event("t1:p1:1"))
            .await
            .unwrap();
        database
            .record_token_usage_event(second)
            .await
            .unwrap();

        let by_model = database
            .token_usage_breakdown(0, TokenUsageGroupBy::Model)
            .await
            .unwrap();
        assert_eq!(by_model.len(), 2);
        assert!(by_model.iter().all(|row| row.requests == 1));

        let by_provider = database
            .token_usage_breakdown(0, TokenUsageGroupBy::Provider)
            .await
            .unwrap();
        let provider_a = by_provider
            .iter()
            .find(|row| row.key.as_deref() == Some("provider-a"))
            .expect("provider-a row");
        assert_eq!(provider_a.requests, 1);
        assert_eq!(provider_a.cache_read_tokens, 100);

        let by_thread = database
            .token_usage_breakdown(0, TokenUsageGroupBy::Thread)
            .await
            .unwrap();
        assert_eq!(by_thread.len(), 2);

        database.clear_token_usage_events().await.unwrap();
        let after_clear = database
            .token_usage_breakdown(0, TokenUsageGroupBy::Thread)
            .await
            .unwrap();
        assert!(after_clear.is_empty(), "clearing must drop all rows");
    }
}
