use crate::{DbThread, DbThreadMetadata, ThreadsDatabase};
use agent_client_protocol::v2::schema as acp_v2;
use agent_settings::{AgentSettings, ThreadMaintenanceSettings};
use anyhow::{Result, anyhow};
use chrono::{Duration as ChronoDuration, Utc};
use futures::{FutureExt, future::Shared};
use gpui::{App, Context, Entity, Global, Task, prelude::*};
use settings::Settings;
use std::sync::Arc;
use util::path_list::PathList;

/// Rough bytes-per-token estimate used to bound stored thread size. Matches the
/// estimate used when building model requests.
const STORED_THREAD_BYTES_PER_TOKEN: u64 = 4;

struct GlobalThreadStore(Entity<ThreadStore>);

impl Global for GlobalThreadStore {}

pub struct ThreadStore {
    threads: Vec<DbThreadMetadata>,
    reload_task: Shared<Task<()>>,
    _maintenance_task: Task<()>,
}

impl ThreadStore {
    pub fn init_global(cx: &mut App) {
        let thread_store = cx.new(|cx| Self::new(cx));
        cx.set_global(GlobalThreadStore(thread_store));
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalThreadStore>().0.clone()
    }

    pub fn try_global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalThreadStore>().map(|g| g.0.clone())
    }

    pub fn new(cx: &mut Context<Self>) -> Self {
        let reload_task = Self::spawn_reload(cx);
        let maintenance_task = Self::spawn_maintenance(cx);
        Self {
            threads: Vec::new(),
            reload_task,
            _maintenance_task: maintenance_task,
        }
    }

    /// Resolves when the most recently initiated reload has completed.
    /// Callers that need to read `entries()` and can't tolerate the initial
    /// empty state must await this before reading.
    pub fn reload_task(&self) -> Shared<Task<()>> {
        self.reload_task.clone()
    }

    pub fn thread_from_session_id(
        &self,
        session_id: &acp_v2::SessionId,
    ) -> Option<&DbThreadMetadata> {
        self.threads.iter().find(|thread| &thread.id == session_id)
    }

    pub fn load_thread(
        &mut self,
        id: acp_v2::SessionId,
        cx: &mut Context<Self>,
    ) -> Task<Result<Option<DbThread>>> {
        let database_future = ThreadsDatabase::connect(cx);
        cx.background_spawn(async move {
            let database = database_future.await.map_err(|err| anyhow!(err))?;
            database.load_thread(id).await
        })
    }

    pub fn save_thread(
        &mut self,
        id: acp_v2::SessionId,
        thread: crate::DbThread,
        folder_paths: PathList,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let database_future = ThreadsDatabase::connect(cx);
        cx.spawn(async move |this, cx| {
            let database = database_future.await.map_err(|err| anyhow!(err))?;
            database.save_thread(id, thread, folder_paths).await?;
            this.update(cx, |this, cx| this.reload(cx))
        })
    }

    pub fn delete_thread(
        &mut self,
        id: acp_v2::SessionId,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let database_future = ThreadsDatabase::connect(cx);
        cx.spawn(async move |this, cx| {
            let database = database_future.await.map_err(|err| anyhow!(err))?;
            database.delete_thread(id.clone()).await?;
            this.update(cx, |this, cx| this.reload(cx))
        })
    }

    pub fn delete_threads(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        let database_future = ThreadsDatabase::connect(cx);
        cx.spawn(async move |this, cx| {
            let database = database_future.await.map_err(|err| anyhow!(err))?;
            database.delete_threads().await?;
            this.update(cx, |this, cx| this.reload(cx))
        })
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.reload_task = Self::spawn_reload(cx);
    }

    fn spawn_reload(cx: &mut Context<Self>) -> Shared<Task<()>> {
        let database_connection = ThreadsDatabase::connect(cx);
        cx.spawn(async move |this, cx| {
            let Ok(database) = database_connection.await.map_err(|err| anyhow!(err)) else {
                return;
            };
            let Ok(all_threads) = database.list_threads().await else {
                return;
            };
            this.update(cx, |this, cx| {
                this.threads.clear();
                for thread in all_threads {
                    if thread.parent_session_id.is_some() {
                        continue;
                    }
                    this.threads.push(thread);
                }
                cx.notify();
            })
            .ok();
        })
        .shared()
    }

    fn spawn_maintenance(cx: &mut Context<Self>) -> Task<()> {
        if cfg!(test) {
            return Task::ready(());
        }
        let settings = AgentSettings::get_global(cx).thread_maintenance.clone();
        if !settings.enabled {
            return Task::ready(());
        }

        let database_future = ThreadsDatabase::connect(cx);
        cx.spawn(async move |this, cx| {
            let interval = settings.interval_hours;
            loop {
                let database = match database_future.clone().await {
                    Ok(database) => database,
                    Err(error) => {
                        log::error!("thread maintenance: failed to open database: {error:#}");
                        return;
                    }
                };
                if let Err(error) = Self::run_maintenance(&database, &settings).await {
                    log::warn!("thread maintenance failed: {error:#}");
                }
                if let Err(error) = this.update(cx, |this, cx| this.reload(cx)) {
                    log::warn!("thread maintenance: failed to reload threads: {error:#}");
                    return;
                }

                if interval == 0 {
                    break;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(
                        interval.saturating_mul(3600),
                    ))
                    .await;
            }
        })
    }

    async fn run_maintenance(
        database: &ThreadsDatabase,
        settings: &ThreadMaintenanceSettings,
    ) -> Result<()> {
        let mut threads: Vec<DbThreadMetadata> = database
            .list_threads()
            .await?
            .into_iter()
            .filter(|thread| thread.parent_session_id.is_none())
            .collect();

        if settings.max_thread_age_days > 0 {
            let cutoff = Utc::now()
                - ChronoDuration::days(settings.max_thread_age_days.min(i64::MAX as u64) as i64);
            let mut retained = Vec::with_capacity(threads.len());
            for thread in threads {
                if thread.updated_at < cutoff {
                    match database.delete_thread(thread.id.clone()).await {
                        Ok(()) => {
                            log::info!(
                                "thread maintenance: deleted thread {} older than {} days",
                                thread.id.0,
                                settings.max_thread_age_days
                            );
                        }
                        Err(error) => {
                            log::warn!(
                                "thread maintenance: failed to delete old thread {}: {error:#}",
                                thread.id.0
                            );
                            retained.push(thread);
                        }
                    }
                } else {
                    retained.push(thread);
                }
            }
            threads = retained;
        }

        if settings.max_threads > 0 && threads.len() > settings.max_threads {
            threads.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
            let excess = threads.split_off(settings.max_threads);
            for thread in excess {
                match database.delete_thread(thread.id.clone()).await {
                    Ok(()) => {
                        log::info!(
                            "thread maintenance: deleted thread {} beyond the {} thread limit",
                            thread.id.0,
                            settings.max_threads
                        );
                    }
                    Err(error) => {
                        log::warn!(
                            "thread maintenance: failed to delete excess thread {}: {error:#}",
                            thread.id.0
                        );
                    }
                }
            }
        }

        if settings.shrink_threads_over_tokens > 0 {
            for thread in &threads {
                let Some(stored) = database.load_thread(thread.id.clone()).await? else {
                    continue;
                };
                if let Some(shrunk) = shrink_thread_messages(
                    &stored.messages,
                    settings.keep_recent_messages,
                    settings.shrink_threads_over_tokens,
                ) {
                    let mut stored = stored;
                    stored.messages = shrunk;
                    match database
                        .save_thread(thread.id.clone(), stored, thread.folder_paths.clone())
                        .await
                    {
                        Ok(()) => {
                            log::info!(
                                "thread maintenance: shrank thread {} to within {} tokens",
                                thread.id.0,
                                settings.shrink_threads_over_tokens
                            );
                        }
                        Err(error) => {
                            log::warn!(
                                "thread maintenance: failed to save shrank thread {}: {error:#}",
                                thread.id.0
                            );
                        }
                    }
                }
            }
        }

        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.threads.is_empty()
    }

    pub fn entries(&self) -> impl Iterator<Item = DbThreadMetadata> + '_ {
        self.threads.iter().cloned()
    }

    pub fn entry_ids(&self) -> impl Iterator<Item = acp_v2::SessionId> + '_ {
        self.threads.iter().map(|t| t.id.clone())
    }
}

