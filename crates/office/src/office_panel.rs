use crate::{
    accounts::AccountChoice,
    accounts_view::AccountsView,
    office_store::{self, OfficeFile, Post, Provider},
    post_activity::{self, ThreadActivity},
    post_threads::PostThreads,
};
use acp_thread::AcpThread;
use agent_ui::{
    Agent, AgentPanel, AgentPanelEvent, AgentThreadSource, CreateThreadOptions, ThreadId,
    thread_metadata_store::ThreadMetadataStore,
};
use std::time::Duration;

/// Thread changes arrive on every streamed token; cards refresh at most this often.
const ACTIVITY_REFRESH_INTERVAL: Duration = Duration::from_millis(500);

struct ThreadWatcher {
    thread: WeakEntity<AcpThread>,
    _subscription: Subscription,
}
use collections::HashMap;
use db::kvp::KeyValueStore;
use fs::Fs;
use gpui::{
    Action, App, AppContext, AsyncWindowContext, Context, Entity, EventEmitter, FocusHandle,
    Focusable, IntoElement, Pixels, Render, Styled, Subscription, Task, WeakEntity, Window, px,
};
use project::{AgentId, Project};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use ui::{ContextMenu, DropdownMenu, Tooltip, prelude::*};
use ui_input::InputField;
use util::ResultExt as _;
use workspace::{
    Workspace,
    dock::{DockPosition, Panel, PanelEvent},
};

const DOCK_KEY: &str = "office-panel-position";

struct PostDraft {
    post: Post,
    name: Entity<InputField>,
    role: Entity<InputField>,
    model: Entity<InputField>,
    instructions: Entity<InputField>,
}

pub struct OfficePanel {
    workspace: WeakEntity<Workspace>,
    project: Entity<Project>,
    fs: Arc<dyn Fs>,
    accounts: Entity<AccountsView>,
    post_threads: PostThreads,
    activities: HashMap<String, ThreadActivity>,
    thread_watchers: HashMap<String, ThreadWatcher>,
    activity_refresh: Option<Task<()>>,
    agent_panel_subscription: Option<Subscription>,
    focus_handle: FocusHandle,
    position: DockPosition,
    path: Option<PathBuf>,
    file: Option<OfficeFile>,
    error: Option<String>,
    draft: Option<PostDraft>,
    show_archived: bool,
    loading: bool,
    saving: bool,
    _subscriptions: Vec<Subscription>,
    _load_task: Task<()>,
    _save_task: Task<()>,
}

impl OfficePanel {
    pub fn load(
        workspace: WeakEntity<Workspace>,
        cx: AsyncWindowContext,
    ) -> Task<anyhow::Result<Entity<Self>>> {
        cx.spawn(async move |cx| {
            let project = workspace.read_with(cx, |workspace, _| workspace.project().clone())?;
            let fs = project.read_with(cx, |project, _| project.fs().clone());
            Ok(cx.new(|cx| {
                let position = match KeyValueStore::global(cx)
                    .read_kvp(DOCK_KEY)
                    .log_err()
                    .flatten()
                    .as_deref()
                {
                    Some("left") => DockPosition::Left,
                    Some("bottom") => DockPosition::Bottom,
                    _ => DockPosition::Right,
                };
                let subscription = cx.subscribe(&project, |panel: &mut Self, _, event, cx| {
                    if matches!(
                        event,
                        project::Event::WorktreeAdded(_)
                            | project::Event::WorktreeRemoved(_)
                            | project::Event::WorktreeOrderChanged
                    ) {
                        panel.refresh(false, cx);
                    }
                });
                let post_threads = PostThreads::default();
                let accounts = cx.new(|cx| {
                    AccountsView::new(workspace.clone(), fs.clone(), post_threads.clone(), cx)
                });
                let accounts_subscription = cx.observe(&accounts, |_, _, cx| cx.notify());
                let mut panel = Self {
                    workspace,
                    project,
                    fs,
                    accounts,
                    post_threads,
                    activities: HashMap::default(),
                    thread_watchers: HashMap::default(),
                    activity_refresh: None,
                    agent_panel_subscription: None,
                    focus_handle: cx.focus_handle(),
                    position,
                    path: None,
                    file: None,
                    error: None,
                    draft: None,
                    show_archived: false,
                    loading: false,
                    saving: false,
                    _subscriptions: vec![subscription, accounts_subscription],
                    _load_task: Task::ready(()),
                    _save_task: Task::ready(()),
                };
                panel.refresh(true, cx);
                panel
            }))
        })
    }

    fn refresh(&mut self, force: bool, cx: &mut Context<Self>) {
        let project = self.project.read(cx);
        let path = if project.is_local() {
            project.visible_worktrees(cx).find_map(|worktree| {
                let worktree = worktree.read(cx);
                (!worktree.is_single_file())
                    .then(|| worktree.abs_path().join(".uacode/office.json"))
            })
        } else {
            None
        };
        if !force && path == self.path {
            return;
        }
        self.path = path.clone();
        self.file = None;
        self.error = None;
        self.draft = None;
        self.loading = path.is_some();
        self._load_task = Task::ready(());
        if let Some(path) = path {
            let fs = self.fs.clone();
            self._load_task = cx.spawn(async move |this, cx| {
                let result = office_store::load(fs.as_ref(), &path).await;
                this.update(cx, |panel, cx| {
                    panel.loading = false;
                    match result {
                        Ok(file) => panel.file = Some(file),
                        Err(error) => {
                            panel.error = Some(format!("No se pudo leer office.json: {error:#}"))
                        }
                    }
                    panel.load_post_threads(cx);
                    panel.watch_post_threads(cx);
                    cx.notify();
                })
                .log_err();
            });
        }
        cx.notify();
    }

