//! Policy hook for ACP threads.
//!
//! Lets an embedder (UACode Office) decide permission requests, client file writes, client
//! terminals, session mode changes and session start for the threads of one agent. Agents
//! without a registered policy keep the default behavior: every hook returns `None` for them.

use crate::{AcpThread, PermissionRequestId, ToolCall};
use agent_client_protocol::v2::schema as acp_v2;
use collections::HashMap;
use gpui::{App, Global, SharedString, WeakEntity};
use project::AgentId;
use std::{path::Path, rc::Rc};

pub struct PolicyContext<'a> {
    pub agent_id: &'a AgentId,
    /// The version the agent reported in `initialize`, if any.
    pub agent_version: Option<&'a str>,
    pub session_id: &'a acp_v2::SessionId,
    pub parent_session_id: Option<&'a acp_v2::SessionId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeChangeOrigin {
    /// The default mode or config option Zed applies when a session starts.
    Default,
    /// A change the user made from the thread's mode or config selector.
    User,
    /// A change the agent reported on its own.
    Agent,
}

pub enum PolicyRequest<'a> {
    Permission {
        tool_call: Option<&'a ToolCall>,
    },
    WriteTextFile {
        path: &'a Path,
    },
    CreateTerminal {
        command: &'a str,
        args: &'a [String],
        cwd: Option<&'a Path>,
    },
    SetMode {
        mode: &'a str,
        origin: ModeChangeOrigin,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    Deny { reason: SharedString },
    AskUser,
}

impl PolicyDecision {
    /// A mode change offers nobody to ask: the user already acted when they picked the mode,
    /// and changes from the agent or the defaults have no prompt.
    pub fn allows_mode_change(&self, origin: ModeChangeOrigin) -> bool {
        match self {
            Self::Allow => true,
            Self::Deny { .. } => false,
            Self::AskUser => origin == ModeChangeOrigin::User,
        }
    }

    pub fn denial_reason(&self) -> SharedString {
        match self {
            Self::Deny { reason } => reason.clone(),
            Self::Allow | Self::AskUser => "La política del agente no lo permite".into(),
        }
    }
}

pub trait ThreadPolicy: 'static {
    fn decide(&self, context: &PolicyContext, request: &PolicyRequest, cx: &App) -> PolicyDecision;

    /// Runs before `session/new`, `session/load` and `session/resume`. An error refuses the
    /// session with that reason. `Ok(Some(meta))` is sent as the request's `_meta`: agents
    /// read launch options from it (for example `claudeCode.options`), and they must be
    /// repeated whenever the agent restores a session.
    fn start_session(
        &self,
        _agent_id: &AgentId,
        _agent_version: Option<&str>,
        _cx: &App,
    ) -> Result<Option<acp_v2::Meta>, SharedString> {
        Ok(None)
    }

    /// Whether the agent's preferences saved in settings.json (default mode, default config
    /// options and favorite values) are read and written. When `false`, choices made in a
    /// thread only affect that session.
    fn uses_saved_preferences(&self) -> bool {
        true
    }

    /// A request this policy left to the user is waiting in `thread`. Lets the embedder show
    /// it elsewhere and answer it with [`AcpThread::answer_permission_request`].
    fn permission_pending(
        &self,
        _context: &PolicyContext,
        _thread: WeakEntity<AcpThread>,
        _request: PermissionRequestId,
        _cx: &App,
    ) {
    }
}

#[derive(Default)]
pub struct ThreadPolicies {
    policies: HashMap<AgentId, Rc<dyn ThreadPolicy>>,
}

impl Global for ThreadPolicies {}

impl ThreadPolicies {
    pub fn register(agent_id: AgentId, policy: Rc<dyn ThreadPolicy>, cx: &mut App) {
        cx.default_global::<Self>()
            .policies
            .insert(agent_id, policy);
    }

    pub fn unregister(agent_id: &AgentId, cx: &mut App) {
        if let Some(policies) = cx.try_global::<Self>()
            && policies.policies.contains_key(agent_id)
        {
            cx.global_mut::<Self>().policies.remove(agent_id);
        }
    }

    pub fn for_agent(agent_id: &AgentId, cx: &App) -> Option<Rc<dyn ThreadPolicy>> {
        cx.try_global::<Self>()?.policies.get(agent_id).cloned()
    }

    pub fn decide(
        context: &PolicyContext,
        request: &PolicyRequest,
        cx: &App,
    ) -> Option<PolicyDecision> {
        let policy = Self::for_agent(context.agent_id, cx)?;
        Some(policy.decide(context, request, cx))
    }

