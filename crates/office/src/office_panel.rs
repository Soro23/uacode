use crate::office_store::{self, OfficeFile, Post, Provider};
use db::kvp::KeyValueStore;
use fs::Fs;
use gpui::{
    Action, App, AppContext, AsyncWindowContext, Context, Entity, EventEmitter, FocusHandle,
    Focusable, IntoElement, Pixels, Render, Styled, Subscription, Task, WeakEntity, Window, px,
};
use project::Project;
use std::{path::PathBuf, sync::Arc};
use ui::prelude::*;
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
                let mut panel = Self {
                    workspace,
                    project,
                    fs,
                    focus_handle: cx.focus_handle(),
                    position,
                    path: None,
                    file: None,
                    error: None,
                    draft: None,
                    show_archived: false,
                    loading: false,
                    saving: false,
                    _subscriptions: vec![subscription],
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
                    cx.notify();
                })
                .log_err();
            });
        }
        cx.notify();
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                                draft.post.provider = match draft.post.provider {
                                    Provider::Claude => Provider::Codex,
                                    Provider::Codex => Provider::Claude,
                                };
                            }
                            cx.notify();
                        })),
                    )
                    .child(Label::new("Cuenta: sin asignar"))
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
            content = content.child(
                v_flex()
                    .gap_1()
                    .p_2()
                    .border_1()
                    .border_color(cx.theme().colors().border)
                    .child(Label::new(post.name.clone()))
                    .child(Label::new(format!(
                        "{} · {}",
                        post.role,
                        post.provider.label()
                    )))
                    .child(Label::new(if post.archived {
                        "Archivado"
                    } else {
                        "Sin hilo · cuenta sin asignar"
                    }))
                    .child(Label::new(if post.model.is_empty() {
                        "Modelo: por defecto".into()
                    } else {
                        format!("Modelo: {} (Fase 2)", post.model)
                    }))
                    .child(
                        h_flex()
                            .gap_2()
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
            .into_any_element()
    }
}