    /// Each post's thread is saved per project, keyed by the folder holding `office.json`.
    fn load_post_threads(&self, cx: &App) {
        let project_root = self
            .path
            .as_deref()
            .and_then(Path::parent)
            .and_then(Path::parent);
        let post_ids = self
            .file
            .iter()
            .flat_map(|file| file.document.posts.iter().map(|post| post.id.clone()))
            .collect::<Vec<_>>();
        self.post_threads.load(project_root, post_ids, cx);
    }

    /// The post's saved thread, when it still exists and runs on the post's current agent.
    fn reusable_thread(
        &self,
        post: &Post,
        agent_id: &AgentId,
        agent_panel: &AgentPanel,
        cx: &App,
    ) -> Option<ThreadId> {
        let thread_id = self.post_threads.thread(&post.id)?;
        let saved_agent = ThreadMetadataStore::try_global(cx).and_then(|store| {
            store
                .read(cx)
                .entry(thread_id)
                .map(|metadata| metadata.agent_id.clone())
        });
        let reusable = match saved_agent {
            // A post whose account changed starts a new thread on its new agent.
            Some(saved_agent) => &saved_agent == agent_id,
            // A thread with no saved metadata yet is only still reachable while in memory.
            None => {
                agent_panel.is_retained_thread(&thread_id)
                    || agent_panel.active_thread_id(cx) == Some(thread_id)
            }
        };
        reusable.then_some(thread_id)
    }

    /// Opens the post's thread in the agent panel, creating it when the post has none yet or
    /// `new_thread` is set. A replaced thread stays in the agent panel's history.
    fn open_post_thread(
        &mut self,
        post: &Post,
        new_thread: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(agent_id) = self.accounts.read(cx).post_agent_id(post).cloned() else {
            self.error = Some(format!("«{}» no tiene una cuenta disponible", post.name));
            cx.notify();
            return;
        };
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        let Some(agent_panel) = workspace.read(cx).panel::<AgentPanel>(cx) else {
            self.error = Some("El panel de agentes no está disponible".into());
            cx.notify();
            return;
        };
        let existing = (!new_thread)
            .then(|| self.reusable_thread(post, &agent_id, agent_panel.read(cx), cx))
            .flatten();
        let agent = Agent::Custom { id: agent_id };
        let title = SharedString::from(post.name.clone());
        let created = agent_panel.update(cx, |agent_panel, cx| match existing {
            Some(thread_id) => {
                agent_panel.load_agent_thread(
                    agent,
                    thread_id,
                    None,
                    Some(title),
                    true,
                    AgentThreadSource::AgentPanel,
                    window,
                    cx,
                );
                None
            }
            None => {
                let thread_id = agent_panel.create_thread_with_options(
                    CreateThreadOptions {
                        title: Some(title),
                        agent: Some(agent),
                        ..Default::default()
                    },
                    AgentThreadSource::AgentPanel,
                    window,
                    cx,
                );
                agent_panel.activate_retained_thread(thread_id, true, window, cx);
                Some(thread_id)
            }
        });
        if let Some(thread_id) = created {
            // Approvals of the previous thread no longer belong on the post's card.
            self.accounts.update(cx, |accounts, cx| {
                accounts.forget_post_approvals(&post.id, cx)
            });
            let save = self.post_threads.set(&post.id, thread_id, cx);
            cx.spawn(async move |panel, cx| {
                if let Err(error) = save.await {
                    panel
                        .update(cx, |panel, cx| {
                            panel.error =
                                Some(format!("No se pudo recordar el hilo del puesto: {error:#}"));
                            cx.notify();
                        })
                        .log_err();
                }
            })
            .detach();
        }
        workspace.update(cx, |workspace, cx| {
            workspace.focus_panel::<AgentPanel>(window, cx);
        });
        self.watch_post_threads(cx);
        cx.notify();
    }

    fn agent_panel(&self, cx: &App) -> Option<Entity<AgentPanel>> {
        self.workspace.upgrade()?.read(cx).panel::<AgentPanel>(cx)
    }

    /// The agent panel loads after Office, so its events are followed once it exists.
    fn ensure_agent_panel_subscription(&mut self, cx: &mut Context<Self>) {
        if self.agent_panel_subscription.is_some() {
            return;
        }
        let Some(agent_panel) = self.agent_panel(cx) else {
            return;
        };
        self.agent_panel_subscription = Some(
            cx.subscribe(&agent_panel, |panel, _, _: &AgentPanelEvent, cx| {
                panel.watch_post_threads(cx)
            }),
        );
        self.watch_post_threads(cx);
    }

    /// Follows the open thread of each post so its card shows what the agent is doing.
    fn watch_post_threads(&mut self, cx: &mut Context<Self>) {
        let Some(agent_panel) = self.agent_panel(cx) else {
            return;
        };
        let post_ids = self
            .file
            .iter()
            .flat_map(|file| file.document.posts.iter().map(|post| post.id.clone()))
            .collect::<Vec<_>>();
        let threads = post_ids
            .into_iter()
            .filter_map(|post_id| {
                let thread_id = self.post_threads.thread(&post_id)?;
                let thread = agent_panel.read(cx).agent_thread(thread_id, cx)?;
                Some((post_id, thread))
            })
            .collect::<Vec<_>>();
        self.set_watched_threads(threads, cx);
    }

