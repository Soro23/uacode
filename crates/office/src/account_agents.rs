use crate::{
    accounts::{AccountsDocument, default_folder},
    office_store::Provider,
};
use anyhow::{Context as _, Result};
use collections::{HashMap, HashSet};
use gpui::{App, AsyncApp, Entity, SharedString, Task};
use project::{
    AgentId, Project, ProjectEnvironment,
    agent_server_store::{
        AgentServerCommand, AgentServerStore, ExternalAgentEntry, ExternalAgentServer,
        ExternalAgentSource,
    },
};
use std::{
    any::Any,
    path::{Path, PathBuf},
};

/// Reserved for Office: the agent store lets these ids win over settings entries.
pub const AGENT_ID_PREFIX: &str = "office-";

const CLAUDE_ADAPTER: &str = "@agentclientprotocol/claude-agent-acp@0.94.0";
const CODEX_ADAPTER: &str = "@agentclientprotocol/codex-acp@2.5.0";
const CODEX_INITIAL_MODE_VARIABLE: &str = "INITIAL_AGENT_MODE";
const CODEX_INITIAL_MODE: &str = "read-only";

/// Cleared so an inherited key never replaces the account's subscription login.
const CLEARED_CREDENTIAL_VARIABLES: [&str; 5] = [
    "ANTHROPIC_API_KEY",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "OPENAI_API_KEY",
    "OPEN_AI_API_KEY",
    "CODEX_API_KEY",
];

/// Identifies an account row: `None` is the provider's virtual default account.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AccountKey {
    pub provider: Provider,
    pub account_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountAgent {
    pub key: AccountKey,
    pub agent_id: AgentId,
    pub display_name: SharedString,
    pub folder: PathBuf,
}

impl AccountAgent {
    pub fn command(&self, extra_args: Vec<String>) -> AgentServerCommand {
        adapter_command(self.key.provider, &self.folder, extra_args)
    }
}

/// One agent per (provider, account), including the default account of each provider.
pub fn account_agents(document: &AccountsDocument, credentials_root: &Path) -> Vec<AccountAgent> {
    let mut agents = Vec::new();
    for provider in Provider::ALL {
        agents.push(AccountAgent {
            key: AccountKey {
                provider,
                account_id: None,
            },
            agent_id: agent_id(provider, "default"),
            display_name: display_name(provider, "Por defecto"),
            folder: credentials_root.join(default_folder(provider)),
        });
        for account in document.accounts_for(provider) {
            let short_id: String = account.id.chars().take(8).collect();
            agents.push(AccountAgent {
                key: AccountKey {
                    provider,
                    account_id: Some(account.id.clone()),
                },
                agent_id: agent_id(provider, &short_id),
                display_name: display_name(provider, &account.name),
                folder: credentials_root.join(account.folder_name()),
            });
        }
    }
    agents
}

fn agent_id(provider: Provider, suffix: &str) -> AgentId {
    AgentId::new(format!("{AGENT_ID_PREFIX}{}-{suffix}", provider.slug()))
}

fn display_name(provider: Provider, account_label: &str) -> SharedString {
    format!("Office · {} · {account_label}", provider.label()).into()
}

pub fn adapter_command(
    provider: Provider,
    folder: &Path,
    extra_args: Vec<String>,
) -> AgentServerCommand {
    let (package, config_variable) = match provider {
        Provider::Claude => (CLAUDE_ADAPTER, "CLAUDE_CONFIG_DIR"),
        Provider::Codex => (CODEX_ADAPTER, "CODEX_HOME"),
    };
    let mut args = vec!["-y".to_string(), package.to_string()];
    args.extend(extra_args);
    let mut env = CLEARED_CREDENTIAL_VARIABLES
        .iter()
        .map(|name| (name.to_string(), String::new()))
        .collect::<HashMap<_, _>>();
    env.insert(
        config_variable.to_string(),
        folder.to_string_lossy().into_owned(),
    );
    if provider == Provider::Codex {
        // codex-acp otherwise starts sessions in "agent" mode, where an automatic reviewer
        // approves workspace writes instead of the user. Phase 1 is read-only.
        env.insert(
            CODEX_INITIAL_MODE_VARIABLE.to_string(),
            CODEX_INITIAL_MODE.to_string(),
        );
    }
    AgentServerCommand {
        // On Windows `npx` resolves to a PowerShell script that cannot be spawned directly.
        path: if cfg!(windows) { "npx.cmd" } else { "npx" }.into(),
        args,
        env: Some(env),
    }
}

struct OfficeAgentServer {
    agent: AccountAgent,
    project_environment: Entity<ProjectEnvironment>,
}

