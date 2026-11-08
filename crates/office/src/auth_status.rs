//! Account status probe.
//!
//! Interim extension: `_auth/status_update` is not part of ACP yet. Both pinned adapters
//! (claude-agent-acp 0.94.0, codex-acp 2.5.0) push it right after `initialize`. Everything
//! that depends on its shape lives in `parse_status_notification`, so it can be replaced in
//! one place once the protocol defines an official equivalent.

use anyhow::{Context as _, Result, anyhow, bail};
use futures::{
    AsyncBufReadExt as _, AsyncWriteExt as _, FutureExt as _, StreamExt as _, io::BufReader,
};
use gpui::BackgroundExecutor;
use project::agent_server_store::AgentServerCommand;
use serde::Deserialize;
use std::{
    process::Stdio,
    time::{Duration, Instant},
};
use util::{ResultExt as _, process::Child, shell::Shell, shell_builder::ShellBuilder};

/// The first run downloads the adapter through npx, which can take a while.
pub const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(120);
pub const STATUS_TIMEOUT: Duration = Duration::from_secs(20);

const INITIALIZE_REQUEST_ID: u64 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthStatus {
    /// Subscription login. The email and plan are only kept in memory for the UI.
    Account {
        email: Option<String>,
        plan: Option<String>,
    },
    /// Logged in through another mechanism (API key, gateway, external credentials).
    OtherCredentials {
        kind: String,
        label: Option<String>,
    },
    LoggedOut,
}

#[derive(Deserialize)]
struct StatusNotification {
    method: String,
    params: StatusParams,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StatusParams {
    auth_status: RawStatus,
}

#[derive(Deserialize)]
struct RawStatus {
    kind: String,
    label: Option<String>,
    account: Option<RawAccount>,
}

#[derive(Deserialize)]
struct RawAccount {
    email: Option<String>,
    plan: Option<String>,
}

/// Returns `None` for any line that is not an `_auth/status_update` notification.
pub fn parse_status_notification(line: &str) -> Option<AuthStatus> {
    let notification = serde_json::from_str::<StatusNotification>(line).ok()?;
    if notification.method != "_auth/status_update" {
        return None;
    }
    let status = notification.params.auth_status;
    Some(match status.kind.as_str() {
        "account" => {
            let account = status.account;
            AuthStatus::Account {
                email: account.as_ref().and_then(|account| account.email.clone()),
                plan: account.and_then(|account| account.plan),
            }
        }
        "none" => AuthStatus::LoggedOut,
        _ => AuthStatus::OtherCredentials {
            kind: status.kind,
            label: status.label,
        },
    })
}

#[derive(Deserialize)]
struct Response {
    id: Option<u64>,
    error: Option<ResponseError>,
}

#[derive(Deserialize)]
struct ResponseError {
    message: String,
}

fn initialize_request() -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": INITIALIZE_REQUEST_ID,
        "method": "initialize",
        "params": {
            "protocolVersion": 1,
            "clientCapabilities": {
                "fs": { "readTextFile": false, "writeTextFile": false },
                "terminal": false,
                "auth": { "terminal": true },
                "_meta": { "terminal-auth": true, "terminal_output": true },
            },
            "clientInfo": { "name": "uacode-office-probe", "version": env!("CARGO_PKG_VERSION") },
        },
    })
    .to_string()
}

/// Short-lived adapter process. It is killed when dropped, which also covers a probe task
/// that is cancelled because the panel closed or UACode exits.
pub struct ProbeProcess {
    child: Child,
}

impl ProbeProcess {
    pub fn spawn(command: AgentServerCommand) -> Result<Self> {
        let builder = ShellBuilder::new(&Shell::System, cfg!(windows))
            .non_interactive()
            .literal_args();
        let mut std_command =
            builder.build_std_command(Some(command.path.display().to_string()), &command.args);
        std_command.envs(command.env.unwrap_or_default());
        let child = Child::spawn(std_command, Stdio::piped(), Stdio::piped(), Stdio::null())
            .context("No se pudo arrancar el adaptador")?;
        Ok(Self { child })
    }

