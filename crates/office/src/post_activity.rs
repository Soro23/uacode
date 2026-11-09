//! What a post's thread is doing, summarized for its card.

use acp_thread::{
    AcpThread, AgentThreadEntry, AssistantMessageChunk, PlanContent, ThreadStatus, ToolCallStatus,
};
use agent_client_protocol::v2::schema as acp_v2;
use gpui::App;
use ui::Color;

const MAX_SUMMARY_CHARS: usize = 140;

/// A snapshot of the thread, refreshed at most a few times per second.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThreadActivity {
    pub working: bool,
    pub waiting_approval: bool,
    pub error: Option<String>,
    pub task: Option<String>,
    pub last_message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PostState {
    RequiresLogin,
    Error(String),
    WaitingApproval,
    Working,
    Free,
}

impl PostState {
    pub fn label(&self) -> String {
        match self {
            Self::RequiresLogin => "Requiere login".into(),
            Self::Error(message) => format!("Error: {message}"),
            Self::WaitingApproval => "Esperando aprobación".into(),
            Self::Working => "Trabajando".into(),
            Self::Free => "Libre".into(),
        }
    }

    pub fn color(&self) -> Color {
        match self {
            Self::RequiresLogin | Self::WaitingApproval => Color::Warning,
            Self::Error(_) => Color::Error,
            Self::Working => Color::Accent,
            Self::Free => Color::Muted,
        }
    }
}

/// A post's state from its account and its thread. Login comes first: an account that is
/// logged out makes the thread fail too, and logging in is what fixes it.
pub fn post_state(
    requires_login: bool,
    account_error: Option<&str>,
    waiting_approval: bool,
    activity: Option<&ThreadActivity>,
) -> PostState {
    if requires_login {
        return PostState::RequiresLogin;
    }
    if let Some(error) = account_error.or_else(|| activity.and_then(|a| a.error.as_deref())) {
        return PostState::Error(error.to_string());
    }
    if waiting_approval {
        return PostState::WaitingApproval;
    }
    if activity.is_some_and(|activity| activity.working) {
        return PostState::Working;
    }
    PostState::Free
}

pub fn thread_activity(thread: &AcpThread, cx: &App) -> ThreadActivity {
    let error = thread
        .local_execution_error()
        .map(|error| summarize(&format!("{error:#}")))
        .or_else(|| {
            thread
                .had_error()
                .then(|| "el agente devolvió un error".to_string())
        });
    ThreadActivity {
        working: thread.status() == ThreadStatus::Generating,
        waiting_approval: thread.pending_permission_requests().next().is_some(),
        error,
        task: current_task(thread, cx),
        last_message: last_agent_message(thread.entries()),
    }
}

/// The plan step in progress, or else the tool call that is running.
fn current_task(thread: &AcpThread, cx: &App) -> Option<String> {
    if let Some(plan) = thread.plan()
        && let PlanContent::Items(entries) = &plan.content
        && let Some(entry) = entries
            .iter()
            .find(|entry| entry.source.status == acp_v2::PlanEntryStatus::InProgress)
    {
        return Some(summarize(&entry.source.content));
    }
    thread.entries().iter().rev().find_map(|entry| match entry {
        AgentThreadEntry::ToolCall(call)
            if matches!(
                call.status(),
                ToolCallStatus::Pending
                    | ToolCallStatus::WaitingForConfirmation
                    | ToolCallStatus::InProgress
            ) =>
        {
            Some(summarize(call.label.read(cx).source()))
        }
        _ => None,
    })
}

/// The last line of text the agent wrote, without its thoughts.
pub fn last_agent_message(entries: &[AgentThreadEntry]) -> Option<String> {
    entries.iter().rev().find_map(|entry| {
        let AgentThreadEntry::AssistantMessage(message) = entry else {
            return None;
        };
        let text = message
            .chunks
            .iter()
            .filter_map(|chunk| match chunk {
                AssistantMessageChunk::Message { block, .. } => Some(block),
                _ => None,
            })
            .flat_map(|block| block.source_blocks())
            .filter_map(|block| match block {
                acp_v2::ContentBlock::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<String>();
        text.lines()
            .rev()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(summarize)
    })
}

fn summarize(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default().trim();
    if line.chars().count() <= MAX_SUMMARY_CHARS {
        return line.to_string();
    }
    let mut summary = line.chars().take(MAX_SUMMARY_CHARS).collect::<String>();
    summary.push('…');
    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    fn activity(working: bool, error: Option<&str>) -> ThreadActivity {
        ThreadActivity {
            working,
            error: error.map(Into::into),
            ..Default::default()
        }
    }

    #[test]
    fn post_states_in_order_of_precedence() {
        let busy_failing = activity(true, Some("sin conexión"));
        assert_eq!(
            post_state(true, Some("x"), true, Some(&busy_failing)),
            PostState::RequiresLogin
        );
        assert_eq!(
            post_state(false, None, true, Some(&busy_failing)),
            PostState::Error("sin conexión".into())
        );
        assert_eq!(
            post_state(false, Some("cuenta"), false, None),
            PostState::Error("cuenta".into())
        );
        assert_eq!(
            post_state(false, None, true, Some(&activity(true, None))),
            PostState::WaitingApproval
        );
        assert_eq!(
            post_state(false, None, false, Some(&activity(true, None))),
            PostState::Working
        );
        assert_eq!(
            post_state(false, None, false, Some(&activity(false, None))),
            PostState::Free
        );
        assert_eq!(post_state(false, None, false, None), PostState::Free);
    }

    #[test]
    fn summaries_keep_one_short_line() {
        assert_eq!(summarize("  hola\nsegunda"), "hola");
        let long = "a".repeat(MAX_SUMMARY_CHARS + 10);
        let summary = summarize(&long);
        assert_eq!(summary.chars().count(), MAX_SUMMARY_CHARS + 1);
        assert!(summary.ends_with('…'));
    }
}