    fn set_watched_threads(
        &mut self,
        threads: Vec<(String, Entity<AcpThread>)>,
        cx: &mut Context<Self>,
    ) {
        let mut watchers = HashMap::default();
        for (post_id, thread) in threads {
            let watcher = match self.thread_watchers.remove(&post_id) {
                Some(watcher) if watcher.thread == thread.downgrade() => watcher,
                _ => ThreadWatcher {
                    thread: thread.downgrade(),
                    _subscription: cx.subscribe(&thread, |panel, _, _, cx| {
                        panel.schedule_activity_refresh(cx)
                    }),
                },
            };
            watchers.insert(post_id, watcher);
        }
        self.thread_watchers = watchers;
        self.refresh_activities(cx);
    }

    fn schedule_activity_refresh(&mut self, cx: &mut Context<Self>) {
        if self.activity_refresh.is_some() {
            return;
        }
        self.activity_refresh = Some(cx.spawn(async move |panel, cx| {
            cx.background_executor()
                .timer(ACTIVITY_REFRESH_INTERVAL)
                .await;
            panel
                .update(cx, |panel, cx| {
                    panel.activity_refresh = None;
                    panel.refresh_activities(cx);
                })
                .log_err();
        }));
    }

    fn refresh_activities(&mut self, cx: &mut Context<Self>) {
        let activities = self
            .thread_watchers
            .iter()
            .filter_map(|(post_id, watcher)| {
                let thread = watcher.thread.upgrade()?;
                Some((
                    post_id.clone(),
                    post_activity::thread_activity(thread.read(cx), cx),
                ))
            })
            .collect::<HashMap<_, _>>();
        if activities != self.activities {
            self.activities = activities;
            cx.notify();
        }
    }

    /// Stops the response the post's thread is producing, if any.
    fn cancel_post_thread(&mut self, post: &Post, cx: &mut Context<Self>) {
        let Some(thread_id) = self.post_threads.thread(&post.id) else {
            return;
        };
        let Some(agent_panel) = self
            .workspace
            .upgrade()
            .and_then(|workspace| workspace.read(cx).panel::<AgentPanel>(cx))
        else {
            return;
        };
        agent_panel.update(cx, |agent_panel, cx| {
            agent_panel.cancel_thread(&thread_id, cx);
        });
    }

    fn thread_label(&self, post: &Post, cx: &App) -> String {
        let Some(thread_id) = self.post_threads.thread(&post.id) else {
            return "Sin hilo".into();
        };
        ThreadMetadataStore::try_global(cx)
            .and_then(|store| {
                store
                    .read(cx)
                    .entry(thread_id)
                    .map(|metadata| format!("Hilo: {}", metadata.display_title()))
            })
            .unwrap_or_else(|| "Hilo nuevo".into())
    }

    fn writable(&self) -> bool {
        !self.loading && !self.saving && self.file.as_ref().is_some_and(|file| !file.is_read_only())
    }

    fn edit_post(&mut self, post: Option<Post>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.writable() {
            return;
        }
        let post = post.unwrap_or_else(|| Post {
            id: uuid::Uuid::new_v4().to_string(),
            name: String::new(),
            role: "backend".into(),
            provider: Provider::Claude,
            account: None,
            model: String::new(),
            instructions: String::new(),
            archived: false,
        });
        let mut field = |label: &str, text: &str| {
            let input = cx.new(|cx| InputField::new(window, cx, label).label(label.to_owned()));
            let editor = input.read(cx).editor().clone();
            editor.set_text(text, window, cx);
            input
        };
        let name = field("Nombre", &post.name);
        let role = field("Rol", &post.role);
        let model = field("Modelo (Fase 2)", &post.model);
        let instructions = field("Instrucciones (Fase 2)", &post.instructions);
        let instructions_editor = instructions.read(cx).editor().clone();
        instructions_editor.set_multiline(Some(5), window, cx);
        name.focus_handle(cx).focus(window, cx);
        self.draft = Some(PostDraft {
            post,
            name,
            role,
            model,
            instructions,
        });
        cx.notify();
    }

    fn save_draft(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = &self.draft else {
            return;
        };
        let mut post = draft.post.clone();
        post.name = draft.name.read(cx).text(cx).trim().to_owned();
        post.role = draft.role.read(cx).text(cx).trim().to_owned();
        post.model = draft.model.read(cx).text(cx).trim().to_owned();
        post.instructions = draft.instructions.read(cx).text(cx);
        if post.name.is_empty() || post.role.is_empty() {
            self.error = Some("El nombre y el rol son obligatorios".into());
            cx.notify();
            return;
        }
        let Some(file) = &self.file else {
            return;
        };
        let mut document = file.document.clone();
        if let Some(existing) = document
            .posts
            .iter_mut()
            .find(|existing| existing.id == post.id)
        {
            *existing = post;
        } else {
            document.posts.push(post);
        }
        self.persist(document, cx);
    }