    pub fn start_session(
        agent_id: &AgentId,
        agent_version: Option<&str>,
        cx: &App,
    ) -> Result<Option<acp_v2::Meta>, SharedString> {
        match Self::for_agent(agent_id, cx) {
            Some(policy) => policy.start_session(agent_id, agent_version, cx),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AcpThread, AcpThreadEvent, AgentConnection as _, AuthorizationKind, PermissionOptions,
        RequestPermissionOutcome, StubAgentConnection,
    };
    use agent_client_protocol::v1::schema as acp_v1;
    use fs::Fs as _;
    use futures::FutureExt as _;
    use gpui::{Entity, Task, TestAppContext};
    use project::{FakeFs, Project};
    use serde_json::json;
    use settings::SettingsStore;
    use std::{cell::RefCell, path::PathBuf, sync::Arc};
    use util::{path, path_list::PathList};

    const OFFICE_AGENT: &str = "office-claude-default";

    struct FixedPolicy(PolicyDecision);

    impl ThreadPolicy for FixedPolicy {
        fn decide(&self, _: &PolicyContext, _: &PolicyRequest, _: &App) -> PolicyDecision {
            self.0.clone()
        }
    }

    fn deny() -> PolicyDecision {
        PolicyDecision::Deny {
            reason: "solo consulta".into(),
        }
    }

    async fn new_thread(
        agent_id: &str,
        cx: &mut TestAppContext,
    ) -> (Entity<AcpThread>, Rc<StubAgentConnection>, Arc<FakeFs>) {
        env_logger::try_init().ok();
        cx.update(|cx| {
            let mut settings_store = SettingsStore::test(cx);
            settings_store.register_setting::<feature_flags::FeatureFlagsSettings>();
            cx.set_global(settings_store);
        });
        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(path!("/project"), json!({"file.txt": "original"}))
            .await;
        let project = Project::test(fs.clone(), [Path::new(path!("/project"))], cx).await;
        let connection = Rc::new(StubAgentConnection::new().with_agent_id(AgentId::new(agent_id)));
        let thread = cx
            .update(|cx| {
                connection.clone().new_session(
                    project,
                    PathList::new(&[Path::new(path!("/project"))]),
                    cx,
                )
            })
            .await
            .expect("session");
        (thread, connection, fs)
    }

    fn register(decision: PolicyDecision, cx: &mut TestAppContext) {
        cx.update(|cx| {
            ThreadPolicies::register(
                AgentId::new(OFFICE_AGENT),
                Rc::new(FixedPolicy(decision)),
                cx,
            )
        });
    }

    fn request_edit_permission(
        thread: &Entity<AcpThread>,
        cx: &mut TestAppContext,
    ) -> Task<RequestPermissionOutcome> {
        thread.update(cx, |thread, cx| {
            thread
                .request_tool_call_authorization(
                    acp_v1::ToolCall::new(acp_v1::ToolCallId::new("edit"), "Edit")
                        .kind(acp_v1::ToolKind::Edit)
                        .into(),
                    PermissionOptions::Flat(vec![
                        acp_v2::PermissionOption::new(
                            "always",
                            "Always allow",
                            acp_v2::PermissionOptionKind::AllowAlways,
                        ),
                        acp_v2::PermissionOption::new(
                            "once",
                            "Allow",
                            acp_v2::PermissionOptionKind::AllowOnce,
                        ),
                        acp_v2::PermissionOption::new(
                            "reject",
                            "Reject",
                            acp_v2::PermissionOptionKind::RejectOnce,
                        ),
                    ]),
                    AuthorizationKind::PermissionGrant,
                    cx,
                )
                .expect("permission request")
        })
    }

    fn record_authorization_requests(
        thread: &Entity<AcpThread>,
        cx: &mut TestAppContext,
    ) -> (Rc<RefCell<usize>>, gpui::Subscription) {
        let requested = Rc::new(RefCell::new(0));
        let subscription = cx.update(|cx| {
            cx.subscribe(thread, {
                let requested = requested.clone();
                move |_, event, _| {
                    if matches!(event, AcpThreadEvent::ToolAuthorizationRequested(_)) {
                        *requested.borrow_mut() += 1;
                    }
                }
            })
        });
        (requested, subscription)
    }

    fn selected_option(outcome: RequestPermissionOutcome) -> String {
        match outcome {
            RequestPermissionOutcome::Selected(outcome) => outcome.option_id.0.to_string(),
            RequestPermissionOutcome::Cancelled
            | RequestPermissionOutcome::InterruptedByFollowUp => "cancelled".into(),
        }
    }

    fn write(thread: &Entity<AcpThread>, cx: &mut TestAppContext) -> Task<anyhow::Result<()>> {
        thread.update(cx, |thread, cx| {
            thread.write_text_file(
                PathBuf::from(path!("/project/file.txt")),
                "changed".into(),
                cx,
            )
        })
    }

    async fn file_text(fs: &FakeFs) -> String {
        // Saving may normalize line endings and add a final newline.
        fs.load(Path::new(path!("/project/file.txt")))
            .await
            .expect("file")
            .trim_end()
            .to_string()
    }

    #[gpui::test]
    async fn without_policy_permission_waits_for_the_user(cx: &mut TestAppContext) {
        let (thread, _, fs) = new_thread("claude-acp", cx).await;
        // A policy for another agent does not affect this thread.
        register(deny(), cx);
        let (requested, _subscription) = record_authorization_requests(&thread, cx);

        let mut permission = request_edit_permission(&thread, cx);
        cx.run_until_parked();
        assert_eq!(*requested.borrow(), 1);
        assert!((&mut permission).now_or_never().is_none());
        thread.read_with(cx, |thread, _| {
            assert_eq!(thread.pending_permission_requests().count(), 1);
            assert!(thread.notices().is_empty());
        });

        write(&thread, cx).await.expect("write without policy");
        assert_eq!(file_text(&fs).await, "changed");
        thread
            .update(cx, |thread, cx| {
                thread.authorize_terminal("cargo", &["build".into()], None, cx)
            })
            .await
            .expect("terminal without policy");
    }

    #[gpui::test]
    async fn denying_policy_rejects_without_asking(cx: &mut TestAppContext) {
        let (thread, _, fs) = new_thread(OFFICE_AGENT, cx).await;
        register(deny(), cx);
        let (requested, _subscription) = record_authorization_requests(&thread, cx);

        let permission = request_edit_permission(&thread, cx);
        assert_eq!(selected_option(permission.await), "reject");
        assert_eq!(*requested.borrow(), 0);

        let error = write(&thread, cx).await.expect_err("write is denied");
        assert!(error.to_string().contains("solo consulta"), "{error:#}");
        assert_eq!(file_text(&fs).await, "original");
        thread
            .update(cx, |thread, cx| {
                thread.authorize_terminal("cargo", &["build".into()], None, cx)
            })
            .await
            .expect_err("terminal is denied");

        thread.read_with(cx, |thread, _| {
            assert_eq!(thread.pending_permission_requests().count(), 0);
            assert_eq!(thread.notices().len(), 3);
            assert!(
                thread
                    .notices()
                    .iter()
                    .all(|(_, notice)| { notice.description.as_deref() == Some("solo consulta") })
            );
        });
    }

    #[gpui::test]
    async fn denial_without_reject_option_cancels(cx: &mut TestAppContext) {
        let (thread, _, _) = new_thread(OFFICE_AGENT, cx).await;
        register(deny(), cx);
        let permission = thread.update(cx, |thread, cx| {
            thread
                .request_tool_call_authorization(
                    acp_v1::ToolCall::new(acp_v1::ToolCallId::new("bash"), "Bash").into(),
                    PermissionOptions::Flat(vec![acp_v2::PermissionOption::new(
                        "once",
                        "Allow",
                        acp_v2::PermissionOptionKind::AllowOnce,
                    )]),
                    AuthorizationKind::PermissionGrant,
                    cx,
                )
                .expect("permission request")
        });
        assert_eq!(selected_option(permission.await), "cancelled");
    }

    #[gpui::test]
    async fn allowing_policy_never_picks_an_always_option(cx: &mut TestAppContext) {
        let (thread, _, _) = new_thread(OFFICE_AGENT, cx).await;
        register(PolicyDecision::Allow, cx);
        let permission = request_edit_permission(&thread, cx);
        assert_eq!(selected_option(permission.await), "once");
    }

    #[gpui::test]
    async fn asking_policy_leaves_the_decision_to_the_user(cx: &mut TestAppContext) {
        let (thread, _, fs) = new_thread(OFFICE_AGENT, cx).await;
        register(PolicyDecision::AskUser, cx);
        let (requested, _subscription) = record_authorization_requests(&thread, cx);

        let mut permission = request_edit_permission(&thread, cx);
        cx.run_until_parked();
        assert_eq!(*requested.borrow(), 1);
        assert!((&mut permission).now_or_never().is_none());

        let write_task = write(&thread, cx);
        cx.run_until_parked();
        assert_eq!(*requested.borrow(), 2);
        assert_eq!(file_text(&fs).await, "original");
        let write_request = thread.read_with(cx, |thread, _| {
            thread
                .pending_permission_requests()
                .find(|request| request.generic_request().is_some())
                .expect("write asks the user")
                .id
        });
        thread.update(cx, |thread, cx| {
            thread.select_permission_option(write_request, crate::POLICY_ALLOW_OPTION_ID.into(), cx)
        });
        write_task.await.expect("the user allowed the write");
        assert_eq!(file_text(&fs).await, "changed");

        let terminal = thread.update(cx, |thread, cx| {
            thread.authorize_terminal("cargo", &["build".into()], None, cx)
        });
        cx.run_until_parked();
        let terminal_request = thread.read_with(cx, |thread, _| {
            thread
                .pending_permission_requests()
                .find(|request| request.generic_request().is_some())
                .expect("terminal asks the user")
                .id
        });
        thread.update(cx, |thread, cx| {
            thread.select_permission_option(
                terminal_request,
                crate::POLICY_REJECT_OPTION_ID.into(),
                cx,
            )
        });
        terminal.await.expect_err("the user rejected the terminal");
    }

    #[gpui::test]
    async fn repeated_denied_agent_mode_change_cancels_the_turn(cx: &mut TestAppContext) {
        let (thread, connection, _) = new_thread(OFFICE_AGENT, cx).await;
        let _response = connection.defer_next_prompt_response();
        let _turn = thread.update(cx, |thread, cx| thread.send(vec!["hola".into()], cx));
        cx.run_until_parked();

        register(PolicyDecision::Allow, cx);
        assert!(thread.update(cx, |thread, cx| thread.agent_mode_change_stands("plan", cx)));

        register(PolicyDecision::AskUser, cx);
        assert!(!thread.update(cx, |thread, cx| {
            thread.agent_mode_change_stands("bypassPermissions", cx)
        }));
        cx.run_until_parked();
        assert!(connection.take_cancel_calls().is_empty());

        assert!(!thread.update(cx, |thread, cx| {
            thread.agent_mode_change_stands("bypassPermissions", cx)
        }));
        cx.run_until_parked();
        assert_eq!(connection.take_cancel_calls().len(), 1);
        thread.read_with(cx, |thread, _| {
            let titles = thread
                .notices()
                .iter()
                .map(|(_, notice)| notice.title.clone())
                .collect::<Vec<_>>();
            assert_eq!(titles.len(), 2);
            assert!(titles[1].starts_with("Turno detenido"), "{titles:?}");
        });
    }

    #[derive(Default)]
    struct RecordingPolicy {
        pending: RefCell<Vec<(Option<String>, PermissionRequestId)>>,
    }

    impl ThreadPolicy for RecordingPolicy {
        fn decide(&self, _: &PolicyContext, _: &PolicyRequest, _: &App) -> PolicyDecision {
            PolicyDecision::AskUser
        }

        fn permission_pending(
            &self,
            context: &PolicyContext,
            _thread: WeakEntity<AcpThread>,
            request: PermissionRequestId,
            _: &App,
        ) {
            self.pending
                .borrow_mut()
                .push((context.agent_version.map(ToString::to_string), request));
        }
    }

    #[gpui::test]
    async fn pending_requests_can_be_answered_outside_the_thread(cx: &mut TestAppContext) {
        let (thread, _, _) = new_thread(OFFICE_AGENT, cx).await;
        let policy = Rc::new(RecordingPolicy::default());
        cx.update(|cx| ThreadPolicies::register(AgentId::new(OFFICE_AGENT), policy.clone(), cx));

        let approved = request_edit_permission(&thread, cx);
        let (_, request) = policy.pending.borrow()[0];
        let title = thread.read_with(cx, |thread, cx| {
            thread.permission_request_title(request, cx)
        });
        assert_eq!(title.as_deref(), Some("Edit"));
        thread.update(cx, |thread, cx| {
            thread.answer_permission_request(request, true, cx)
        });
        assert_eq!(selected_option(approved.await), "once");

        let rejected = request_edit_permission(&thread, cx);
        let (_, request) = policy.pending.borrow()[1];
        thread.update(cx, |thread, cx| {
            thread.answer_permission_request(request, false, cx)
        });
        assert_eq!(selected_option(rejected.await), "reject");
        assert_eq!(policy.pending.borrow().len(), 2);
    }

    #[test]
    fn mode_change_ask_user_only_allows_user_changes() {
        let decision = PolicyDecision::AskUser;
        assert!(decision.allows_mode_change(ModeChangeOrigin::User));
        assert!(!decision.allows_mode_change(ModeChangeOrigin::Agent));
        assert!(!decision.allows_mode_change(ModeChangeOrigin::Default));
        assert!(!deny().allows_mode_change(ModeChangeOrigin::User));
    }
}
