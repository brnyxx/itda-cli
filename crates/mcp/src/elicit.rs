//! Approving a change in the MCP client: the person sees the exact request in
//! a form the client shows them, and their answer grants or rejects it.
use crate::Caller;
use itda_policy::{Approval, Approvals};
use itda_runtime::audit::{AuditLog, Event};
use itda_types::error::{AppError, AppResult};
use rmcp::model::{ElicitRequestParams, ElicitationAction, ElicitationSchema};
use serde_json::Value;
use std::{sync::Arc, time::Duration};

/// How long the form waits for the person; approvals expire after five minutes.
const WAIT: Duration = Duration::from_secs(290);

#[derive(Clone)]
pub(crate) struct ClientApproval {
    pub(crate) approvals: Arc<Approvals>,
    pub(crate) audit: Arc<AuditLog>,
}

impl ClientApproval {
    /// Ask the person about approval `id`. `None` when the client cannot ask,
    /// so the approval stays pending for `itda approve`.
    pub(crate) async fn ask(&self, caller: &Caller<'_>, id: &str) -> Option<AppResult<()>> {
        if !shows_forms(caller) {
            return None;
        }
        let review = match self.approvals.review(id).await {
            Ok(review) => review,
            Err(error) => return Some(Err(error)),
        };
        let schema = ElicitationSchema::builder()
            .required_bool_with("approve", |b| {
                b.title("Approve this change")
                    .description("It runs once, with exactly these inputs.")
            })
            .build()
            .ok()?;
        let answer = caller
            .peer
            .create_elicitation_with_timeout(
                ElicitRequestParams::FormElicitationParams {
                    meta: None,
                    message: message(&review),
                    requested_schema: schema,
                },
                Some(WAIT),
            )
            .await
            .ok()?;
        let approved = answer.action == ElicitationAction::Accept
            && answer
                .content
                .as_ref()
                .and_then(|content| content.get("approve"))
                .and_then(Value::as_bool)
                == Some(true);
        Some(if approved {
            self.decide(&review, "approved").await
        } else {
            self.decide(&review, "rejected").await.and(Err(
                AppError::api("approval_rejected", 403).with("approval", review.id.clone())
            ))
        })
    }

    async fn decide(&self, review: &Approval, kind: &'static str) -> AppResult<()> {
        match kind {
            "approved" => self.approvals.claim(&review.id).await?,
            _ => self.approvals.reject(&review.id).await?,
        }
        self.audit.append(Event {
            kind,
            site: review.site_id.clone(),
            action: review.action_id.clone(),
            account: review.account_id.clone(),
            approval: review.id.clone(),
            inputs: review
                .request
                .as_ref()
                .map(|r| r.inputs.clone())
                .unwrap_or_default(),
            outcome: "ok".into(),
            ..Event::default()
        })?;
        Ok(())
    }
}

/// Whether the client said it can show the person a form. A client that
/// declared `elicitation` without saying which kind is taken to show forms,
/// as the 2025-06-18 specification reads.
pub(crate) fn shows_forms(caller: &Caller<'_>) -> bool {
    caller
        .capabilities
        .as_ref()
        .and_then(|capabilities| capabilities.elicitation.as_ref())
        .is_some_and(|elicitation| {
            elicitation.form.is_some() || (elicitation.form.is_none() && elicitation.url.is_none())
        })
}

/// What came of a yes-or-no question put to the person.
pub(crate) enum Confirmed {
    Yes,
    No,
    /// The form was shown but nobody answered before the wait ran out, or
    /// the client failed it. Not a yes.
    Unanswered,
    /// The client cannot show forms at all.
    CannotAsk,
}

/// Put a yes-or-no question to the person in the client.
pub(crate) async fn confirm(
    caller: &Caller<'_>,
    message: String,
    title: &'static str,
) -> Confirmed {
    if !shows_forms(caller) {
        return Confirmed::CannotAsk;
    }
    let Ok(schema) = ElicitationSchema::builder()
        .required_bool_with("confirm", |b| b.title(title))
        .build()
    else {
        return Confirmed::CannotAsk;
    };
    let Ok(answer) = caller
        .peer
        .create_elicitation_with_timeout(
            ElicitRequestParams::FormElicitationParams {
                meta: None,
                message,
                requested_schema: schema,
            },
            Some(WAIT),
        )
        .await
    else {
        return Confirmed::Unanswered;
    };
    let yes = answer.action == ElicitationAction::Accept
        && answer
            .content
            .as_ref()
            .and_then(|content| content.get("confirm"))
            .and_then(Value::as_bool)
            == Some(true);
    if yes { Confirmed::Yes } else { Confirmed::No }
}

/// What the person reviews: the same lines `itda approve` prints.
fn message(review: &Approval) -> String {
    let mut text = format!(
        "{} wants to change {} ({}) as account {}:\n",
        review.action_title, review.site_title, review.site_id, review.account_id
    );
    for input in &review.inputs {
        text.push_str(&format!("\n{}: {}", input.label, input.value));
    }
    text
}
