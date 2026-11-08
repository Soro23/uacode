use crate::{account_agents::AccountAgent, office_store::Provider};
use acp_thread::AgentConnection;
use agent_client_protocol::v2::schema::AuthMethodId;
use agent_servers::{AgentServer as _, AgentServerDelegate, CustomAgentServer};
use anyhow::{Context as _, Result, anyhow, bail};
use fs::Fs;
use gpui::{AsyncWindowContext, Entity, WeakEntity};
use project::Project;
use std::{rc::Rc, sync::Arc};
use terminal_view::terminal_panel::TerminalPanel;
use workspace::Workspace;

/// Subscription login offered by each pinned adapter: "Claude Subscription" runs in a
/// terminal, while Codex "ChatGPT" is handled by the agent through `authenticate`.
fn login_method(provider: Provider) -> AuthMethodId {
    AuthMethodId::new(match provider {
        Provider::Claude => "claude-ai-login",
        Provider::Codex => "chat-gpt",
    })
}

/// Creates the account folder (only here, never when the account is added) and runs the
/// adapter's subscription login with the account's environment.
pub async fn login(
    agent: AccountAgent,
    project: Entity<Project>,
    workspace: WeakEntity<Workspace>,
    fs: Arc<dyn Fs>,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    fs.create_dir(&agent.folder)
        .await
        .with_context(|| format!("No se pudo crear la carpeta {}", agent.folder.display()))?;

    let connection = connect(&agent, &project, cx).await?;
    let result = run_login_method(&agent, connection.clone(), workspace, cx).await;
    connection.shutdown();
    result
}

async fn connect(
    agent: &AccountAgent,
    project: &Entity<Project>,
    cx: &mut AsyncWindowContext,
) -> Result<Rc<dyn AgentConnection>> {
    let server = CustomAgentServer::new(agent.agent_id.clone());
    cx.update(|_, cx| {
        let store = project.read(cx).agent_server_store().clone();
        server.connect(
            AgentServerDelegate::new(store, None, None),
            project.clone(),
            cx,
        )
    })?
    .await
    .context("No se pudo arrancar el adaptador")
}

async fn run_login_method(
    agent: &AccountAgent,
    connection: Rc<dyn AgentConnection>,
    workspace: WeakEntity<Workspace>,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    let method = login_method(agent.key.provider);
    if !connection
        .auth_methods()
        .iter()
        .any(|advertised| advertised.method_id() == &method)
    {
        bail!("El adaptador no ofrece el inicio de sesión {method}");
    }

    let terminal_login = cx.update(|_, cx| connection.terminal_auth_task(&method, cx))?;
    let Some(terminal_login) = terminal_login else {
        return cx
            .update(|_, cx| connection.authenticate(method, cx))?
            .await
            .context("El inicio de sesión no se completó");
    };

    let mut task = terminal_login.await?;
    task.shell = task::Shell::WithArguments {
        program: task
            .command
            .take()
            .context("El inicio de sesión no tiene comando")?,
        args: std::mem::take(&mut task.args),
        title_override: None,
    };
    let terminal_panel = workspace
        .read_with(cx, |workspace, cx| workspace.panel::<TerminalPanel>(cx))?
        .context("El panel de terminal no está disponible")?;
    let terminal = terminal_panel
        .update_in(cx, |terminal_panel, window, cx| {
            terminal_panel.spawn_task(&task, window, cx)
        })?
        .await?;
    let exit_status = terminal
        .read_with(cx, |terminal, cx| terminal.wait_for_completed_task(cx))?
        .await;
    match exit_status {
        Some(status) if status.success() => Ok(()),
        Some(status) => Err(anyhow!(
            "El inicio de sesión terminó con código {:?}",
            status.code()
        )),
        None => Err(anyhow!("El inicio de sesión terminó sin código de salida")),
    }
}
