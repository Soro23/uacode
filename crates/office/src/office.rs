mod office_panel;

pub use office_panel::OfficePanel;

use gpui::{App, actions};
use workspace::Workspace;

actions!(
    office,
    [
        /// Opens or focuses the Office panel.
        ToggleFocus,
    ]
);

pub fn init(cx: &mut App) {
    cx.observe_new(|workspace: &mut Workspace, _, _| {
        workspace.register_action(|workspace, _: &ToggleFocus, window, cx| {
            workspace.toggle_panel_focus::<OfficePanel>(window, cx);
        });
    })
    .detach();
}