fn stored_message_tokens(message: &crate::Message) -> u64 {
    (message.to_markdown().len() as u64).div_ceil(STORED_THREAD_BYTES_PER_TOKEN)
}

/// Drops the oldest user turns of a stored thread until its estimated token
/// size fits within `budget_tokens`, always keeping at least
/// `keep_recent_messages` trailing messages and starting the remaining history
/// on a user message. Returns `None` when no shrink is needed or possible.
fn shrink_thread_messages(
    messages: &[Arc<crate::Message>],
    keep_recent_messages: usize,
    budget_tokens: u64,
) -> Option<Vec<Arc<crate::Message>>> {
    let token_counts: Vec<u64> = messages
        .iter()
        .map(|message| stored_message_tokens(message))
        .collect();
    let total_tokens: u64 = token_counts.iter().copied().sum();
    if total_tokens <= budget_tokens || messages.len() <= keep_recent_messages {
        return None;
    }

    let max_cut = messages.len().saturating_sub(keep_recent_messages.max(1));
    let mut suffix_tokens: Vec<u64> = vec![0; messages.len() + 1];
    for index in (0..messages.len()).rev() {
        suffix_tokens[index] = suffix_tokens[index + 1].saturating_add(token_counts[index]);
    }

    let mut cut = None;
    for index in 1..=max_cut {
        if !matches!(&*messages[index], crate::Message::User(_)) {
            continue;
        }
        if suffix_tokens[index] <= budget_tokens {
            cut = Some(index);
            break;
        }
    }

    let cut = match cut {
        Some(cut) => cut,
        None => (1..=max_cut)
            .rev()
            .find(|index| matches!(&*messages[*index], crate::Message::User(_)))?,
    };

    let mut shrunk = Vec::with_capacity(messages.len() - cut + 1);
    shrunk.push(Arc::new(crate::Message::Compaction(
        crate::CompactionInfo::Summary(
            "Earlier messages in this chat were automatically pruned to keep the stored history small."
                .into(),
        ),
    )));
    shrunk.extend(messages[cut..].iter().cloned());
    Some(shrunk)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, TimeZone, Utc};
    use collections::HashMap;
    use gpui::TestAppContext;
    use std::sync::Arc;

    fn session_id(value: &str) -> acp_v2::SessionId {
        acp_v2::SessionId::new(Arc::<str>::from(value))
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
            sandbox_grants: Default::default(),
        }
    }

    #[gpui::test]
    async fn test_entries_are_sorted_by_updated_at(cx: &mut TestAppContext) {
        let thread_store = cx.new(|cx| ThreadStore::new(cx));
        cx.run_until_parked();

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

        let save_older = thread_store.update(cx, |store, cx| {
            store.save_thread(older_id.clone(), older_thread, PathList::default(), cx)
        });
        save_older.await.unwrap();

        let save_newer = thread_store.update(cx, |store, cx| {
            store.save_thread(newer_id.clone(), newer_thread, PathList::default(), cx)
        });
        save_newer.await.unwrap();

        cx.run_until_parked();

        let entries: Vec<_> = thread_store.read_with(cx, |store, _cx| store.entries().collect());
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, newer_id);
        assert_eq!(entries[1].id, older_id);
    }

    #[gpui::test]
    async fn test_delete_threads_clears_entries(cx: &mut TestAppContext) {
        let thread_store = cx.new(|cx| ThreadStore::new(cx));
        cx.run_until_parked();

        let thread_id = session_id("thread-a");
        let thread = make_thread(
            "Thread A",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );

        let save_task = thread_store.update(cx, |store, cx| {
            store.save_thread(thread_id, thread, PathList::default(), cx)
        });
        save_task.await.unwrap();

        cx.run_until_parked();
        assert!(!thread_store.read_with(cx, |store, _cx| store.is_empty()));

        let delete_task = thread_store.update(cx, |store, cx| store.delete_threads(cx));
        delete_task.await.unwrap();
        cx.run_until_parked();

        assert!(thread_store.read_with(cx, |store, _cx| store.is_empty()));
    }

    #[gpui::test]
    async fn test_delete_thread_removes_only_target(cx: &mut TestAppContext) {
        let thread_store = cx.new(|cx| ThreadStore::new(cx));
        cx.run_until_parked();

        let first_id = session_id("thread-a");
        let second_id = session_id("thread-b");

        let first_thread = make_thread(
            "Thread A",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        let second_thread = make_thread(
            "Thread B",
            Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap(),
        );

        let save_first = thread_store.update(cx, |store, cx| {
            store.save_thread(first_id.clone(), first_thread, PathList::default(), cx)
        });
        save_first.await.unwrap();
        let save_second = thread_store.update(cx, |store, cx| {
            store.save_thread(second_id.clone(), second_thread, PathList::default(), cx)
        });
        save_second.await.unwrap();
        cx.run_until_parked();

        let delete_task =
            thread_store.update(cx, |store, cx| store.delete_thread(first_id.clone(), cx));
        delete_task.await.unwrap();
        cx.run_until_parked();

        let entries: Vec<_> = thread_store.read_with(cx, |store, _cx| store.entries().collect());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, second_id);
    }

    #[gpui::test]
    async fn test_save_thread_refreshes_ordering(cx: &mut TestAppContext) {
        let thread_store = cx.new(|cx| ThreadStore::new(cx));
        cx.run_until_parked();

        let first_id = session_id("thread-a");
        let second_id = session_id("thread-b");

        let first_thread = make_thread(
            "Thread A",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        let second_thread = make_thread(
            "Thread B",
            Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap(),
        );

        let save_first = thread_store.update(cx, |store, cx| {
            store.save_thread(first_id.clone(), first_thread, PathList::default(), cx)
        });
        save_first.await.unwrap();
        let save_second = thread_store.update(cx, |store, cx| {
            store.save_thread(second_id.clone(), second_thread, PathList::default(), cx)
        });
        save_second.await.unwrap();
        cx.run_until_parked();

        let updated_first = make_thread(
            "Thread A",
            Utc.with_ymd_and_hms(2024, 1, 3, 0, 0, 0).unwrap(),
        );
        let update_task = thread_store.update(cx, |store, cx| {
            store.save_thread(first_id.clone(), updated_first, PathList::default(), cx)
        });
        update_task.await.unwrap();
        cx.run_until_parked();

        let entries: Vec<_> = thread_store.read_with(cx, |store, _cx| store.entries().collect());
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, first_id);
        assert_eq!(entries[1].id, second_id);
    }
}