    fn persist(&mut self, document: office_store::OfficeDocument, cx: &mut Context<Self>) {
        if !self.writable() {
            return;
        }
        let (Some(path), Some(previous)) = (self.path.clone(), self.file.clone()) else {
            return;
        };
        self.saving = true;
        self.error = None;
        let fs = self.fs.clone();
        self._save_task = cx.spawn(async move |this, cx| {
            let result = office_store::save(fs.as_ref(), &path, &previous, document).await;
            this.update(cx, |panel, cx| {
                panel.saving = false;
                if panel.path.as_ref() == Some(&path) {
                    match result {
                        Ok(file) => {
                            panel.file = Some(file);
                            panel.draft = None;
                            panel.load_post_threads(cx);
                            panel.watch_post_threads(cx);
                        }
                        Err(error) => panel.error = Some(format!("No se pudo guardar: {error:#}")),
                    }
                } else if let Err(error) = result {
                    panel.error = Some(format!("No se pudo guardar {}: {error:#}", path.display()));
                }
                cx.notify();
            })
            .log_err();
        });
        cx.notify();
    }

    fn archive(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(file) = &self.file else {
            return;
        };
        let mut document = file.document.clone();
        if let Some(post) = document.posts.iter_mut().find(|post| post.id == id) {
            post.archived = !post.archived;
            self.persist(document, cx);
        }
    }

    /// Requests waiting for a person and the latest denial on the post's account.
    fn render_policy_state(&self, post: &Post, cx: &mut Context<Self>) -> AnyElement {
        let accounts = self.accounts.read(cx);
        let approvals = accounts.post_pending_approvals(&post.id);
        let notice = accounts.post_policy_notice(&post.id);
        let mut state = v_flex().gap_1();
        if !approvals.is_empty() {
            state = state.child(
                Label::new(format!("Esperando aprobación ({})", approvals.len()))
                    .color(Color::Warning),
            );
        }
        for (index, (request, title)) in approvals.into_iter().enumerate() {
            let element_id =
                |action: &str| -> SharedString { format!("{action}-{}-{index}", post.id).into() };
            // Titles and reasons can be long paths: they get their own truncated line so
            // the buttons always stay inside the card.
            state = state
                .child(Label::new(title).size(LabelSize::Small).truncate())
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            div()
                                .debug_selector(|| element_id("approve").to_string())
                                .child(Button::new(element_id("approve"), "Aprobar").on_click(
                                    cx.listener(move |panel, _, _, cx| {
                                        panel.accounts.update(cx, |accounts, cx| {
                                            accounts.answer_approval(request, true, cx)
                                        })
                                    }),
                                )),
                        )
                        .child(
                            div()
                                .debug_selector(|| element_id("reject").to_string())
                                .child(Button::new(element_id("reject"), "Rechazar").on_click(
                                    cx.listener(move |panel, _, _, cx| {
                                        panel.accounts.update(cx, |accounts, cx| {
                                            accounts.answer_approval(request, false, cx)
                                        })
                                    }),
                                )),
                        ),
                );
        }
        if let Some(notice) = notice {
            let dismissed = post.id.clone();
            state = state
                .child(
                    Label::new(format!("Bloqueado: {notice}"))
                        .size(LabelSize::Small)
                        .color(Color::Error)
                        .truncate(),
                )
                .child(
                    Button::new(
                        SharedString::from(format!("dismiss-notice-{}", post.id)),
                        "Descartar",
                    )
                    .on_click(cx.listener(move |panel, _, _, cx| {
                        panel.accounts.update(cx, |accounts, cx| {
                            accounts.dismiss_policy_notice(&dismissed, cx)
                        })
                    })),
                );
        }
        state.into_any_element()
    }

    /// The post's state, current task and the agent's last message.
    fn render_activity(&self, post: &Post, cx: &App) -> AnyElement {
        let accounts = self.accounts.read(cx);
        let (requires_login, account_error) = accounts.post_account_problem(post);
        let activity = self.activities.get(&post.id);
        let waiting_approval = !accounts.post_pending_approvals(&post.id).is_empty()
            || activity.is_some_and(|activity| activity.waiting_approval);
        let state = post_activity::post_state(
            requires_login,
            account_error.as_deref(),
            waiting_approval,
            activity,
        );
        v_flex()
            .gap_1()
            .child(Label::new(state.label()).color(state.color()).truncate())
            .children(
                activity
                    .and_then(|activity| activity.task.clone())
                    .map(|task| {
                        Label::new(format!("Tarea: {task}"))
                            .size(LabelSize::Small)
                            .truncate()
                    }),
            )
            .children(
                activity
                    .and_then(|activity| activity.last_message.clone())
                    .map(|message| {
                        Label::new(format!("Último mensaje: {message}"))
                            .size(LabelSize::Small)
                            .color(Color::Muted)
                            .truncate()
                    }),
            )
            .into_any_element()
    }

    fn account_label(&self, post: &Post, cx: &App) -> String {
        match self.accounts.read(cx).document() {
            Some(document) => document.resolve(post).label(),
            None if post.account.is_some() => "sin cargar".into(),
            None => AccountChoice::Default.label(),
        }
    }

    fn render_account_picker(
        &self,
        post: &Post,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> DropdownMenu {
        let label = format!("Cuenta: {}", self.account_label(post, cx));
        let mut options = vec![(None, AccountChoice::Default.label())];
        if let Some(document) = self.accounts.read(cx).document() {
            options.extend(
                document
                    .accounts_for(post.provider)
                    .map(|account| (Some(account.id.clone()), account.name.clone())),
            );
        }
        let panel = cx.weak_entity();
        DropdownMenu::new(
            "post-account",
            label,
            ContextMenu::build(window, cx, move |mut menu, _, _| {
                for (account, name) in options {
                    let panel = panel.clone();
                    menu = menu.entry(name, None, move |_, cx| {
                        panel
                            .update(cx, |panel, cx| {
                                if let Some(draft) = &mut panel.draft {
                                    draft.post.account = account.clone();
                                }
                                cx.notify();
                            })
                            .log_err();
                    });
                }
                menu
            }),
        )
        .disabled(self.saving)
    }

    fn open_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.path.clone() else {
            return;
        };
        match self.workspace.update(cx, |workspace, cx| {
            workspace.open_abs_path(path, Default::default(), window, cx)
        }) {
            Ok(task) => {
                cx.spawn(async move |this, cx| {
                    if let Err(error) = task.await {
                        this.update(cx, |panel, cx| {
                            panel.error = Some(format!("No se pudo abrir office.json: {error:#}"));
                            cx.notify();
                        })
                        .log_err();
                    }
                })
                .detach();
            }
            Err(error) => {
                self.error = Some(format!("No se pudo abrir office.json: {error:#}"));
                cx.notify();
            }
        }
    }
}

