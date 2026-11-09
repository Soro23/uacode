//! The thread policy checks shared by every ACP connection version.
//!
//! v1 and v2 sessions call these functions for session start, default modes, the user's mode
//! changes and the agent's own mode changes; the connections only differ in how they send the
//! resulting requests. Keeping the decisions here means a new connection path cannot forget
//! one of them.

use acp_thread::{
    AcpThread,
    policy::{ModeChangeOrigin, PolicyContext, PolicyRequest, ThreadPolicies},
};
use agent_client_protocol::v2::schema as acp_v2;
use anyhow::{Result, anyhow};
use gpui::{App, AppContext as _, AsyncApp, Entity, SharedString, WeakEntity};
use project::AgentId;
use util::ResultExt as _;

/// Asks the agent's policy whether a session may start, and for its launch options.
pub(crate) fn session_meta(
    agent_id: &AgentId,
    agent_version: Option<&str>,
    cx: &App,
) -> Result<Option<acp_v2::Meta>> {
    ThreadPolicies::start_session(agent_id, agent_version, cx).map_err(|reason| anyhow!("{reason}"))
}

pub(crate) struct PolicyTarget<'a> {
    pub agent_id: &'a AgentId,
    pub agent_version: Option<&'a str>,
    pub session_id: &'a acp_v2::SessionId,
}

impl PolicyTarget<'_> {
    /// Why the policy refuses switching the session to `mode`, if it does.
    pub fn mode_denial(
        &self,
        mode: &str,
        origin: ModeChangeOrigin,
        cx: &App,
    ) -> Option<SharedString> {
        let decision = ThreadPolicies::decide(
            &PolicyContext {
                agent_id: self.agent_id,
                agent_version: self.agent_version,
                session_id: self.session_id,
                parent_session_id: None,
            },
            &PolicyRequest::SetMode { mode, origin },
            cx,
        )?;
        (!decision.allows_mode_change(origin)).then(|| decision.denial_reason())
    }

    /// Whether a saved default mode may be applied when the session starts.
    pub fn default_mode_allowed(&self, mode: &str, cx: &App) -> bool {
        match self.mode_denial(mode, ModeChangeOrigin::Default, cx) {
            None => true,
            Some(reason) => {
                log::info!("Default mode `{mode}` refused by the agent's policy: {reason}");
                false
            }
        }
    }

    /// Why the policy refuses setting `config_id` to `value`, when that option is the mode.
    pub fn config_mode_denial(
        &self,
        options: &[acp_v2::SessionConfigOption],
        config_id: &acp_v2::SessionConfigId,
        value: &acp_v2::SessionConfigOptionValue,
        origin: ModeChangeOrigin,
        cx: &App,
    ) -> Option<SharedString> {
        let option = options
            .iter()
            .find(|option| &option.config_id == config_id)?;
        let mode = mode_option_value(option, value)?;
        self.mode_denial(mode, origin, cx)
    }

    /// Whether a saved default config option may be applied when the session starts.
    pub fn default_config_allowed(
        &self,
        option: &acp_v2::SessionConfigOption,
        value: &acp_v2::SessionConfigOptionValue,
        cx: &App,
    ) -> bool {
        match mode_option_value(option, value) {
            Some(mode) => self.default_mode_allowed(mode, cx),
            None => true,
        }
    }
}

/// The mode a value selects when `option` is the session's mode selector.
pub(crate) fn mode_option_value<'a>(
    option: &acp_v2::SessionConfigOption,
    value: &'a acp_v2::SessionConfigOptionValue,
) -> Option<&'a str> {
    if !is_mode_option(option) {
        return None;
    }
    match value {
        acp_v2::SessionConfigOptionValue::Id { value } => Some(value.0.as_ref()),
        _ => None,
    }
}

fn is_mode_option(option: &acp_v2::SessionConfigOption) -> bool {
    matches!(
        option.category,
        Some(acp_v2::SessionConfigOptionCategory::Mode)
    )
}

fn current_mode(
    options: &[acp_v2::SessionConfigOption],
) -> Option<(acp_v2::SessionConfigId, String)> {
    options.iter().find_map(|option| {
        if !is_mode_option(option) {
            return None;
        }
        match &option.kind {
            acp_v2::SessionConfigKind::Select(select) => {
                Some((option.config_id.clone(), select.current_value.0.to_string()))
            }
            _ => None,
        }
    })
}

/// Whether a mode the agent switched to on its own may stand. A denied switch is counted by
/// the thread, which also cancels the turn from the second one; the caller reverts it.
pub(crate) fn agent_mode_change_stands(
    thread: &Entity<AcpThread>,
    previous_mode: &str,
    updated_mode: &str,
    cx: &mut AsyncApp,
) -> bool {
    previous_mode == updated_mode
        || thread.update(cx, |thread, cx| {
            thread.agent_mode_change_stands(updated_mode, cx)
        })
}

/// Checks an agent's change to the mode config option and reverts a denied one through the
/// session's config options.
pub(crate) fn check_agent_config_mode_change(
    thread: &WeakEntity<AcpThread>,
    session_id: &acp_v2::SessionId,
    previous: &[acp_v2::SessionConfigOption],
    updated: &[acp_v2::SessionConfigOption],
    cx: &mut AsyncApp,
) {
    let (Some((config_id, previous_mode)), Some((_, updated_mode))) =
        (current_mode(previous), current_mode(updated))
    else {
        return;
    };
    let Some(thread) = thread.upgrade() else {
        return;
    };
    if agent_mode_change_stands(&thread, &previous_mode, &updated_mode, cx) {
        return;
    }
    let revert = cx.update(|cx| {
        let options = thread
            .read(cx)
            .connection()
            .session_config_options(session_id, cx)?;
        Some(options.set_config_option(
            config_id,
            acp_v2::SessionConfigOptionValue::id(previous_mode),
            cx,
        ))
    });
    if let Some(revert) = revert {
        cx.background_spawn(async move {
            revert.await.log_err();
        })
        .detach();
    }
}

/// A policy with Office's Phase 1 mode rules, shared by the v1 and v2 connection tests so
/// both paths are checked against the same decisions.
#[cfg(test)]
pub(crate) struct ConsultOnlyModes;

#[cfg(test)]
impl acp_thread::policy::ThreadPolicy for ConsultOnlyModes {
    fn decide(
        &self,
        _: &PolicyContext,
        request: &PolicyRequest,
        _: &App,
    ) -> acp_thread::policy::PolicyDecision {
        match request {
            PolicyRequest::SetMode { mode, .. }
                if !["default", "plan", "read-only"].contains(mode) =>
            {
                acp_thread::policy::PolicyDecision::Deny {
                    reason: "solo consulta".into(),
                }
            }
            _ => acp_thread::policy::PolicyDecision::Allow,
        }
    }
}
