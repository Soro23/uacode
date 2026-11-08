use gpui::{
    Action, App, AppContext, AsyncWindowContext, Context, Entity, EventEmitter, FocusHandle,
    Focusable, IntoElement, Pixels, Render, Styled, Task, WeakEntity, Window, px,
};
use ui::prelude::*;
use workspace::{
    Workspace,
    dock::{DockPosition, Panel, PanelEvent},
};

pub struct OfficePanel {
    workspace: WeakEntity<Workspace>,
    focus_handle: FocusHandle,
    position: DockPosition,
}

impl OfficePanel {
    pub fn load(
        workspace: WeakEntity<Workspace>,
        cx: AsyncWindowContext,
    ) -> Task<anyhow::Result<Entity<Self>>> {
        cx.spawn(async move |cx| {
            Ok(cx.new(|cx| Self {
                workspace,
                focus_handle: cx.focus_handle(),
                position: DockPosition::Right,
            }))
        })
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
        px(320.)
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
        v_flex()
            .id("office-panel")
            .track_focus(&self.focus_handle)
            .size_full()
            .p_4()
            .bg(cx.theme().colors().panel_background)
            .child(Label::new("Office"))
    }
}
