use crate::{
    accounts::{self, AccountsDocument, AccountsFile, ImportReport, default_folder},
    office_store::Provider,
    versioned_file,
};
use anyhow::Result;
use fs::Fs;
use futures::StreamExt as _;
use gpui::{Context, Entity, Focusable as _, IntoElement, Render, Task, WeakEntity, Window};
use std::{path::PathBuf, sync::Arc};
use time::OffsetDateTime;
use ui::prelude::*;
use ui_input::InputField;
use util::ResultExt as _;
use workspace::Workspace;

struct AccountDraft {
    provider: Provider,
    name: Entity<InputField>,
}

/// Global account catalog editor shown inside the Office panel.
pub struct AccountsView {
    workspace: WeakEntity<Workspace>,
    fs: Arc<dyn Fs>,
    path: PathBuf,
    credentials_root: PathBuf,
    file: Option<AccountsFile>,
    error: Option<String>,
    notice: Option<String>,
    draft: Option<AccountDraft>,
    loading: bool,
    saving: bool,
    _load_task: Task<()>,
    _save_task: Task<()>,
}

impl AccountsView {
    pub fn new(workspace: WeakEntity<Workspace>, fs: Arc<dyn Fs>, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            workspace,
            fs,
            path: accounts::accounts_path(),
            credentials_root: accounts::credentials_root(),
            file: None,
            error: None,
            notice: None,
            draft: None,
            loading: false,
            saving: false,
            _load_task: Task::ready(()),
            _save_task: Task::ready(()),
        };
        view.reload(cx);
        view
    }

    pub fn document(&self) -> Option<&AccountsDocument> {
        self.file.as_ref().map(|file| &file.document)
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
                    Ok(file) => view.file = Some(file),
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

    fn render_provider(&self, provider: Provider, cx: &App) -> impl IntoElement {
        let border = cx.theme().colors().border;
        let mut column = v_flex()
            .gap_1()
            .child(Label::new(provider.label()).color(Color::Muted))
            .child(account_row(
                "Por defecto".into(),
                default_folder(provider),
                border,
            ));
        if let Some(document) = self.document() {
            for account in document.accounts_for(provider) {
                column = column.child(account_row(
                    account.name.clone(),
                    account.folder_name(),
                    border,
                ));
            }
        }
        column
    }
}

fn account_row(name: String, folder: String, border: gpui::Hsla) -> impl IntoElement {
    v_flex()
        .p_2()
        .border_1()
        .border_color(border)
        .child(Label::new(name))
        .child(
            Label::new(format!("Carpeta: {folder} · Sin probar"))
                .size(LabelSize::Small)
                .color(Color::Muted),
        )
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
        if let Some(draft) = &self.draft {
            content = content.child(self.render_draft(draft, cx));
        }
        for provider in Provider::ALL {
            content = content.child(self.render_provider(provider, cx));
        }
        content
    }
}