impl EventEmitter<PanelEvent> for OfficePanel {}
impl Focusable for OfficePanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
impl Panel for OfficePanel {
    fn persistent_name() -> &'static str {
        "Office"
    }
    fn panel_key() -> &'static str {
        "OfficePanel"
    }
    fn position(&self, _window: &Window, _cx: &App) -> DockPosition {
        self.position
    }
    fn position_is_valid(&self, _position: DockPosition) -> bool {
        true
    }
    fn set_position(
        &mut self,
        position: DockPosition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.position == position {
            return;
        }
        self.position = position;
        let value = match position {
            DockPosition::Left => "left",
            DockPosition::Right => "right",
            DockPosition::Bottom => "bottom",
        };
        let database = KeyValueStore::global(cx);
        cx.spawn(async move |this, cx| {
            if let Err(error) = database.write_kvp(DOCK_KEY.into(), value.into()).await {
                this.update(cx, |panel, cx| {
                    panel.error = Some(format!("No se pudo guardar la posición: {error:#}"));
                    cx.notify();
                })
                .log_err();
            }
        })
        .detach();
        let workspace = self.workspace.clone();
        let panel = cx.entity();
        // Dock relocation otherwise depends on a global settings-file change.
        window.defer(cx, move |window, cx| {
            if let Err(error) = workspace.update(cx, |workspace, cx| {
                workspace.remove_panel::<Self>(&panel, window, cx);
                workspace.add_panel(panel, window, cx);
                workspace.focus_panel::<Self>(window, cx);
            }) {
                log::error!("Failed to move Office panel: {error}");
            }
        });
        cx.notify();
    }
    fn default_size(&self, _window: &Window, _cx: &App) -> Pixels {
        px(360.)
    }
    fn icon(&self, _window: &Window, _cx: &App) -> Option<IconName> {
        Some(IconName::Person)
    }
    fn icon_tooltip(&self, _window: &Window, _cx: &App) -> Option<&'static str> {
        Some("Office")
    }
    fn toggle_action(&self) -> Box<dyn Action> {
        Box::new(crate::ToggleFocus)
    }
    fn activation_priority(&self) -> u32 {
        8
    }
}

