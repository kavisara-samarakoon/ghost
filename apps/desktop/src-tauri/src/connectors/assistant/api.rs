use super::model::*;
use crate::credentials::Secret;
use reqwest::{
    blocking::{Client, Request},
    Method,
};
use serde::{
    de::{self, MapAccess, SeqAccess, Visitor},
    Deserialize,
};
use serde_json::Value;
use std::io::Read;
use std::time::Duration;
use zeroize::Zeroizing;

pub const MAX_RESPONSE_BYTES: usize = 256 * 1024;
pub const MAX_TOTAL_BYTES: usize = 2 * 1024 * 1024;
pub enum ApiOperation {
    Profile,
    MailList {
        query: String,
        limit: usize,
    },
    MailMetadata {
        id: String,
    },
    Draft {
        raw: String,
    },
    Send {
        raw: String,
    },
    Agenda {
        window: Window,
        limit: usize,
    },
    FreeBusy {
        window: Window,
    },
    EventGet {
        id: String,
    },
    EventCreate {
        event: EventInput,
    },
    EventUpdate {
        id: String,
        etag: String,
        changes: EventChanges,
    },
    Contacts,
}
impl ApiOperation {
    pub fn request(&self, client: &Client, token: &Secret) -> Result<Request> {
        let (root, path, method) = match self {
            Self::Profile => (
                "https://gmail.googleapis.com",
                "/gmail/v1/users/me/profile".into(),
                Method::GET,
            ),
            Self::MailList { .. } => (
                "https://gmail.googleapis.com",
                "/gmail/v1/users/me/messages".into(),
                Method::GET,
            ),
            Self::MailMetadata { id } => (
                "https://gmail.googleapis.com",
                format!("/gmail/v1/users/me/messages/{}", mail_id(id)?),
                Method::GET,
            ),
            Self::Draft { .. } => (
                "https://gmail.googleapis.com",
                "/gmail/v1/users/me/drafts".into(),
                Method::POST,
            ),
            Self::Send { .. } => (
                "https://gmail.googleapis.com",
                "/gmail/v1/users/me/messages/send".into(),
                Method::POST,
            ),
            Self::Agenda { .. } => (
                "https://www.googleapis.com",
                "/calendar/v3/calendars/primary/events".into(),
                Method::GET,
            ),
            Self::FreeBusy { .. } => (
                "https://www.googleapis.com",
                "/calendar/v3/freeBusy".into(),
                Method::POST,
            ),
            Self::EventGet { id } => (
                "https://www.googleapis.com",
                format!("/calendar/v3/calendars/primary/events/{}", identifier(id)?),
                Method::GET,
            ),
            Self::EventCreate { .. } => (
                "https://www.googleapis.com",
                "/calendar/v3/calendars/primary/events".into(),
                Method::POST,
            ),
            Self::EventUpdate { id, .. } => (
                "https://www.googleapis.com",
                format!("/calendar/v3/calendars/primary/events/{}", identifier(id)?),
                Method::PATCH,
            ),
            Self::Contacts => (
                "https://people.googleapis.com",
                "/v1/people/me/connections".into(),
                Method::GET,
            ),
        };
        let raw = Zeroizing::new(format!("Bearer {}", token.expose()));
        let mut auth = reqwest::header::HeaderValue::from_str(&raw).map_err(|_| "auth_required")?;
        auth.set_sensitive(true);
        let mut builder = client
            .request(method, format!("{root}{path}"))
            .header(reqwest::header::AUTHORIZATION, auth);
        match self {
            Self::Profile=>builder=builder.query(&[("fields","emailAddress")]),
            Self::MailList{query,limit}=>builder=builder.query(&[("q",query.as_str()),("maxResults",&limit.to_string()),("includeSpamTrash","false"),("fields","messages(id,threadId),nextPageToken")]),
            Self::MailMetadata{..}=>builder=builder.query(&[("format","metadata"),("metadataHeaders","From"),("metadataHeaders","Subject"),("metadataHeaders","Date"),("fields","id,threadId,snippet,labelIds,internalDate,payload/headers")]),
            Self::Draft{raw}=>builder=builder.json(&serde_json::json!({"message":{"raw":raw}})).query(&[("fields","id,message/id")]),
            Self::Send{raw}=>builder=builder.json(&serde_json::json!({"raw":raw})).query(&[("fields","id")]),
            Self::Agenda{window,limit}=>builder=builder.query(&[("timeMin",window.start.as_str()),("timeMax",window.end.as_str()),("maxResults",&limit.to_string()),("singleEvents","true"),("orderBy","startTime"),("showDeleted","false"),("fields","items(id,etag,summary,description,location,start,end,status),nextPageToken")]),
            Self::FreeBusy{window}=>builder=builder.json(&serde_json::json!({"timeMin":window.start,"timeMax":window.end,"items":[{"id":"primary"}],"calendarExpansionMax":1})),
            Self::EventGet{..}=>builder=builder.query(&[("fields","id,etag,summary,description,location,start,end,status,attendees,recurrence,recurringEventId,eventType")]),
            Self::EventCreate{event}=>builder=builder.json(&event.provider()).query(&[("fields","id"),("sendUpdates","none")]),
            Self::EventUpdate{etag:tag,changes,..}=>builder=builder.header(reqwest::header::IF_MATCH,etag(tag)?).json(&changes.provider()).query(&[("fields","id"),("sendUpdates","none")]),
            Self::Contacts=>builder=builder.query(&[("personFields","names,emailAddresses,phoneNumbers,organizations"),("pageSize","100"),("sortOrder","FIRST_NAME_ASCENDING"),("fields","connections(resourceName,names/displayName,emailAddresses/value,phoneNumbers/value,organizations/name),nextPageToken")]),
        }
        builder.build().map_err(|_| "invalid_input")
    }
}
pub struct HttpReply {
    pub status: u16,
    pub body: Zeroizing<Vec<u8>>,
}
pub trait ApiTransport {
    fn set_deadline(&mut self, _: Option<std::time::Instant>) {}
    fn call(&mut self, operation: &ApiOperation, token: &Secret) -> Result<HttpReply>;
    fn refresh(
        &mut self,
        request: crate::connectors::google::TokenRequest,
    ) -> std::result::Result<
        crate::connectors::google::TokenHttpResponse,
        crate::connectors::ConnectorError,
    >;
}
#[derive(Default)]
pub struct GoogleTransport {
    deadline: Option<std::time::Instant>,
}
pub fn client() -> Result<Client> {
    client_with_timeout(Duration::from_secs(45))
}
fn client_with_timeout(timeout: Duration) -> Result<Client> {
    Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(timeout.min(Duration::from_secs(10)))
        .timeout(timeout.min(Duration::from_secs(45)))
        .build()
        .map_err(|_| "transport_failed")
}
impl ApiTransport for GoogleTransport {
    fn set_deadline(&mut self, deadline: Option<std::time::Instant>) {
        self.deadline = deadline;
    }
    fn refresh(
        &mut self,
        request: crate::connectors::google::TokenRequest,
    ) -> std::result::Result<
        crate::connectors::google::TokenHttpResponse,
        crate::connectors::ConnectorError,
    > {
        crate::connectors::google::send_token_request(request)
    }
    fn call(&mut self, operation: &ApiOperation, token: &Secret) -> Result<HttpReply> {
        let timeout = self
            .deadline
            .map(|d| d.saturating_duration_since(std::time::Instant::now()))
            .unwrap_or(Duration::from_secs(45));
        if timeout.is_zero() {
            return Err("timeout");
        }
        let client = client_with_timeout(timeout)?;
        let request = operation.request(&client, token)?;
        let response = client.execute(request).map_err(|e| {
            if e.is_timeout() {
                "timeout"
            } else {
                "transport_failed"
            }
        })?;
        let status = response.status().as_u16();
        status_check(status)?;
        if response
            .content_length()
            .is_some_and(|s| s > MAX_RESPONSE_BYTES as u64)
        {
            return Err("invalid_response");
        }
        let length = response.content_length();
        let mut body = Zeroizing::new(Vec::new());
        response
            .take(MAX_RESPONSE_BYTES as u64 + 1)
            .read_to_end(&mut body)
            .map_err(|_| "transport_failed")?;
        if body.len() > MAX_RESPONSE_BYTES || length.is_some_and(|s| s != body.len() as u64) {
            return Err("invalid_response");
        }
        Ok(HttpReply { status, body })
    }
}
pub fn status_check(status: u16) -> Result<()> {
    match status {
        200..=299 => Ok(()),
        401 => Err("auth_required"),
        403 => Err("permission_missing"),
        409 | 412 => Err("conflict"),
        _ => Err("provider_failed"),
    }
}

