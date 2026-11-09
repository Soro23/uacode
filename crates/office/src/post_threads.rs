//! Which agent thread each post uses.
//!
//! Thread ids only exist on this machine, so the link lives in the local database, keyed by
//! the project folder and the post id, and never in the shareable `office.json`.

use agent_client_protocol::v2::schema as acp_v2;
use agent_ui::{ThreadId, thread_metadata_store::ThreadMetadataStore};
use anyhow::Result;
use collections::HashMap;
use db::kvp::KeyValueStore;
use gpui::{App, AppContext as _, Task};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
};
use util::ResultExt as _;

/// Shared by the panel, which opens the threads, and by the policy, which looks up the post
/// of a session.
#[derive(Clone, Default)]
pub struct PostThreads {
    state: Rc<RefCell<State>>,
}

#[derive(Default)]
struct State {
    project_root: Option<PathBuf>,
    threads: HashMap<String, ThreadId>,
}

fn key(project_root: &Path, post_id: &str) -> String {
    format!("office-post-thread:{}:{post_id}", project_root.display())
}

impl PostThreads {
    /// Reads the threads of `post_ids` for the project whose `office.json` is under
    /// `project_root`, replacing what was loaded before.
    pub fn load(
        &self,
        project_root: Option<&Path>,
        post_ids: impl IntoIterator<Item = String>,
        cx: &App,
    ) {
        let mut state = self.state.borrow_mut();
        state.threads.clear();
        state.project_root = project_root.map(Path::to_path_buf);
        let Some(project_root) = project_root else {
            return;
        };
        let database = KeyValueStore::global(cx);
        for post_id in post_ids {
            let Some(value) = database
                .read_kvp(&key(project_root, &post_id))
                .log_err()
                .flatten()
            else {
                continue;
            };
            if let Some(thread_id) = serde_json::from_str::<ThreadId>(&value).log_err() {
                state.threads.insert(post_id, thread_id);
            }
        }
    }

    pub fn thread(&self, post_id: &str) -> Option<ThreadId> {
        self.state.borrow().threads.get(post_id).copied()
    }

    /// Makes `thread_id` the post's thread. The previous one stays in the agent panel's
    /// history.
    pub fn set(&self, post_id: &str, thread_id: ThreadId, cx: &App) -> Task<Result<()>> {
        let mut state = self.state.borrow_mut();
        state.threads.insert(post_id.to_string(), thread_id);
        let Some(project_root) = state.project_root.as_deref() else {
            return Task::ready(Ok(()));
        };
        let key = key(project_root, post_id);
        let value = match serde_json::to_string(&thread_id) {
            Ok(value) => value,
            Err(error) => return Task::ready(Err(error.into())),
        };
        let database = KeyValueStore::global(cx);
        cx.background_spawn(async move { database.write_kvp(key, value).await })
    }

    pub fn post_for_thread(&self, thread_id: ThreadId) -> Option<String> {
        self.state
            .borrow()
            .threads
            .iter()
            .find(|(_, thread)| **thread == thread_id)
            .map(|(post_id, _)| post_id.clone())
    }

    pub fn post_for_session(&self, session_id: &acp_v2::SessionId, cx: &App) -> Option<String> {
        let thread_id = ThreadMetadataStore::try_global(cx)?
            .read(cx)
            .entry_by_session(session_id)?
            .thread_id;
        self.post_for_thread(thread_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_scoped_by_project_and_post() {
        let first = key(Path::new("/work/app"), "post-1");
        assert_ne!(first, key(Path::new("/work/other"), "post-1"));
        assert_ne!(first, key(Path::new("/work/app"), "post-2"));
    }

    #[test]
    fn each_post_has_its_own_thread() {
        let threads = PostThreads::default();
        let first = ThreadId::new();
        let second = ThreadId::new();
        {
            let mut state = threads.state.borrow_mut();
            state.threads.insert("backend".into(), first);
            state.threads.insert("qa".into(), second);
        }
        assert_eq!(threads.thread("backend"), Some(first));
        assert_eq!(threads.post_for_thread(second).as_deref(), Some("qa"));
        assert_eq!(threads.post_for_thread(ThreadId::new()), None);
    }

    #[test]
    fn thread_ids_round_trip_through_the_database_value() {
        let thread_id = ThreadId::new();
        let value = serde_json::to_string(&thread_id).expect("serialize");
        assert_eq!(
            serde_json::from_str::<ThreadId>(&value).expect("deserialize"),
            thread_id
        );
    }
}