impl ExternalAgentServer for OfficeAgentServer {
    fn get_command(
        &mut self,
        extra_args: Vec<String>,
        extra_env: HashMap<String, String>,
        cx: &mut AsyncApp,
    ) -> Task<Result<AgentServerCommand>> {
        let mut command = self.agent.command(extra_args);
        let project_environment = self.project_environment.downgrade();
        cx.spawn(async move |cx| {
            let mut env = project_environment
                .update(cx, |environment, cx| environment.default_environment(cx))?
                .await
                .unwrap_or_default();
            env.extend(extra_env);
            // Applied last: the account folder and the cleared keys must not be overridden
            // by the shell, the proxy settings or an auth method's environment.
            env.extend(command.env.take().unwrap_or_default());
            command.env = Some(env);
            Ok(command)
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Keeps the project's agent store in sync with the account catalog.
#[derive(Default)]
pub struct RegisteredAgents {
    agents: Vec<AccountAgent>,
}

impl RegisteredAgents {
    pub fn get(&self, key: &AccountKey) -> Option<&AccountAgent> {
        self.agents.iter().find(|agent| &agent.key == key)
    }

    /// Returns the agents that were removed, so their processes can be stopped.
    pub fn sync(
        &mut self,
        agents: Vec<AccountAgent>,
        project: &Entity<Project>,
        cx: &mut App,
    ) -> Result<Vec<AccountAgent>> {
        let store = project.read(cx).agent_server_store().clone();
        let project_environment = project.read(cx).environment().clone();
        let current = agents
            .iter()
            .map(|agent| &agent.agent_id)
            .collect::<HashSet<_>>();
        let (kept, removed): (Vec<_>, Vec<_>) = std::mem::take(&mut self.agents)
            .into_iter()
            .partition(|agent| current.contains(&agent.agent_id));
        for agent in &removed {
            store.update(cx, |store, cx| {
                store.unregister_dynamic_agent(&agent.agent_id, cx)
            });
        }
        for agent in &agents {
            if kept.contains(agent) {
                continue;
            }
            register(&store, agent.clone(), project_environment.clone(), cx)?;
        }
        self.agents = agents;
        Ok(removed)
    }

    pub fn unregister_all(&mut self, project: &Entity<Project>, cx: &mut App) {
        let store = project.read(cx).agent_server_store().clone();
        for agent in self.agents.drain(..) {
            store.update(cx, |store, cx| {
                store.unregister_dynamic_agent(&agent.agent_id, cx)
            });
        }
    }
}

fn register(
    store: &Entity<AgentServerStore>,
    agent: AccountAgent,
    project_environment: Entity<ProjectEnvironment>,
    cx: &mut App,
) -> Result<()> {
    let agent_id = agent.agent_id.clone();
    let display_name = agent.display_name.clone();
    store
        .update(cx, |store, cx| {
            store.register_dynamic_agent(
                agent_id.clone(),
                ExternalAgentEntry::new(
                    Box::new(OfficeAgentServer {
                        agent,
                        project_environment,
                    }),
                    ExternalAgentSource::Custom,
                    None,
                    Some(display_name),
                ),
                cx,
            )
        })
        .with_context(|| format!("No se pudo registrar el agente {agent_id}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::Account;
    use fs::Fs as _;
    use time::OffsetDateTime;

    fn account(id: &str, provider: Provider, name: &str, folder: Option<&str>) -> Account {
        Account {
            id: id.into(),
            provider,
            name: name.into(),
            created_at: OffsetDateTime::UNIX_EPOCH,
            folder: folder.map(Into::into),
        }
    }

    fn document() -> AccountsDocument {
        AccountsDocument {
            accounts: vec![
                account(
                    "031a5164-0000-4000-8000-000000000000",
                    Provider::Claude,
                    "Empresa",
                    None,
                ),
                account(
                    "9f00aa11-0000-4000-8000-000000000000",
                    Provider::Claude,
                    "Personal",
                    Some("claude-b"),
                ),
                account(
                    "5e5e5e5e-0000-4000-8000-000000000000",
                    Provider::Codex,
                    "Trabajo",
                    None,
                ),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn builds_one_agent_per_account_and_default() {
        let root = Path::new("/config/accounts");
        let agents = account_agents(&document(), root);
        let summary = agents
            .iter()
            .map(|agent| {
                (
                    agent.agent_id.to_string(),
                    agent.display_name.to_string(),
                    agent.folder.clone(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            summary,
            vec![
                (
                    "office-claude-default".into(),
                    "Office · Claude · Por defecto".into(),
                    root.join("claude-default"),
                ),
                (
                    "office-claude-031a5164".into(),
                    "Office · Claude · Empresa".into(),
                    root.join("claude-031a5164"),
                ),
                (
                    "office-claude-9f00aa11".into(),
                    "Office · Claude · Personal".into(),
                    root.join("claude-b"),
                ),
                (
                    "office-codex-default".into(),
                    "Office · Codex · Por defecto".into(),
                    root.join("codex-default"),
                ),
                (
                    "office-codex-5e5e5e5e".into(),
                    "Office · Codex · Trabajo".into(),
                    root.join("codex-5e5e5e5e"),
                ),
            ]
        );
        assert!(
            agents
                .iter()
                .all(|agent| agent.agent_id.as_ref().starts_with(AGENT_ID_PREFIX))
        );
    }

    fn expected_env(config_variable: &str, folder: &Path) -> HashMap<String, String> {
        let mut env = HashMap::from_iter([
            ("ANTHROPIC_API_KEY".to_string(), String::new()),
            ("CLAUDE_CODE_OAUTH_TOKEN".to_string(), String::new()),
            ("OPENAI_API_KEY".to_string(), String::new()),
            ("OPEN_AI_API_KEY".to_string(), String::new()),
            ("CODEX_API_KEY".to_string(), String::new()),
        ]);
        env.insert(
            config_variable.to_string(),
            folder.to_string_lossy().into_owned(),
        );
        env
    }

    #[test]
    fn claude_command_uses_pinned_adapter_and_account_folder() {
        let folder = Path::new("/config/accounts/claude-031a5164");
        let command = adapter_command(Provider::Claude, folder, vec!["--cli".into()]);
        let npx = if cfg!(windows) { "npx.cmd" } else { "npx" };
        assert_eq!(command.path, PathBuf::from(npx));
        assert_eq!(
            command.args,
            vec![
                "-y",
                "@agentclientprotocol/claude-agent-acp@0.94.0",
                "--cli"
            ]
        );
        assert_eq!(command.env, Some(expected_env("CLAUDE_CONFIG_DIR", folder)));
    }

    #[test]
    fn codex_command_uses_pinned_adapter_and_codex_home() {
        let folder = Path::new("/config/accounts/codex-default");
        let command = adapter_command(Provider::Codex, folder, Vec::new());
        assert_eq!(
            command.args,
            vec!["-y", "@agentclientprotocol/codex-acp@2.5.0"]
        );
        let mut env = expected_env("CODEX_HOME", folder);
        env.insert("INITIAL_AGENT_MODE".into(), "read-only".into());
        assert_eq!(command.env, Some(env));
    }

    // `gpui::test` discards the returned value, so failures must surface through `expect`.
    #[gpui::test]
    async fn registering_accounts_creates_no_folders(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
        });
        let fs = fs::FakeFs::new(cx.executor());
        fs.insert_tree("/project", serde_json::json!({})).await;
        fs.insert_tree("/config", serde_json::json!({})).await;
        let project = Project::test(fs.clone(), [Path::new("/project")], cx).await;
        let root = Path::new("/config/accounts");

        let mut document = AccountsDocument::default();
        let added = document
            .add(Provider::Claude, "Empresa", OffsetDateTime::UNIX_EPOCH)
            .expect("add account");
        let mut registered = RegisteredAgents::default();
        cx.update(|cx| registered.sync(account_agents(&document, root), &project, cx))
            .expect("register agents");

        let store = project.read_with(cx, |project, _| project.agent_server_store().clone());
        store.read_with(cx, |store, _| {
            let mut ids = store
                .external_agents()
                .map(|id| id.to_string())
                .filter(|id| id.starts_with(AGENT_ID_PREFIX))
                .collect::<Vec<_>>();
            ids.sort();
            let short_id: String = added.id.chars().take(8).collect();
            assert_eq!(
                ids,
                vec![
                    format!("office-claude-{short_id}"),
                    "office-claude-default".to_string(),
                    "office-codex-default".to_string(),
                ]
            );
        });
        assert!(
            fs.metadata(root).await.expect("read metadata").is_none(),
            "adding and registering an account must not create credential folders"
        );

        document.accounts.clear();
        let removed = cx
            .update(|cx| registered.sync(account_agents(&document, root), &project, cx))
            .expect("unregister removed account");
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].key.account_id, Some(added.id.clone()));
        store.read_with(cx, |store, _| {
            assert_eq!(
                store
                    .external_agents()
                    .filter(|id| id.as_ref().starts_with(AGENT_ID_PREFIX))
                    .count(),
                2
            );
        });
    }
}