// Value's default parser accepts duplicate keys; provider replies must not have ambiguous fields.
struct Strict(Value);
impl<'de> Deserialize<'de> for Strict {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Strict;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("strict JSON")
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Strict, A::Error> {
                let mut map = serde_json::Map::new();
                while let Some((key, value)) = a.next_entry::<String, Strict>()? {
                    if map.insert(key, value.0).is_some() {
                        return Err(de::Error::custom("duplicate field"));
                    }
                }
                Ok(Strict(Value::Object(map)))
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Strict, A::Error> {
                let mut v = Vec::new();
                while let Some(s) = a.next_element::<Strict>()? {
                    v.push(s.0);
                }
                Ok(Strict(Value::Array(v)))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<Strict, E> {
                Ok(Strict(Value::String(v.into())))
            }
            fn visit_string<E: de::Error>(self, v: String) -> std::result::Result<Strict, E> {
                Ok(Strict(Value::String(v)))
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> std::result::Result<Strict, E> {
                Ok(Strict(Value::Bool(v)))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<Strict, E> {
                Ok(Strict(Value::Number(v.into())))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<Strict, E> {
                Ok(Strict(Value::Number(v.into())))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> std::result::Result<Strict, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| Strict(Value::Number(n)))
                    .ok_or_else(|| de::Error::custom("invalid number"))
            }
            fn visit_unit<E: de::Error>(self) -> std::result::Result<Strict, E> {
                Ok(Strict(Value::Null))
            }
        }
        d.deserialize_any(V)
    }
}
pub fn parse(reply: HttpReply, key: &Secret, budget: &mut usize) -> Result<Value> {
    status_check(reply.status)?;
    *budget = budget
        .checked_add(reply.body.len())
        .ok_or("invalid_response")?;
    if reply.body.len() > MAX_RESPONSE_BYTES || *budget > MAX_TOTAL_BYTES {
        return Err("invalid_response");
    }
    let text = std::str::from_utf8(&reply.body).map_err(|_| "invalid_response")?;
    let mut value = serde_json::from_str::<Strict>(text)
        .map_err(|_| "invalid_response")?
        .0;
    if !value.is_object() {
        return Err("invalid_response");
    }
    fn redact(v: &mut Value, key: &str) {
        match v {
            Value::String(s) => *s = crate::snapshot::text::redact(&s.replace(key, "[REDACTED]")),
            Value::Array(a) => {
                for v in a {
                    redact(v, key)
                }
            }
            Value::Object(o) => {
                for v in o.values_mut() {
                    redact(v, key)
                }
            }
            _ => (),
        }
    }
    redact(&mut value, key.expose());
    Ok(value)
}
