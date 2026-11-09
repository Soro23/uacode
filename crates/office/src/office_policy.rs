//! Office's policy for the threads of its agents (`office-*`).
//!
//! Phase 1 is read-only for every post and role. The rules are looked up per thread so that
//! later phases can decide by post and role once Office links sessions to posts.

use crate::office_store::Provider;
use acp_thread::{
    AcpThread, PermissionRequestId, ToolCall,
    policy::{ModeChangeOrigin, PolicyContext, PolicyDecision, PolicyRequest, ThreadPolicy},
};
use agent_client_protocol::v2::schema as acp_v2;
use futures::channel::mpsc::UnboundedSender;
use gpui::{App, SharedString, WeakEntity};
use project::AgentId;

pub struct Adapter {
    pub package: &'static str,
    pub version: &'static str,
}

impl Adapter {
    pub fn spec(&self) -> String {
        format!("{}@{}", self.package, self.version)
    }
}

/// The adapter versions whose tools and permission flows this policy was checked against.
/// Changing one requires checking `CLAUDE_WRITE_AND_EXECUTE_TOOLS` and the permission kinds
/// again: a session on any other version is refused.
pub const CLAUDE_ADAPTER: Adapter = Adapter {
    package: "@agentclientprotocol/claude-agent-acp",
    version: "0.94.0",
};
pub const CODEX_ADAPTER: Adapter = Adapter {
    package: "@agentclientprotocol/codex-acp",
    version: "2.5.0",
};

pub fn adapter(provider: Provider) -> &'static Adapter {
    match provider {
        Provider::Claude => &CLAUDE_ADAPTER,
        Provider::Codex => &CODEX_ADAPTER,
    }
}

/// Claude Code tools that write files or run commands in claude-agent-acp 0.94.0
/// (`file-change-audit.js`, `permissions/options.js`, `tool-calls/reporters/index.js`).
const CLAUDE_WRITE_AND_EXECUTE_TOOLS: [&str; 7] = [
    "Edit",
    "Write",
    "MultiEdit",
    "NotebookEdit",
    "Bash",
    "PowerShell",
    "Monitor",
];

const PHASE_ONE: &str = "Office, Fase 1: solo consulta";

pub enum PolicyEvent {
    Denied {
        agent_id: AgentId,
        reason: SharedString,
    },
    Pending {
        agent_id: AgentId,
        thread: WeakEntity<AcpThread>,
        request: PermissionRequestId,
    },
}

/// What a post may do. In Phase 1 every post and role gets `consult_only`.
struct PostRules {
    allowed_modes: &'static [&'static str],
}

impl PostRules {
    fn consult_only(provider: Provider) -> Self {
        Self {
            allowed_modes: match provider {
                Provider::Claude => &["default", "plan"],
                Provider::Codex => &["read-only"],
            },
        }
    }
}

pub struct OfficePolicy {
    provider: Provider,
    events: UnboundedSender<PolicyEvent>,
}

impl OfficePolicy {
    pub fn new(provider: Provider, events: UnboundedSender<PolicyEvent>) -> Self {
        Self { provider, events }
    }

    // Sessions are not linked to posts yet, so every thread gets the Phase 1 rules.
    fn rules_for(&self, _context: &PolicyContext) -> PostRules {
        PostRules::consult_only(self.provider)
    }

    fn version_problem(&self, agent_version: Option<&str>) -> Option<SharedString> {
        let expected = adapter(self.provider);
        if agent_version == Some(expected.version) {
            return None;
        }
        Some(
            format!(
                "Versión de adaptador no verificada: {} {}; Office solo admite {}",
                self.provider.label(),
                agent_version.unwrap_or("desconocida"),
                expected.version
            )
            .into(),
        )
    }

    fn decision(&self, context: &PolicyContext, request: &PolicyRequest) -> PolicyDecision {
        if let Some(reason) = self.version_problem(context.agent_version) {
            return PolicyDecision::Deny { reason };
        }
        let rules = self.rules_for(context);
        match request {
            PolicyRequest::SetMode { mode, .. } if rules.allowed_modes.contains(mode) => {
                PolicyDecision::Allow
            }
            PolicyRequest::SetMode { mode, origin } => deny(&format!(
                "Modo «{mode}» no permitido{}",
                match origin {
                    ModeChangeOrigin::User => " al cambiarlo desde el hilo",
                    ModeChangeOrigin::Agent => " (lo pidió el agente)",
                    ModeChangeOrigin::Default => " como modo por defecto",
                }
            )),
            PolicyRequest::WriteTextFile { .. } => deny("Escritura de archivos"),
            PolicyRequest::CreateTerminal { .. } => deny("Ejecución de comandos"),
            PolicyRequest::Permission { tool_call } => classify_permission(
                self.provider,
                tool_call.and_then(|call: &ToolCall| call.reported_kind.as_ref()),
            ),
        }
    }

