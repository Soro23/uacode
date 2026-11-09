use crate::{
    account_agents::{self, AccountAgent, AccountKey, RegisteredAgents},
    account_login,
    accounts::{self, AccountChoice, AccountsDocument, AccountsFile, ImportReport, default_folder},
    auth_status::{self, AuthStatus},
    office_policy::PolicyEvent,
    office_store::{Post, Provider},
    post_threads::PostThreads,
    versioned_file,
};
use acp_thread::{AcpThread, AcpThreadEvent, PermissionRequestId};
use anyhow::{Context as _, Result};
use collections::HashMap;
use fs::Fs;
use futures::{StreamExt as _, channel::mpsc};
use gpui::{
    AsyncApp, Context, Entity, Focusable as _, IntoElement, Render, Subscription, Task, WeakEntity,
    Window,
};
use project::{AgentId, Project};
use std::{collections::VecDeque, path::PathBuf, sync::Arc};
use time::OffsetDateTime;
use ui::prelude::*;
use ui_input::InputField;
use util::ResultExt as _;
use workspace::Workspace;

const MAX_CONCURRENT_PROBES: usize = 2;

struct AccountDraft {
    provider: Provider,
    name: Entity<InputField>,
}

/// Only kept in memory: the email and plan are never written to disk or logged.
#[derive(Clone, Debug, PartialEq, Eq)]
enum AccountStatus {
    Untested,
    Queued,
    Checking,
    LoggingIn,
    Connected {
        email: Option<String>,
        plan: Option<String>,
    },
    ConnectedWithOtherCredentials(String),
    RequiresLogin,
    Error(String),
}

impl AccountStatus {
    fn from_probe(result: Result<Option<AuthStatus>>) -> Self {
        match result {
            Ok(None) | Ok(Some(AuthStatus::LoggedOut)) => Self::RequiresLogin,
            Ok(Some(AuthStatus::Account { email, plan })) => Self::Connected { email, plan },
            Ok(Some(AuthStatus::OtherCredentials { kind, label })) => {
                Self::ConnectedWithOtherCredentials(label.unwrap_or(kind))
            }
            Err(error) => Self::Error(format!("{error:#}")),
        }
    }

    fn label(&self) -> String {
        match self {
            Self::Untested => "Sin probar".into(),
            Self::Queued => "En cola".into(),
            Self::Checking => "Comprobando…".into(),
            Self::LoggingIn => "Iniciando sesión…".into(),
            Self::Connected { email, plan } => {
                let mut label = "Conectada".to_string();
                for detail in [email, plan].into_iter().flatten() {
                    label.push_str(" · ");
                    label.push_str(detail);
                }
                label
            }
            Self::ConnectedWithOtherCredentials(credentials) => {
                format!("Conectada con {credentials}, no con la suscripción")
            }
            Self::RequiresLogin => "Requiere login".into(),
            Self::Error(message) => format!("Error: {message}"),
        }
    }

    fn color(&self) -> Color {
        match self {
            Self::Connected { .. } => Color::Success,
            Self::ConnectedWithOtherCredentials(_) | Self::RequiresLogin => Color::Warning,
            Self::Error(_) => Color::Error,
            Self::Untested | Self::Queued | Self::Checking | Self::LoggingIn => Color::Muted,
        }
    }

    fn is_busy(&self) -> bool {
        matches!(self, Self::Queued | Self::Checking | Self::LoggingIn)
    }
}

/// A permission request Office's policy left to a person, shown on its post's card.
struct PendingApproval {
    post_id: String,
    request: PermissionRequestId,
    title: String,
    thread: WeakEntity<AcpThread>,
    _subscription: Subscription,
}

/// Global account catalog editor shown inside the Office panel.
pub struct AccountsView {
    workspace: WeakEntity<Workspace>,
    project: WeakEntity<Project>,
    fs: Arc<dyn Fs>,
    path: PathBuf,
    credentials_root: PathBuf,
    file: Option<AccountsFile>,
    error: Option<String>,
    notice: Option<String>,
    draft: Option<AccountDraft>,
    loading: bool,
    saving: bool,
    agents: RegisteredAgents,
    agents_error: Option<String>,
    statuses: HashMap<AccountKey, AccountStatus>,
    probe_queue: VecDeque<AccountKey>,
    // Dropping a task kills its adapter process, so removing an entry stops it.
    probes: HashMap<AccountKey, Task<()>>,
    logins: HashMap<AccountKey, Task<()>>,
    post_threads: PostThreads,
    policy_events: mpsc::UnboundedSender<PolicyEvent>,
    /// The latest denial or block for each post, until the user dismisses it.
    policy_notices: HashMap<String, SharedString>,
    pending_approvals: Vec<PendingApproval>,
    _policy_events_task: Task<()>,
    _load_task: Task<()>,
    _save_task: Task<()>,
}

