//! ACP client implementation used inside the bridge thread.

use crate::acp::bridge::{AcpEvt, AcpPermissionOption};
use crate::acp::types::AcpUpdate;

use agent_client_protocol::{self as acp};
use tokio::sync::{mpsc, oneshot};

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

pub type PendingPermissionReplies = Rc<RefCell<HashMap<String, oneshot::Sender<Option<String>>>>>;

/// Local bridge-side ACP client.
pub struct BridgeClient {
    evt_tx: mpsc::Sender<AcpEvt>,
    pending_replies: PendingPermissionReplies,
    default_permission_timeout: Duration,
}

impl BridgeClient {
    pub fn new(
        evt_tx: mpsc::Sender<AcpEvt>,
        pending_replies: PendingPermissionReplies,
        default_permission_timeout: Duration,
    ) -> Self {
        Self {
            evt_tx,
            pending_replies,
            default_permission_timeout,
        }
    }
}

#[async_trait::async_trait(?Send)]
impl acp::Client for BridgeClient {
    async fn request_permission(
        &self,
        args: acp::RequestPermissionRequest,
    ) -> acp::Result<acp::RequestPermissionResponse> {
        let request_id = args.tool_call.tool_call_id.to_string();
        let title = args
            .tool_call
            .fields
            .title
            .clone()
            .unwrap_or_else(|| "ACP tool permission requested".into());
        let options = args
            .options
            .iter()
            .map(|option| AcpPermissionOption {
                option_id: option.option_id.to_string(),
                name: option.name.clone(),
                kind: match option.kind {
                    acp::PermissionOptionKind::AllowOnce => "allow_once",
                    acp::PermissionOptionKind::AllowAlways => "allow_always",
                    acp::PermissionOptionKind::RejectOnce => "reject_once",
                    acp::PermissionOptionKind::RejectAlways => "reject_always",
                    _ => "unknown",
                }
                .to_string(),
            })
            .collect::<Vec<_>>();

        self.evt_tx
            .send(AcpEvt::PermissionRequested {
                request_id: request_id.clone(),
                description: title,
                options,
            })
            .await
            .ok();

        let (reply_tx, reply_rx) = oneshot::channel();
        self.pending_replies
            .borrow_mut()
            .insert(request_id.clone(), reply_tx);

        let selected_option =
            match tokio::time::timeout(self.default_permission_timeout, reply_rx).await {
                Ok(Ok(option_id)) => option_id,
                _ => args
                    .options
                    .iter()
                    .find(|option| {
                        matches!(
                            option.kind,
                            acp::PermissionOptionKind::AllowOnce
                                | acp::PermissionOptionKind::AllowAlways
                        )
                    })
                    .or_else(|| args.options.first())
                    .map(|option| option.option_id.to_string()),
            };

        self.pending_replies.borrow_mut().remove(&request_id);

        let outcome = if let Some(option_id) = selected_option {
            acp::RequestPermissionOutcome::Selected(acp::SelectedPermissionOutcome::new(option_id))
        } else {
            acp::RequestPermissionOutcome::Cancelled
        };

        Ok(acp::RequestPermissionResponse::new(outcome))
    }

    async fn session_notification(&self, args: acp::SessionNotification) -> acp::Result<()> {
        if let Some(update) = AcpUpdate::from_session_update(&args.update) {
            self.evt_tx
                .send(AcpEvt::SessionUpdate { update })
                .await
                .ok();
        }
        Ok(())
    }

    async fn write_text_file(
        &self,
        _args: acp::WriteTextFileRequest,
    ) -> acp::Result<acp::WriteTextFileResponse> {
        Err(acp::Error::method_not_found())
    }

    async fn read_text_file(
        &self,
        _args: acp::ReadTextFileRequest,
    ) -> acp::Result<acp::ReadTextFileResponse> {
        Err(acp::Error::method_not_found())
    }

    async fn create_terminal(
        &self,
        _args: acp::CreateTerminalRequest,
    ) -> acp::Result<acp::CreateTerminalResponse> {
        Err(acp::Error::method_not_found())
    }

    async fn terminal_output(
        &self,
        _args: acp::TerminalOutputRequest,
    ) -> acp::Result<acp::TerminalOutputResponse> {
        Err(acp::Error::method_not_found())
    }

    async fn release_terminal(
        &self,
        _args: acp::ReleaseTerminalRequest,
    ) -> acp::Result<acp::ReleaseTerminalResponse> {
        Err(acp::Error::method_not_found())
    }

    async fn wait_for_terminal_exit(
        &self,
        _args: acp::WaitForTerminalExitRequest,
    ) -> acp::Result<acp::WaitForTerminalExitResponse> {
        Err(acp::Error::method_not_found())
    }

    async fn kill_terminal(
        &self,
        _args: acp::KillTerminalRequest,
    ) -> acp::Result<acp::KillTerminalResponse> {
        Err(acp::Error::method_not_found())
    }

    async fn ext_method(&self, _args: acp::ExtRequest) -> acp::Result<acp::ExtResponse> {
        Err(acp::Error::method_not_found())
    }

    async fn ext_notification(&self, _args: acp::ExtNotification) -> acp::Result<()> {
        Ok(())
    }
}