    fn session_meta(&self) -> Option<acp_v2::Meta> {
        match self.provider {
            Provider::Claude => Some(acp_v2::Meta::from_iter([(
                "claudeCode".into(),
                serde_json::json!({
                    "options": {
                        "disallowedTools": CLAUDE_WRITE_AND_EXECUTE_TOOLS,
                        "allowDangerouslySkipPermissions": false,
                        // Office controls the whole configuration: no allow rules, hooks
                        // or default mode from the account folder or the project.
                        "settingSources": [],
                    }
                }),
            )])),
            Provider::Codex => None,
        }
    }

    fn send(&self, event: PolicyEvent) {
        if self.events.unbounded_send(event).is_err() {
            log::debug!("Office panel is gone; policy event dropped");
        }
    }
}

fn deny(what: &str) -> PolicyDecision {
    PolicyDecision::Deny {
        reason: format!("{what}: {PHASE_ONE}").into(),
    }
}

fn classify_permission(provider: Provider, kind: Option<&acp_v2::ToolKind>) -> PolicyDecision {
    // Codex only asks when an action would leave its read-only sandbox.
    if provider == Provider::Codex {
        return deny("Escalada fuera del sandbox read-only");
    }
    match kind {
        Some(acp_v2::ToolKind::Edit | acp_v2::ToolKind::Delete | acp_v2::ToolKind::Move) => {
            deny("Escritura de archivos")
        }
        Some(acp_v2::ToolKind::Execute) => deny("Ejecución de comandos"),
        Some(acp_v2::ToolKind::Fetch) => deny("Acceso a la red"),
        Some(acp_v2::ToolKind::SwitchMode) => deny("Cambio de modo"),
        // Reads inside the project never ask; one that asks is outside it, so a person
        // decides, as with anything the policy cannot classify.
        _ => PolicyDecision::AskUser,
    }
}

impl ThreadPolicy for OfficePolicy {
    fn decide(&self, context: &PolicyContext, request: &PolicyRequest, _: &App) -> PolicyDecision {
        let decision = self.decision(context, request);
        if let PolicyDecision::Deny { reason } = &decision {
            self.send(PolicyEvent::Denied {
                agent_id: context.agent_id.clone(),
                reason: reason.clone(),
            });
        }
        decision
    }

    // Thread choices of mode, model or favorites only affect their session; a post's model
    // comes from its card in Phase 2, never from settings.json.
    fn uses_saved_preferences(&self) -> bool {
        false
    }

    fn start_session(
        &self,
        agent_id: &AgentId,
        agent_version: Option<&str>,
        _: &App,
    ) -> Result<Option<acp_v2::Meta>, SharedString> {
        if let Some(reason) = self.version_problem(agent_version) {
            self.send(PolicyEvent::Denied {
                agent_id: agent_id.clone(),
                reason: reason.clone(),
            });
            return Err(reason);
        }
        Ok(self.session_meta())
    }