impl AccountsView {
    pub fn new(
        workspace: WeakEntity<Workspace>,
        fs: Arc<dyn Fs>,
        post_threads: PostThreads,
        cx: &mut Context<Self>,
    ) -> Self {
        let project = workspace
            .upgrade()
            .map(|workspace| workspace.read(cx).project().downgrade())
            .unwrap_or_else(WeakEntity::new_invalid);
        cx.on_release(|view, cx| {
            if let Some(project) = view.project.upgrade() {
                view.agents.unregister_all(&project, cx);
            }
        })
        .detach();
        let (policy_events, mut policy_receiver) = mpsc::unbounded();
        let policy_events_task = cx.spawn(async move |this, cx| {
            while let Some(event) = policy_receiver.next().await {
                if this
                    .update(cx, |view, cx| view.handle_policy_event(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let mut view = Self {
            workspace,
            project,
            fs,
            path: accounts::accounts_path(),
            credentials_root: accounts::credentials_root(),
            file: None,
            error: None,
            notice: None,
            draft: None,
            loading: false,
            saving: false,
            agents: RegisteredAgents::default(),
            agents_error: None,
            statuses: HashMap::default(),
            probe_queue: VecDeque::default(),
            probes: HashMap::default(),
            logins: HashMap::default(),
            post_threads,
            policy_events,
            policy_notices: HashMap::default(),
            pending_approvals: Vec::new(),
            _policy_events_task: policy_events_task,
            _load_task: Task::ready(()),
            _save_task: Task::ready(()),
        };
        view.reload(cx);
        view
    }

    pub fn document(&self) -> Option<&AccountsDocument> {
        self.file.as_ref().map(|file| &file.document)
    }

    /// Status of the account a post uses, or `None` when the account is missing.
    pub fn post_status_label(&self, post: &Post) -> Option<String> {
        let key = self.post_account_key(post)?;
        Some(self.status(&key).label())
    }

    /// Whether the post's account needs a login, and its last error, from the latest check.
    pub fn post_account_problem(&self, post: &Post) -> (bool, Option<String>) {
        let Some(key) = self.post_account_key(post) else {
            return (false, None);
        };
        match self.status(&key) {
            AccountStatus::RequiresLogin => (true, None),
            AccountStatus::Error(error) => (false, Some(error.clone())),
            _ => (false, None),
        }
    }

    /// The Office agent a post's threads run on, or `None` when its account is missing.
    pub fn post_agent_id(&self, post: &Post) -> Option<&AgentId> {
        let key = self.post_account_key(post)?;
        self.agents.get(&key).map(|agent| &agent.agent_id)
    }

    fn post_account_key(&self, post: &Post) -> Option<AccountKey> {
        match self.document()?.resolve(post) {
            AccountChoice::Default => Some(AccountKey {
                provider: post.provider,
                account_id: None,
            }),
            AccountChoice::Account(account) => Some(AccountKey {
                provider: post.provider,
                account_id: Some(account.id.clone()),
            }),
            AccountChoice::Missing => None,
        }
    }

    pub fn post_policy_notice(&self, post_id: &str) -> Option<SharedString> {
        self.policy_notices.get(post_id).cloned()
    }

    pub fn post_pending_approvals(&self, post_id: &str) -> Vec<(PermissionRequestId, String)> {
        self.pending_approvals
            .iter()
            .filter(|pending| pending.post_id == post_id)
            .map(|pending| (pending.request, pending.title.clone()))
            .collect()
    }

    pub fn dismiss_policy_notice(&mut self, post_id: &str, cx: &mut Context<Self>) {
        if self.policy_notices.remove(post_id).is_some() {
            cx.notify();
        }
    }

    /// Answers a pending request from the panel, as the user would in the thread.
    pub fn answer_approval(
        &mut self,
        request: PermissionRequestId,
        allow: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self
            .pending_approvals
            .iter()
            .find(|pending| pending.request == request)
        else {
            return;
        };
        let answered = pending.thread.upgrade().is_some_and(|thread| {
            thread.update(cx, |thread, cx| {
                thread.answer_permission_request(request, allow, cx)
            })
        });
        // Answered requests leave through `ToolAuthorizationReceived`; one that could not be
        // answered is gone from its thread and must not stay on the card.
        if !answered {
            log::warn!(
                "Pending approval {request:?} could not be answered; removing it from the card"
            );
            self.pending_approvals
                .retain(|pending| pending.request != request);
            cx.notify();
        }
    }

    /// Forgets a post's pending approvals, for example when it starts a new thread.
    pub fn forget_post_approvals(&mut self, post_id: &str, cx: &mut Context<Self>) {
        let before = self.pending_approvals.len();
        self.pending_approvals
            .retain(|pending| pending.post_id != post_id);
        if self.pending_approvals.len() != before {
            cx.notify();
        }
    }

    fn handle_policy_event(&mut self, event: PolicyEvent, cx: &mut Context<Self>) {
        match event {
            // Threads outside Office's posts show their denials in the thread only.
            PolicyEvent::Denied {
                post_id: Some(post_id),
                reason,
            } => {
                self.policy_notices.insert(post_id, reason);
            }
            PolicyEvent::Denied { post_id: None, .. } => return,
            PolicyEvent::Pending { post_id: None, .. } => return,
            PolicyEvent::Pending {
                post_id: Some(post_id),
                thread,
                request,
            } => {
                let Some(thread) = thread.upgrade() else {
                    return;
                };
                // Answered in the thread before this event arrived.
                let Some(title) = thread.read(cx).permission_request_title(request, cx) else {
                    return;
                };
                // Any change in the thread can end the request: an answer, a cancelled
                // request or turn, or a tool call that finished.
                let subscription = cx.subscribe(&thread, move |view, thread, event, cx| {
                    let answered = matches!(
                        event,
                        AcpThreadEvent::ToolAuthorizationReceived(id) if *id == request
                    );
                    if answered || thread.read(cx).permission_request(request).is_none() {
                        view.pending_approvals
                            .retain(|pending| pending.request != request);
                        cx.notify();
                    }
                });
                self.pending_approvals.push(PendingApproval {
                    post_id,
                    request,
                    title,
                    thread: thread.downgrade(),
                    _subscription: subscription,
                });
            }
        }
        cx.notify();
    }

    fn status(&self, key: &AccountKey) -> &AccountStatus {
        self.statuses.get(key).unwrap_or(&AccountStatus::Untested)
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.file = None;
        self.error = None;
        self.notice = None;
        self.draft = None;
        self.loading = true;
        let fs = self.fs.clone();
        let path = self.path.clone();
        self._load_task = cx.spawn(async move |this, cx| {
            let result = versioned_file::load::<AccountsDocument>(fs.as_ref(), &path).await;
            this.update(cx, |view, cx| {
                view.loading = false;
                match result {
                    Ok(file) => {
                        view.file = Some(file);
                        view.sync_agents(cx);
                    }
                    Err(error) => {
                        view.error = Some(format!("No se pudo leer accounts.json: {error:#}"))
                    }
                }
                cx.notify();
            })
            .log_err();
        });
        cx.notify();
    }

    /// Registering does not start any process; adapters only run on login or check.
    fn sync_agents(&mut self, cx: &mut Context<Self>) {
        let (Some(project), Some(document)) = (self.project.upgrade(), self.document()) else {
            return;
        };
        if !project.read(cx).is_local() {
            return;
        }
        let agents = account_agents::account_agents(document, &self.credentials_root);
        match self.agents.sync(
            agents,
            &project,
            &self.post_threads,
            &self.policy_events,
            cx,
        ) {
            Ok(removed) => {
                self.agents_error = None;
                for agent in removed {
                    self.forget_agent(&agent);
                }
            }
            Err(error) => self.agents_error = Some(format!("{error:#}")),
        }
    }

    fn forget_agent(&mut self, agent: &AccountAgent) {
        let key = &agent.key;
        self.statuses.remove(key);
        self.probe_queue.retain(|queued| queued != key);
        self.probes.remove(key);
        self.logins.remove(key);
    }

    fn check(&mut self, key: AccountKey, cx: &mut Context<Self>) {
        if self.probes.contains_key(&key) || self.probe_queue.contains(&key) {
            return;
        }
        self.statuses.insert(key.clone(), AccountStatus::Queued);
        self.probe_queue.push_back(key);
        self.start_queued_probes(cx);
        cx.notify();
    }

    fn start_queued_probes(&mut self, cx: &mut Context<Self>) {
        while self.probes.len() < MAX_CONCURRENT_PROBES {
            let Some(key) = self.probe_queue.pop_front() else {
                return;
            };
            self.start_probe(key, cx);
        }
    }

    fn start_probe(&mut self, key: AccountKey, cx: &mut Context<Self>) {
        let Some(agent) = self.agents.get(&key).cloned() else {
            self.statuses.insert(
                key,
                AccountStatus::Error("la cuenta no está registrada".into()),
            );
            return;
        };
        self.statuses.insert(key.clone(), AccountStatus::Checking);
        let fs = self.fs.clone();
        let project = self.project.clone();
        let task = cx.spawn({
            let key = key.clone();
            async move |this, cx| {
                let result = probe_account(agent, fs, project, cx).await;
                this.update(cx, |view, cx| {
                    view.probes.remove(&key);
                    view.statuses.insert(key, AccountStatus::from_probe(result));
                    view.start_queued_probes(cx);
                    cx.notify();
                })
                .log_err();
            }
        });
        self.probes.insert(key, task);
    }

    fn login(&mut self, key: AccountKey, window: &mut Window, cx: &mut Context<Self>) {
        if self.logins.contains_key(&key) {
            return;
        }
        let (Some(agent), Some(project)) = (self.agents.get(&key).cloned(), self.project.upgrade())
        else {
            return;
        };
        self.statuses.insert(key.clone(), AccountStatus::LoggingIn);
        let workspace = self.workspace.clone();
        let fs = self.fs.clone();
        let task = cx.spawn_in(window, {
            let key = key.clone();
            async move |this, cx| {
                let result = account_login::login(agent, project, workspace, fs, cx).await;
                this.update(cx, |view, cx| {
                    view.logins.remove(&key);
                    match result {
                        Ok(()) => view.check(key, cx),
                        Err(error) => {
                            view.statuses.insert(
                                key,
                                AccountStatus::Error(format!("inicio de sesión: {error:#}")),
                            );
                        }
                    }
                    cx.notify();
                })
                .log_err();
            }
        });
        self.logins.insert(key, task);
        cx.notify();
    }

    fn writable(&self) -> bool {
        !self.loading && !self.saving && self.file.as_ref().is_some_and(|file| !file.is_read_only())
    }

    fn start_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.writable() {
            return;
        }
        let name = cx.new(|cx| InputField::new(window, cx, "Nombre").label("Nombre de la cuenta"));
        name.focus_handle(cx).focus(window, cx);
        self.draft = Some(AccountDraft {
            provider: Provider::Claude,
            name,
        });
        self.error = None;
        self.notice = None;
        cx.notify();
    }

    fn save_draft(&mut self, cx: &mut Context<Self>) {
        let (Some(draft), Some(file)) = (&self.draft, &self.file) else {
            return;
        };
        let name = draft.name.read(cx).text(cx);
        let mut document = file.document.clone();
        match document.add(draft.provider, &name, OffsetDateTime::now_utc()) {
            Ok(account) => self.persist(
                document,
                format!(
                    "Cuenta «{}» añadida; su carpeta será {}",
                    account.name,
                    account.folder_name()
                ),
                cx,
            ),
            Err(error) => {
                self.error = Some(format!("{error:#}"));
                cx.notify();
            }
        }
    }

    fn import_existing(&mut self, cx: &mut Context<Self>) {
        if !self.writable() {
            return;
        }
        let fs = self.fs.clone();
        let root = self.credentials_root.clone();
        self.saving = true;
        self.error = None;
        self.notice = None;
        self._save_task = cx.spawn(async move |this, cx| {
            let folders = list_folder_names(fs.as_ref(), &root).await;
            this.update(cx, |view, cx| {
                view.saving = false;
                match folders {
                    Ok(folders) => view.apply_import(folders, cx),
                    Err(error) => {
                        view.error = Some(format!(
                            "No se pudieron listar las carpetas de {}: {error:#}",
                            root.display()
                        ));
                        cx.notify();
                    }
                }
            })
            .log_err();
        });
        cx.notify();
    }

    fn apply_import(&mut self, folders: Vec<String>, cx: &mut Context<Self>) {
        let Some(file) = &self.file else {
            return;
        };
        let mut document = file.document.clone();
        let report = document.import_folders(folders, OffsetDateTime::now_utc());
        let notice = import_notice(&report);
        if report.imported.is_empty() {
            self.notice = Some(notice);
            cx.notify();
        } else {
            self.persist(document, notice, cx);
        }
    }

    fn persist(&mut self, document: AccountsDocument, notice: String, cx: &mut Context<Self>) {
        if !self.writable() {
            return;
        }
        let Some(previous) = self.file.clone() else {
            return;
        };
        self.saving = true;
        self.error = None;
        let fs = self.fs.clone();
        let path = self.path.clone();
        self._save_task = cx.spawn(async move |this, cx| {
            let result = versioned_file::save(fs.as_ref(), &path, &previous, document).await;
            this.update(cx, |view, cx| {
                view.saving = false;
                match result {
                    Ok(file) => {
                        view.file = Some(file);
                        view.draft = None;
                        view.notice = Some(notice);
                        view.sync_agents(cx);
                    }
                    Err(error) => {
                        view.error = Some(format!("No se pudo guardar accounts.json: {error:#}"))
                    }
                }
                cx.notify();
            })
            .log_err();
        });
        cx.notify();
    }

    fn open_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.path.clone();
        match self.workspace.update(cx, |workspace, cx| {
            workspace.open_abs_path(path, Default::default(), window, cx)
        }) {
            Ok(task) => cx
                .spawn(async move |this, cx| {
                    if let Err(error) = task.await {
                        this.update(cx, |view, cx| {
                            view.error = Some(format!("No se pudo abrir accounts.json: {error:#}"));
                            cx.notify();
                        })
                        .log_err();
                    }
                })
                .detach(),
            Err(error) => {
                self.error = Some(format!("No se pudo abrir accounts.json: {error:#}"));
                cx.notify();
            }
        }
    }

    fn render_draft(&self, draft: &AccountDraft, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_2()
            .child(
                Button::new(
                    "account-provider",
                    format!("Proveedor: {}", draft.provider.label()),
                )
                .disabled(self.saving)
                .on_click(cx.listener(|view, _, _, cx| {
                    if let Some(draft) = &mut view.draft {
                        draft.provider = match draft.provider {
                            Provider::Claude => Provider::Codex,
                            Provider::Codex => Provider::Claude,
                        };
                    }
                    cx.notify();
                })),
            )
            .child(draft.name.clone())
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("save-account", "Guardar")
                            .disabled(!self.writable())
                            .on_click(cx.listener(|view, _, _, cx| view.save_draft(cx))),
                    )
                    .child(
                        Button::new("cancel-account", "Cancelar")
                            .disabled(self.saving)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.draft = None;
                                view.error = None;
                                cx.notify();
                            })),
                    ),
            )
    }

    fn render_provider(&self, provider: Provider, cx: &mut Context<Self>) -> impl IntoElement {
        let mut column = v_flex()
            .gap_1()
            .child(Label::new(provider.label()).color(Color::Muted))
            .child(self.render_account_row(
                AccountKey {
                    provider,
                    account_id: None,
                },
                "Por defecto".into(),
                default_folder(provider),
                cx,
            ));
        if let Some(document) = self.document() {
            let accounts = document
                .accounts_for(provider)
                .map(|account| {
                    (
                        AccountKey {
                            provider,
                            account_id: Some(account.id.clone()),
                        },
                        account.name.clone(),
                        account.folder_name(),
                    )
                })
                .collect::<Vec<_>>();
            for (key, name, folder) in accounts {
                column = column.child(self.render_account_row(key, name, folder, cx));
            }
        }
        column
    }

    fn render_account_row(
        &self,
        key: AccountKey,
        name: String,
        folder: String,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let status = self.status(&key);
        let is_registered = self.agents.get(&key).is_some();
        let is_busy = status.is_busy();
        let element_id = account_element_id(&key);
        v_flex()
            .p_2()
            .gap_1()
            .border_1()
            .border_color(cx.theme().colors().border)
            .child(Label::new(name))
            .child(
                Label::new(format!("Carpeta: {folder}"))
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .child(
                Label::new(status.label())
                    .size(LabelSize::Small)
                    .color(status.color()),
            )
            .when(is_registered, |row| {
                row.child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new(format!("{element_id}-login"), "Iniciar sesión")
                                .disabled(is_busy)
                                .on_click(cx.listener({
                                    let key = key.clone();
                                    move |view, _, window, cx| view.login(key.clone(), window, cx)
                                })),
                        )
                        .child(
                            Button::new(format!("{element_id}-check"), "Comprobar")
                                .disabled(is_busy)
                                .on_click(cx.listener({
                                    let key = key.clone();
                                    move |view, _, _, cx| view.check(key.clone(), cx)
                                })),
                        ),
                )
            })
    }
}