impl Render for OfficePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_agent_panel_subscription(cx);
        let writable = self.writable();
        let warning = self.file.as_ref().and_then(|file| file.warning.clone());
        let posts = self
            .file
            .as_ref()
            .map(|file| file.document.posts.clone())
            .unwrap_or_default();
        let mut content = v_flex().gap_3().child(Label::new("Office"));
        if self.path.is_none() {
            let message = if self.project.read(cx).is_local() {
                "Abre una carpeta para usar Office"
            } else {
                "Office solo admite proyectos locales por ahora"
            };
            return v_flex()
                .id("office-panel")
                .track_focus(&self.focus_handle)
                .size_full()
                .p_4()
                .bg(cx.theme().colors().panel_background)
                .child(content.child(Label::new(message)))
                .into_any_element();
        }
        content = content.child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("new-post", "Nuevo puesto")
                        .disabled(!writable || self.draft.is_some())
                        .on_click(
                            cx.listener(|panel, _, window, cx| panel.edit_post(None, window, cx)),
                        ),
                )
                .child(
                    Button::new("reload-office", "Recargar")
                        .disabled(self.saving || self.loading)
                        .on_click(cx.listener(|panel, _, _, cx| panel.refresh(true, cx))),
                ),
        );
        if self.loading {
            content = content.child(Label::new("Cargando…"));
        }
        if self.saving {
            content = content.child(Label::new("Guardando…"));
        }
        if let Some(message) = self.error.clone().or(warning) {
            let file_exists = self.file.as_ref().is_some_and(|file| file.exists_on_disk());
            content =
                content.child(Label::new(message).color(Color::Error)).when(
                    file_exists,
                    |content| {
                        content.child(Button::new("open-office-file", "Abrir archivo").on_click(
                            cx.listener(|panel, _, window, cx| panel.open_file(window, cx)),
                        ))
                    },
                );
        }
        if let Some(draft) = &self.draft {
            content = content.child(
                v_flex()
                    .gap_2()
                    .child(draft.name.clone())
                    .child(draft.role.clone())
                    .child(
                        Button::new(
                            "post-provider",
                            format!("Proveedor: {}", draft.post.provider.label()),
                        )
                        .disabled(self.saving)
                        .on_click(cx.listener(|panel, _, _, cx| {
                            if let Some(draft) = &mut panel.draft {
                                draft.post.set_provider(match draft.post.provider {
                                    Provider::Claude => Provider::Codex,
                                    Provider::Codex => Provider::Claude,
                                });
                            }
                            cx.notify();
                        })),
                    )
                    .child(self.render_account_picker(&draft.post, window, cx))
                    .child(draft.model.clone())
                    .child(draft.instructions.clone())
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("save-post", "Guardar")
                                    .disabled(!writable)
                                    .on_click(cx.listener(|panel, _, _, cx| panel.save_draft(cx))),
                            )
                            .child(
                                Button::new("cancel-post", "Cancelar")
                                    .disabled(self.saving)
                                    .on_click(cx.listener(|panel, _, _, cx| {
                                        panel.draft = None;
                                        panel.error = None;
                                        cx.notify();
                                    })),
                            ),
                    ),
            );
        }
        content = content.child(
            Button::new(
                "show-archived",
                if self.show_archived {
                    "Ocultar archivados"
                } else {
                    "Mostrar archivados"
                },
            )
            .on_click(cx.listener(|panel, _, _, cx| {
                panel.show_archived = !panel.show_archived;
                cx.notify();
            })),
        );
        if self.file.as_ref().is_some_and(|file| !file.is_read_only()) {
            if posts.is_empty() {
                content = content.child(Label::new("Todavía no hay puestos"));
            } else if !self.show_archived && posts.iter().all(|post| post.archived) {
                content = content.child(Label::new("Todos los puestos están archivados"));
            }
        }
        for post in posts
            .into_iter()
            .filter(|post| self.show_archived || !post.archived)
        {
            let id = post.id.clone();
            let edit = post.clone();
            let opened = post.clone();
            let renewed = post.clone();
            let cancelled = post.clone();
            let account = self.account_label(&post, cx);
            let thread = self.thread_label(&post, cx);
            let has_thread = self.post_threads.thread(&post.id).is_some();
            content = content.child(
                v_flex()
                    .gap_1()
                    .p_2()
                    .border_1()
                    .border_color(cx.theme().colors().border)
                    // Only the summary opens the thread: buttons nested in a clickable
                    // element never receive their own clicks.
                    .child(
                        v_flex()
                            .id(SharedString::from(format!("post-{}", post.id)))
                            .gap_1()
                            .when(!post.archived, |summary| {
                                summary
                                    .cursor_pointer()
                                    .hover(|style| style.bg(cx.theme().colors().element_hover))
                                    .tooltip(Tooltip::text("Abrir el hilo del puesto"))
                                    .on_click(cx.listener(move |panel, _, window, cx| {
                                        panel.open_post_thread(&opened, false, window, cx)
                                    }))
                            })
                            .child(Label::new(post.name.clone()))
                            .child(Label::new(format!(
                                "{} · {}",
                                post.role,
                                post.provider.label()
                            )))
                            .child(Label::new(if post.archived {
                                "Archivado".to_string()
                            } else {
                                match self.accounts.read(cx).post_status_label(&post) {
                                    Some(status) => {
                                        format!("{thread} · Cuenta: {account} · {status}")
                                    }
                                    None => format!("{thread} · Cuenta: {account}"),
                                }
                            })),
                    )
                    .children((!post.archived).then(|| self.render_activity(&post, cx)))
                    .children((!post.archived).then(|| self.render_policy_state(&post, cx)))
                    .child(Label::new(if post.model.is_empty() {
                        "Modelo: por defecto".into()
                    } else {
                        format!("Modelo: {} (Fase 2)", post.model)
                    }))
                    .child(
                        h_flex()
                            .gap_2()
                            .when(!post.archived, |buttons| {
                                buttons
                                    .child(
                                        Button::new(
                                            SharedString::from(format!("new-thread-{}", post.id)),
                                            "Nuevo hilo",
                                        )
                                        .on_click(
                                            cx.listener(move |panel, _, window, cx| {
                                                panel.open_post_thread(&renewed, true, window, cx)
                                            }),
                                        ),
                                    )
                                    .child(
                                        Button::new(
                                            SharedString::from(format!("cancel-{}", post.id)),
                                            "Cancelar",
                                        )
                                        .disabled(!has_thread)
                                        .on_click(
                                            cx.listener(move |panel, _, _, cx| {
                                                panel.cancel_post_thread(&cancelled, cx)
                                            }),
                                        ),
                                    )
                            })
                            .child(
                                Button::new(
                                    SharedString::from(format!("edit-post-{}", post.id)),
                                    "Editar",
                                )
                                .disabled(!writable || self.draft.is_some())
                                .on_click(cx.listener(
                                    move |panel, _, window, cx| {
                                        panel.edit_post(Some(edit.clone()), window, cx)
                                    },
                                )),
                            )
                            .child(
                                Button::new(
                                    SharedString::from(format!("archive-post-{}", post.id)),
                                    if post.archived {
                                        "Restaurar"
                                    } else {
                                        "Archivar"
                                    },
                                )
                                .disabled(!writable || self.draft.is_some())
                                .on_click(
                                    cx.listener(move |panel, _, _, cx| panel.archive(&id, cx)),
                                ),
                            ),
                    ),
            );
        }
        v_flex()
            .id("office-panel")
            .track_focus(&self.focus_handle)
            .size_full()
            .p_4()
            .bg(cx.theme().colors().panel_background)
            .overflow_y_scroll()
            .child(content)
            .child(
                div()
                    .mt_4()
                    .pt_4()
                    .border_t_1()
                    .border_color(cx.theme().colors().border)
                    .child(self.accounts.clone()),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acp_thread::{
        AcpThread, AcpThreadEvent, AgentConnection as _, AuthorizationKind, PermissionOptions,
        RequestPermissionOutcome, StubAgentConnection,
    };
    use agent_client_protocol::{v1::schema as acp_v1, v2::schema as acp_v2};
    use agent_ui::thread_metadata_store::ThreadMetadata;
    use gpui::{Modifiers, TestAppContext, VisualTestContext};
    use project::FakeFs;
    use settings::SettingsStore;
    use std::{cell::RefCell, rc::Rc};
    use util::path_list::PathList;

    const POST_ID: &str = "post-1";

    fn init(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            ThreadMetadataStore::init_global(cx);
        });
    }

    struct Fixture {
        panel: Entity<OfficePanel>,
        thread: Entity<AcpThread>,
        cx: VisualTestContext,
    }

    /// An Office panel with one Claude post whose thread is linked, as after opening it from
    /// the card and sending a first message.
    async fn panel_with_linked_thread(cx: &mut TestAppContext) -> Fixture {
        init(cx);
        let fs = FakeFs::new(cx.executor());
        let office = serde_json::json!({
            "version": 1,
            "posts": [{
                "id": POST_ID,
                "name": "Consulta",
                "role": "backend",
                "provider": "claude",
            }],
        });
        fs.insert_tree(
            "/project",
            serde_json::json!({ ".uacode": { "office.json": office.to_string() } }),
        )
        .await;
        let project = Project::test(fs, [Path::new("/project")], cx).await;
        let window = cx.add_window(|window, cx| Workspace::test_new(project.clone(), window, cx));
        let workspace = window.root(cx).expect("workspace");
        let mut cx = VisualTestContext::from_window(*window, cx);
        let panel = cx
            .update(|window, cx| OfficePanel::load(workspace.downgrade(), window.to_async(cx)))
            .await
            .expect("office panel");
        workspace.update_in(&mut cx, |workspace, window, cx| {
            workspace.add_panel(panel.clone(), window, cx);
            workspace.open_panel::<OfficePanel>(window, cx);
        });
        cx.run_until_parked();

        let connection = Rc::new(
            StubAgentConnection::new()
                .with_agent_id(AgentId::new("office-claude-default"))
                .with_agent_version(crate::office_policy::CLAUDE_ADAPTER.version),
        );
        let thread = cx
            .update(|_, cx| {
                connection.clone().new_session(
                    project.clone(),
                    PathList::new(&[Path::new("/project")]),
                    cx,
                )
            })
            .await
            .expect("session");
        let thread_id = ThreadId::new();
        let session_id = thread.read_with(&cx, |thread, _| thread.session_id().clone());
        cx.update(|_, cx| {
            ThreadMetadataStore::global(cx).update(cx, |store, cx| {
                store.save(
                    ThreadMetadata {
                        thread_id,
                        session_id: Some(session_id),
                        agent_id: AgentId::new("office-claude-default"),
                        title: Some("Consulta".into()),
                        title_override: None,
                        updated_at: Default::default(),
                        created_at: None,
                        interacted_at: None,
                        worktree_paths: Default::default(),
                        remote_connection: None,
                        archived: false,
                    },
                    cx,
                )
            });
        });
        panel.update_in(&mut cx, |panel, _, cx| {
            panel.post_threads.set(POST_ID, thread_id, cx).detach();
        });
        cx.run_until_parked();
        Fixture { panel, thread, cx }
    }

    fn request_read_permission(
        thread: &Entity<AcpThread>,
        tool_call: &str,
        cx: &mut VisualTestContext,
    ) -> Task<RequestPermissionOutcome> {
        thread.update(cx, |thread, cx| {
            thread
                .request_tool_call_authorization(
                    acp_v1::ToolCall::new(acp_v1::ToolCallId::new(tool_call), "Read fuera.txt")
                        .kind(acp_v1::ToolKind::Read)
                        .into(),
                    PermissionOptions::Flat(vec![
                        acp_v2::PermissionOption::new(
                            "allow",
                            "Yes",
                            acp_v2::PermissionOptionKind::AllowOnce,
                        ),
                        acp_v2::PermissionOption::new(
                            "allow_always",
                            "Yes, during this session",
                            acp_v2::PermissionOptionKind::AllowAlways,
                        ),
                        acp_v2::PermissionOption::new(
                            "reject",
                            "No",
                            acp_v2::PermissionOptionKind::RejectOnce,
                        ),
                    ]),
                    AuthorizationKind::PermissionGrant,
                    cx,
                )
                .expect("permission request")
        })
    }

    #[gpui::test]
    async fn approving_and_rejecting_from_the_card_answer_the_thread(cx: &mut TestAppContext) {
        let Fixture {
            panel,
            thread,
            mut cx,
        } = panel_with_linked_thread(cx).await;
        let received = Rc::new(RefCell::new(0));
        let _subscription = cx.update(|_, cx| {
            cx.subscribe(&thread, {
                let received = received.clone();
                move |_, event, _| {
                    if matches!(event, AcpThreadEvent::ToolAuthorizationReceived(_)) {
                        *received.borrow_mut() += 1;
                    }
                }
            })
        });

        for (tool_call, button, expected) in [
            ("read-1", "approve", "allow"),
            ("read-2", "reject", "reject"),
        ] {
            let outcome = request_read_permission(&thread, tool_call, &mut cx);
            cx.run_until_parked();
            panel.read_with(&cx, |panel, cx| {
                assert_eq!(
                    panel
                        .accounts
                        .read(cx)
                        .post_pending_approvals(POST_ID)
                        .len(),
                    1,
                    "the card shows the pending request"
                );
            });
            let selector: &'static str = match button {
                "approve" => "approve-post-1-0",
                _ => "reject-post-1-0",
            };
            let bounds = cx
                .debug_bounds(selector)
                .expect("the card renders the button");
            cx.simulate_click(bounds.center(), Modifiers::default());
            cx.run_until_parked();

            match outcome.await {
                RequestPermissionOutcome::Selected(selected) => {
                    assert_eq!(selected.option_id.0.as_ref(), expected)
                }
                other => panic!("{button}: unexpected outcome {other:?}"),
            }
            panel.read_with(&cx, |panel, cx| {
                assert!(
                    panel
                        .accounts
                        .read(cx)
                        .post_pending_approvals(POST_ID)
                        .is_empty(),
                    "{button}: the card is cleared"
                );
            });
        }
        assert_eq!(*received.borrow(), 2);
    }

    fn pending_on_card(panel: &Entity<OfficePanel>, cx: &VisualTestContext) -> usize {
        panel.read_with(cx, |panel, cx| {
            panel
                .accounts
                .read(cx)
                .post_pending_approvals(POST_ID)
                .len()
        })
    }

    #[gpui::test]
    async fn a_request_cancelled_in_the_thread_leaves_the_card(cx: &mut TestAppContext) {
        let Fixture {
            panel,
            thread,
            mut cx,
        } = panel_with_linked_thread(cx).await;
        let _outcome = request_read_permission(&thread, "read-1", &mut cx);
        cx.run_until_parked();
        assert_eq!(pending_on_card(&panel, &cx), 1);

        thread.update(&mut cx, |thread, cx| {
            thread.cancel_tool_call_authorization(&acp_v2::ToolCallId::new("read-1"), cx)
        });
        cx.run_until_parked();
        assert_eq!(pending_on_card(&panel, &cx), 0);
    }

    #[gpui::test]
    async fn cards_show_state_and_last_message_with_limited_refresh(cx: &mut TestAppContext) {
        let Fixture {
            panel,
            thread,
            mut cx,
        } = panel_with_linked_thread(cx).await;
        panel.update_in(&mut cx, |panel, _, cx| {
            panel.set_watched_threads(vec![(POST_ID.to_string(), thread.clone())], cx)
        });
        let activity = |cx: &VisualTestContext| {
            panel.read_with(cx, |panel, _| panel.activities.get(POST_ID).cloned())
        };
        assert_eq!(activity(&cx).map(|activity| activity.working), Some(false));

        thread.update(&mut cx, |thread, cx| {
            thread
                .handle_session_update(
                    acp_v1::SessionUpdate::AgentMessageChunk(acp_v1::ContentChunk::new(
                        "Primera línea
He revisado notas.txt"
                            .into(),
                    )),
                    cx,
                )
                .expect("message chunk")
        });
        cx.run_until_parked();
        assert_eq!(
            activity(&cx).and_then(|activity| activity.last_message),
            None,
            "streamed text is not shown on every chunk"
        );
        cx.executor().advance_clock(ACTIVITY_REFRESH_INTERVAL);
        cx.run_until_parked();
        assert_eq!(
            activity(&cx)
                .and_then(|activity| activity.last_message)
                .as_deref(),
            Some("He revisado notas.txt")
        );

        let _outcome = request_read_permission(&thread, "read-1", &mut cx);
        cx.executor().advance_clock(ACTIVITY_REFRESH_INTERVAL);
        cx.run_until_parked();
        panel.read_with(&cx, |panel, cx| {
            let accounts = panel.accounts.read(cx);
            let (requires_login, account_error) = accounts.post_account_problem(&office_post());
            let activity = panel.activities.get(POST_ID);
            assert_eq!(
                post_activity::post_state(
                    requires_login,
                    account_error.as_deref(),
                    !accounts.post_pending_approvals(POST_ID).is_empty(),
                    activity,
                ),
                post_activity::PostState::WaitingApproval
            );
            assert!(activity.is_some_and(|activity| activity.waiting_approval));
        });
    }

    fn office_post() -> Post {
        Post {
            id: POST_ID.into(),
            name: "Consulta".into(),
            role: "backend".into(),
            provider: Provider::Claude,
            account: None,
            model: String::new(),
            instructions: String::new(),
            archived: false,
        }
    }

    #[gpui::test]
    async fn an_unanswerable_approval_is_removed_from_the_card(cx: &mut TestAppContext) {
        let Fixture {
            panel,
            thread,
            mut cx,
        } = panel_with_linked_thread(cx).await;
        let _outcome = request_read_permission(&thread, "read-1", &mut cx);
        cx.run_until_parked();
        assert_eq!(pending_on_card(&panel, &cx), 1);

        // The thread goes away without answering, as when its view is closed.
        drop(thread);
        cx.run_until_parked();
        let bounds = cx
            .debug_bounds("approve-post-1-0")
            .expect("the card still offers the approval");
        cx.simulate_click(bounds.center(), Modifiers::default());
        cx.run_until_parked();
        assert_eq!(pending_on_card(&panel, &cx), 0);
    }
}