    fn permission_pending(
        &self,
        context: &PolicyContext,
        thread: WeakEntity<AcpThread>,
        request: PermissionRequestId,
        _: &App,
    ) {
        self.send(PolicyEvent::Pending {
            agent_id: context.agent_id.clone(),
            thread,
            request,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{StreamExt as _, channel::mpsc};
    use std::path::Path;

    fn policy(provider: Provider) -> (OfficePolicy, mpsc::UnboundedReceiver<PolicyEvent>) {
        let (sender, receiver) = mpsc::unbounded();
        (OfficePolicy::new(provider, sender), receiver)
    }

    fn decide(policy: &OfficePolicy, version: &str, request: &PolicyRequest) -> PolicyDecision {
        let agent_id = AgentId::new("office-claude-default");
        let session_id = acp_v2::SessionId::new("session");
        policy.decision(
            &PolicyContext {
                agent_id: &agent_id,
                agent_version: Some(version),
                session_id: &session_id,
                parent_session_id: None,
            },
            request,
        )
    }

    fn is_denied(decision: &PolicyDecision) -> bool {
        matches!(decision, PolicyDecision::Deny { .. })
    }

    #[test]
    fn claude_permissions_by_kind() {
        use acp_v2::ToolKind;
        for kind in [
            ToolKind::Edit,
            ToolKind::Delete,
            ToolKind::Move,
            ToolKind::Execute,
            ToolKind::Fetch,
            ToolKind::SwitchMode,
        ] {
            assert!(
                is_denied(&classify_permission(Provider::Claude, Some(&kind))),
                "{kind:?}"
            );
        }
        for kind in [
            Some(ToolKind::Read),
            Some(ToolKind::Search),
            Some(ToolKind::Think),
            Some(ToolKind::Other),
            None,
        ] {
            assert_eq!(
                classify_permission(Provider::Claude, kind.as_ref()),
                PolicyDecision::AskUser,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn codex_permissions_are_always_escalations() {
        for kind in [
            None,
            Some(acp_v2::ToolKind::Read),
            Some(acp_v2::ToolKind::Other),
        ] {
            assert!(is_denied(&classify_permission(
                Provider::Codex,
                kind.as_ref()
            )));
        }
    }

    #[test]
    fn client_writes_terminals_and_modes() {
        let (claude, _events) = policy(Provider::Claude);
        let version = CLAUDE_ADAPTER.version;
        assert!(is_denied(&decide(
            &claude,
            version,
            &PolicyRequest::WriteTextFile {
                path: Path::new("/a/file")
            }
        )));
        assert!(is_denied(&decide(
            &claude,
            version,
            &PolicyRequest::CreateTerminal {
                command: "git",
                args: &[],
                cwd: None
            }
        )));
        for origin in [
            ModeChangeOrigin::Default,
            ModeChangeOrigin::User,
            ModeChangeOrigin::Agent,
        ] {
            for (mode, allowed) in [
                ("default", true),
                ("plan", true),
                ("acceptEdits", false),
                ("auto", false),
                ("bypassPermissions", false),
            ] {
                let decision = decide(&claude, version, &PolicyRequest::SetMode { mode, origin });
                assert_eq!(
                    decision == PolicyDecision::Allow,
                    allowed,
                    "{mode} {origin:?}"
                );
            }
        }

        let (codex, _events) = policy(Provider::Codex);
        let version = CODEX_ADAPTER.version;
        for (mode, allowed) in [
            ("read-only", true),
            ("workspace-write", false),
            ("agent", false),
            ("agent-full-access", false),
        ] {
            let decision = decide(
                &codex,
                version,
                &PolicyRequest::SetMode {
                    mode,
                    origin: ModeChangeOrigin::User,
                },
            );
            assert_eq!(decision == PolicyDecision::Allow, allowed, "{mode}");
        }
    }

    #[gpui::test]
    fn unverified_adapter_version_blocks_the_session(cx: &mut gpui::TestAppContext) {
        let (claude, mut events) = policy(Provider::Claude);
        let agent_id = AgentId::new("office-claude-default");
        let refusal = cx
            .update(|cx| claude.start_session(&agent_id, Some("0.95.0"), cx))
            .expect_err("another version is refused");
        assert!(
            refusal.contains("Versión de adaptador no verificada"),
            "{refusal}"
        );
        cx.update(|cx| claude.start_session(&agent_id, None, cx))
            .expect_err("an unknown version is refused");
        assert!(matches!(events.try_recv(), Ok(PolicyEvent::Denied { .. })));

        // Even an otherwise allowed mode is denied on an unverified version.
        let decision = decide(
            &claude,
            "0.95.0",
            &PolicyRequest::SetMode {
                mode: "default",
                origin: ModeChangeOrigin::User,
            },
        );
        assert!(is_denied(&decision));
        cx.update(|cx| claude.start_session(&agent_id, Some(CLAUDE_ADAPTER.version), cx))
            .expect("the pinned version starts");
    }

    #[test]
    fn office_agents_keep_preferences_out_of_settings() {
        for provider in Provider::ALL {
            let (policy, _events) = policy(provider);
            assert!(!policy.uses_saved_preferences());
        }
    }

    #[gpui::test]
    fn claude_sessions_disable_tools_and_settings(cx: &mut gpui::TestAppContext) {
        let (claude, _events) = policy(Provider::Claude);
        let meta = cx
            .update(|cx| {
                claude.start_session(
                    &AgentId::new("office-claude-default"),
                    Some(CLAUDE_ADAPTER.version),
                    cx,
                )
            })
            .expect("session starts")
            .expect("Claude sessions carry options");
        assert_eq!(
            meta["claudeCode"]["options"],
            serde_json::json!({
                "disallowedTools": [
                    "Edit", "Write", "MultiEdit", "NotebookEdit", "Bash", "PowerShell", "Monitor"
                ],
                "allowDangerouslySkipPermissions": false,
                "settingSources": [],
            })
        );

        let (codex, _events) = policy(Provider::Codex);
        let meta = cx
            .update(|cx| {
                codex.start_session(
                    &AgentId::new("office-codex-default"),
                    Some(CODEX_ADAPTER.version),
                    cx,
                )
            })
            .expect("session starts");
        assert_eq!(meta, None);
    }

    #[gpui::test]
    async fn denials_are_reported_to_the_panel(cx: &mut gpui::TestAppContext) {
        let (claude, mut events) = policy(Provider::Claude);
        let agent_id = AgentId::new("office-claude-default");
        let session_id = acp_v2::SessionId::new("session");
        cx.update(|cx| {
            claude.decide(
                &PolicyContext {
                    agent_id: &agent_id,
                    agent_version: Some(CLAUDE_ADAPTER.version),
                    session_id: &session_id,
                    parent_session_id: None,
                },
                &PolicyRequest::SetMode {
                    mode: "acceptEdits",
                    origin: ModeChangeOrigin::User,
                },
                cx,
            )
        });
        match events.next().await {
            Some(PolicyEvent::Denied {
                agent_id: denied,
                reason,
            }) => {
                assert_eq!(denied, agent_id);
                assert!(reason.contains("acceptEdits"), "{reason}");
            }
            _ => panic!("expected a denial event"),
        }
    }
}