fn account_element_id(key: &AccountKey) -> String {
    format!(
        "account-{}-{}",
        key.provider.slug(),
        key.account_id.as_deref().unwrap_or("default")
    )
}

/// A missing folder means the account never logged in; the adapter is not started then,
/// because it writes into its folder as soon as it runs.
async fn probe_account(
    agent: AccountAgent,
    fs: Arc<dyn Fs>,
    project: WeakEntity<Project>,
    cx: &mut AsyncApp,
) -> Result<Option<AuthStatus>> {
    if !fs.is_dir(&agent.folder).await {
        return Ok(None);
    }
    let command = project
        .update(cx, |project, cx| {
            project.agent_server_store().update(cx, |store, cx| {
                store.get_external_agent(&agent.agent_id).map(|server| {
                    server.get_command(Vec::new(), HashMap::default(), &mut cx.to_async())
                })
            })
        })?
        .context("la cuenta no está registrada")?
        .await?;
    let executor = cx.background_executor().clone();
    cx.background_spawn(auth_status::probe(command, executor))
        .await
        .map(Some)
}

fn import_notice(report: &ImportReport) -> String {
    let mut notice = if report.imported.is_empty() {
        "No hay carpetas nuevas que importar".to_string()
    } else {
        format!("Importadas: {}", report.imported.join(", "))
    };
    if !report.skipped.is_empty() {
        notice.push_str(&format!(". Omitidas: {}", report.skipped.join("; ")));
    }
    notice
}

