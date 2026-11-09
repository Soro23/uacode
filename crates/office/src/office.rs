mod account_agents;
mod account_login;
mod accounts;
mod accounts_view;
mod auth_status;
mod office_panel;
mod office_policy;
mod office_store;
mod post_threads;
mod versioned_file;

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