    pub async fn read_status(
        &mut self,
        executor: &BackgroundExecutor,
        initialize_timeout: Duration,
        status_timeout: Duration,
    ) -> Result<AuthStatus> {
        let mut stdin = self
            .child
            .stdin
            .take()
            .context("El adaptador no tiene stdin")?;
        let stdout = self
            .child
            .stdout
            .take()
            .context("El adaptador no tiene stdout")?;
        let mut request = initialize_request();
        request.push('\n');
        stdin
            .write_all(request.as_bytes())
            .await
            .context("No se pudo enviar initialize al adaptador")?;

        let mut lines = BufReader::new(stdout).lines();
        let mut deadline = Instant::now() + initialize_timeout;
        let mut initialized = false;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let line = futures::select_biased! {
                line = lines.next().fuse() => line,
                _ = executor.timer(remaining).fuse() => {
                    bail!(if initialized {
                        "El adaptador no informó del estado de la cuenta a tiempo"
                    } else {
                        "El adaptador no respondió a initialize a tiempo"
                    })
                }
            };
            let line = line
                .ok_or_else(|| anyhow!("El adaptador terminó sin informar del estado"))?
                .context("No se pudo leer la salida del adaptador")?;
            if let Some(status) = parse_status_notification(&line) {
                return Ok(status);
            }
            if !initialized
                && let Ok(response) = serde_json::from_str::<Response>(&line)
                && response.id == Some(INITIALIZE_REQUEST_ID)
            {
                if let Some(error) = response.error {
                    bail!("initialize falló: {}", error.message);
                }
                initialized = true;
                deadline = Instant::now() + status_timeout;
            }
        }
    }

    #[cfg(all(test, windows))]
    fn has_running_processes(&self) -> Result<Option<bool>> {
        self.child.has_running_processes()
    }
}

impl ProbeProcess {
    fn kill(&mut self) {
        self.child.kill().log_err();
    }
}

impl Drop for ProbeProcess {
    fn drop(&mut self) {
        self.kill();
    }
}

pub async fn probe(
    command: AgentServerCommand,
    executor: BackgroundExecutor,
) -> Result<AuthStatus> {
    let mut process = ProbeProcess::spawn(command)?;
    process
        .read_status(&executor, INITIALIZE_TIMEOUT, STATUS_TIMEOUT)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_logged_out_status() {
        let line = r#"{"jsonrpc":"2.0","method":"_auth/status_update","params":{"authStatus":{"kind":"none","label":"Not logged in"}}}"#;
        assert_eq!(parse_status_notification(line), Some(AuthStatus::LoggedOut));
    }

    #[test]
    fn parses_account_status_with_identity() {
        let line = r#"{"jsonrpc":"2.0","method":"_auth/status_update","params":{"authStatus":{"kind":"account","label":"Claude Max","account":{"email":"user@example.com","plan":"max","organization":"Org"}}}}"#;
        assert_eq!(
            parse_status_notification(line),
            Some(AuthStatus::Account {
                email: Some("user@example.com".into()),
                plan: Some("max".into()),
            })
        );
    }

    #[test]
    fn parses_other_credentials() {
        let line = r#"{"jsonrpc":"2.0","method":"_auth/status_update","params":{"authStatus":{"kind":"api_key","label":"API key"}}}"#;
        assert_eq!(
            parse_status_notification(line),
            Some(AuthStatus::OtherCredentials {
                kind: "api_key".into(),
                label: Some("API key".into()),
            })
        );
    }

    #[test]
    fn ignores_other_messages() {
        for line in [
            r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#,
            r#"{"jsonrpc":"2.0","method":"session/update","params":{}}"#,
            "npm warn exec The following package was not found",
            "",
        ] {
            assert_eq!(parse_status_notification(line), None, "{line}");
        }
    }

    #[cfg(windows)]
    #[gpui::test]
    async fn probe_kills_process_tree_after_timeout(cx: &mut gpui::TestAppContext) {
        cx.executor().allow_parking();
        let command = AgentServerCommand {
            path: "ping.exe".into(),
            args: vec!["-n".into(), "60".into(), "127.0.0.1".into()],
            env: None,
        };
        let process = ProbeProcess::spawn(command).expect("spawn probe");
        let executor = cx.executor();
        let probe = cx.executor().spawn(async move {
            let mut process = process;
            let result = process
                .read_status(&executor, Duration::from_secs(1), Duration::from_secs(1))
                .await;
            (process, result)
        });
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_secs(2));
        let (mut process, result) = probe.await;
        let error = result.expect_err("a silent process must time out");
        assert!(error.to_string().contains("a tiempo"), "{error:#}");
        assert_eq!(
            process.has_running_processes().expect("query job"),
            Some(true),
            "the process tree is still running before the probe is stopped"
        );

        // The same call `Drop` makes; the job covers the shell and its descendants.
        process.kill();
        let deadline = Instant::now() + Duration::from_secs(5);
        while process.has_running_processes().expect("query job") != Some(false) {
            assert!(
                Instant::now() < deadline,
                "probe processes survived the timeout"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}