/// Lists only the names of the credential folders; their contents are never read.
async fn list_folder_names(fs: &dyn Fs, root: &std::path::Path) -> Result<Vec<String>> {
    let mut entries = fs.read_dir(root).await?;
    let mut names = Vec::new();
    while let Some(entry) = entries.next().await {
        let path = entry?;
        if fs.is_dir(&path).await
            && let Some(name) = path.file_name().and_then(|name| name.to_str())
        {
            names.push(name.to_owned());
        }
    }
    Ok(names)
}

impl Render for AccountsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let writable = self.writable();
        let warning = self.file.as_ref().and_then(|file| file.warning.clone());
        let file_exists = self.file.as_ref().is_some_and(|file| file.exists_on_disk());
        let mut content = v_flex().gap_2().child(Label::new("Cuentas")).child(
            h_flex()
                .gap_2()
                .flex_wrap()
                .child(
                    Button::new("new-account", "Añadir cuenta")
                        .disabled(!writable || self.draft.is_some())
                        .on_click(cx.listener(|view, _, window, cx| view.start_draft(window, cx))),
                )
                .child(
                    Button::new("import-accounts", "Importar carpetas existentes")
                        .disabled(!writable || self.draft.is_some())
                        .on_click(cx.listener(|view, _, _, cx| view.import_existing(cx))),
                )
                .child(
                    Button::new("reload-accounts", "Recargar")
                        .disabled(self.loading || self.saving)
                        .on_click(cx.listener(|view, _, _, cx| view.reload(cx))),
                ),
        );
        if self.loading {
            content = content.child(Label::new("Cargando cuentas…"));
        }
        if let Some(notice) = self.notice.clone() {
            content = content.child(Label::new(notice).color(Color::Muted));
        }
        if let Some(message) = self.error.clone().or(warning) {
            content =
                content.child(Label::new(message).color(Color::Error)).when(
                    file_exists,
                    |content| {
                        content.child(Button::new("open-accounts-file", "Abrir archivo").on_click(
                            cx.listener(|view, _, window, cx| view.open_file(window, cx)),
                        ))
                    },
                );
        }
        if let Some(message) = self.agents_error.clone() {
            content = content.child(Label::new(message).color(Color::Error));
        }
        if let Some(draft) = &self.draft {
            content = content.child(self.render_draft(draft, cx));
        }
        for provider in Provider::ALL {
            content = content.child(self.render_provider(provider, cx));
        }
        content
    }
}
