use super::{api::ApiOperation, model::*};
use crate::connectors::{accounts::Account, Permission};
use crate::credentials::AccountId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum MutationAction {
    CreateMailDraft {
        mail: MailInput,
    },
    SendMail {
        mail: MailInput,
    },
    CreateCalendarEvent {
        event: EventInput,
    },
    UpdateCalendarEvent {
        event_id: String,
        etag: String,
        changes: EventChanges,
    },
}
impl MutationAction {
    pub fn permission(&self) -> Permission {
        match self {
            Self::CreateMailDraft { .. } => Permission::MailDraft,
            Self::SendMail { .. } => Permission::MailSend,
            Self::CreateCalendarEvent { .. } => Permission::CalendarEventCreate,
            Self::UpdateCalendarEvent { .. } => Permission::CalendarEventUpdate,
        }
    }
    pub fn operation(&self) -> &'static str {
        match self {
            Self::CreateMailDraft { .. } => "create_mail_draft",
            Self::SendMail { .. } => "send_mail",
            Self::CreateCalendarEvent { .. } => "create_calendar_event",
            Self::UpdateCalendarEvent { .. } => "update_calendar_event",
        }
    }
    pub fn request(&self, sender: Option<&str>, created_at: u64) -> Result<ApiOperation> {
        Ok(match self {
            Self::CreateMailDraft { mail } => ApiOperation::Draft {
                raw: mail.raw_at(sender, created_at)?,
            },
            Self::SendMail { mail } => ApiOperation::Send {
                raw: mail.raw_at(sender, created_at)?,
            },
            Self::CreateCalendarEvent { event } => ApiOperation::EventCreate {
                event: event.clone(),
            },
            Self::UpdateCalendarEvent {
                event_id,
                etag,
                changes,
            } => ApiOperation::EventUpdate {
                id: event_id.clone(),
                etag: etag.clone(),
                changes: changes.clone(),
            },
        })
    }
    pub fn phrase(&self) -> &'static str {
        match self {
            Self::CreateMailDraft { .. } => "SAVE DRAFT",
            Self::SendMail { .. } => "SEND MAIL",
            Self::CreateCalendarEvent { .. } => "CREATE EVENT",
            Self::UpdateCalendarEvent { .. } => "UPDATE EVENT",
        }
    }
}
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MutationPreview {
    pub sender: Option<String>,
    pub account_label: String,
    pub mail: Option<MailInput>,
    pub old_event: Option<EventInput>,
    pub new_event: Option<EventInput>,
    pub body_bytes: Option<usize>,
}
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PreparedMutation {
    pub version: u8,
    pub request_id: String,
    pub account_id: AccountId,
    pub required_permission: Permission,
    pub payload: MutationAction,
    pub preview: MutationPreview,
    pub created_at: u64,
    pub expires_at: u64,
    pub context_sha256: String,
    pub request_sha256: String,
}
impl PreparedMutation {
    pub fn digest(&self) -> Result<String> {
        let mut copy = self.clone();
        copy.request_sha256.clear();
        Ok(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&copy).map_err(|_| "invalid_input")?)
        ))
    }
    pub fn confirmation(&self) -> String {
        format!("{} {}", self.payload.phrase(), self.request_sha256)
    }
}
pub fn context(account: &Account, client_id: &str) -> Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(account, client_id)).map_err(|_| "invalid_input")?)
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareInput {
    pub account_id: AccountId,
    pub payload: MutationAction,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecuteInput {
    pub prepared: PreparedMutation,
    pub confirmation: String,
}
#[derive(Serialize)]
pub struct MutationResult {
    pub operation: String,
    pub provider_id: String,
    pub audit_recorded: bool,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditOperation {
    Connect,
    Disconnect,
    MailSearch,
    MailDigest,
    Agenda,
    FreeTime,
    Contacts,
    CreateMailDraft,
    SendMail,
    CreateCalendarEvent,
    UpdateCalendarEvent,
}
impl AuditOperation {
    pub fn mutation(action: &MutationAction) -> Self {
        match action {
            MutationAction::CreateMailDraft { .. } => Self::CreateMailDraft,
            MutationAction::SendMail { .. } => Self::SendMail,
            MutationAction::CreateCalendarEvent { .. } => Self::CreateCalendarEvent,
            MutationAction::UpdateCalendarEvent { .. } => Self::UpdateCalendarEvent,
        }
    }
}
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditResult {
    Confirmed,
    Completed,
    Failed,
    ReconciliationRequired,
}
#[derive(Serialize)]
pub struct AuditEvent {
    pub timestamp: u64,
    pub provider: crate::credentials::Provider,
    pub account_id: AccountId,
    pub operation: AuditOperation,
    pub permission: Option<Permission>,
    pub result: AuditResult,
    pub request_sha256: Option<String>,
    pub recipient_count: Option<usize>,
}
