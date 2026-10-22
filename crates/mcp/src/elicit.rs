//! Approving a change in the MCP client: the person sees the exact request in
//! a form the client shows them, and their answer grants or rejects it.
//!
//! Up to protocol 2025-11-25 the server asks with an `elicitation/create`
//! request and waits for the answer. From 2026-07-28 the client takes no
//! requests: the form goes back as the result of the tool call
//! (`resultType: input_required`), the client shows it, and calls the tool
//! again with the same arguments, the answer in `inputResponses` and the
//! `requestState` it was given. See [`Rounds`].
use crate::Caller;
use itda_policy::{Approval, Approvals};
use itda_runtime::audit::{AuditLog, Event};
use itda_types::error::{AppError, AppResult};
use rmcp::model::{
    ElicitRequest, ElicitRequestParams, ElicitationAction, ElicitationSchema, InputRequest,
    InputRequiredResult, InputResponses,
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};

/// How long the form waits for the person; approvals expire after five minutes.
const WAIT: Duration = Duration::from_secs(290);

/// The key of the one form a call puts to the person, in `inputRequests` and
/// `inputResponses`.
const APPROVE: &str = "approve";
const CONFIRM: &str = "confirm";

/// A connection on protocol 2026-07-28 or newer: what the client answered on
/// this call, and the form to hand back when the call has to ask.
pub(crate) struct Rounds {
    /// The client's answers to the forms of the previous round, by key.
    answers: InputResponses,
    /// The `requestState` the client echoed: the id of the approval it
    /// answered. It passes through the client, so it is only ever used to look
    /// the approval up; what is approved is what the server stored under it.
    state: Option<String>,
    /// The form the call asks the person to answer, when it does.
    asked: Mutex<Option<InputRequiredResult>>,
}

impl Rounds {
    pub(crate) fn new(answers: Option<InputResponses>, state: Option<String>) -> Self {
        Self {
            answers: answers.unwrap_or_default(),
            state,
            asked: Mutex::new(None),
        }
    }

    /// The form to send back instead of a result, if the call asked one.
    pub(crate) fn take_form(&self) -> Option<InputRequiredResult> {
        self.asked
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }

    fn ask(&self, key: &str, params: ElicitRequestParams, state: Option<String>) {
        let mut requests = BTreeMap::new();
        requests.insert(
            key.to_owned(),
            InputRequest::Elicitation(ElicitRequest::new(params)),
        );
        let form = InputRequiredResult::new(Some(requests), state);
        *self
            .asked
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(form);
    }

    /// The person's answer to the form under `key`, if the client sent one.
    fn answer(&self, key: &str) -> Option<bool> {
        let answer = self.answers.get(key)?;
        let accepted = answer.get("action").and_then(Value::as_str) == Some("accept")
            && answer
                .get("content")
                .and_then(|content| content.get(key))
                .and_then(Value::as_bool)
                == Some(true);
        Some(accepted)
    }
}

/// What the person decided about a change, or that they are being asked.
pub(crate) enum Decision {
    /// The person approved: run the change with this approval id.
    Approved(String),
    /// The person said no, or closed the form.
    Rejected(AppError),
    /// The form went back to the client as this call's result.
    Asked,
}

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
        let params = form(&review)?;
        let Some(rounds) = &caller.rounds else {
            return Some(self.ask_and_wait(caller, &review, params).await);
        };
        rounds.ask(APPROVE, params, Some(review.id.clone()));
        // The result is replaced by the form; what it says does not matter.
        Some(Err(
            AppError::api("approval_pending", 409).with("approval", review.id.clone())
        ))
    }

    /// Before a change runs on a 2026-07-28 connection: the answer to the
    /// form of the previous round, for the approval the client named, when
    /// that approval is this very call. Anything else is a first call.
    pub(crate) async fn answered(
        &self,
        caller: &Caller<'_>,
        site: &str,
        action: &str,
        inputs: &BTreeMap<String, String>,
    ) -> Option<Decision> {
        let rounds = caller.rounds.as_ref()?;
        let id = rounds.state.as_deref()?;
        let review = self.approvals.review(id).await.ok()?;
        let same = review.site_id == site
            && review.action_id == action
            && review
                .request
                .as_ref()
                .is_some_and(|request| request.inputs == *inputs);
        if !same {
            return None;
        }
        let Some(approved) = rounds.answer(APPROVE) else {
            // The client came back without an answer: ask again.
            rounds.ask(APPROVE, form(&review)?, Some(review.id.clone()));
            return Some(Decision::Asked);
        };
        Some(match self.decide(&review, approved).await {
            Ok(()) if approved => Decision::Approved(review.id.clone()),
            Ok(()) => Decision::Rejected(rejected(&review.id)),
            Err(error) => Decision::Rejected(error),
        })
    }

    async fn ask_and_wait(
        &self,
        caller: &Caller<'_>,
        review: &Approval,
        params: ElicitRequestParams,
    ) -> AppResult<()> {
        let Some(answer) = caller
            .peer
            .create_elicitation_with_timeout(params, Some(WAIT))
            .await
            .ok()
        else {
            return Err(AppError::api("approval_pending", 409).with("approval", review.id.clone()));
        };
        let approved = answer.action == ElicitationAction::Accept
            && answer
                .content
                .as_ref()
                .and_then(|content| content.get(APPROVE))
                .and_then(Value::as_bool)
                == Some(true);
        self.decide(review, approved).await?;
        if approved {
            Ok(())
        } else {
            Err(rejected(&review.id))
        }
    }

    async fn decide(&self, review: &Approval, approved: bool) -> AppResult<()> {
        let kind = if approved {
            self.approvals.claim(&review.id).await?;
            "approved"
        } else {
            self.approvals.reject(&review.id).await?;
            "rejected"
        };
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

fn rejected(id: &str) -> AppError {
    AppError::api("approval_rejected", 403).with("approval", id.to_owned())
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
    /// The form went back to the client as this call's result; the person
    /// has not answered yet. Not a yes.
    Asked,
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
        .required_bool_with(CONFIRM, |b| b.title(title))
        .build()
    else {
        return Confirmed::CannotAsk;
    };
    let params = ElicitRequestParams::FormElicitationParams {
        meta: None,
        message,
        requested_schema: schema,
    };
    if let Some(rounds) = &caller.rounds {
        return match rounds.answer(CONFIRM) {
            Some(true) => Confirmed::Yes,
            Some(false) => Confirmed::No,
            None => {
                rounds.ask(CONFIRM, params, None);
                Confirmed::Asked
            }
        };
    }
    let Ok(answer) = caller
        .peer
        .create_elicitation_with_timeout(params, Some(WAIT))
        .await
    else {
        return Confirmed::Unanswered;
    };
    let yes = answer.action == ElicitationAction::Accept
        && answer
            .content
            .as_ref()
            .and_then(|content| content.get(CONFIRM))
            .and_then(Value::as_bool)
            == Some(true);
    if yes { Confirmed::Yes } else { Confirmed::No }
}

/// The form that reviews a change: the same lines `itda approve` prints, and
/// one box to tick.
fn form(review: &Approval) -> Option<ElicitRequestParams> {
    let schema = ElicitationSchema::builder()
        .required_bool_with(APPROVE, |b| {
            b.title("Approve this change")
                .description("It runs once, with exactly these inputs.")
        })
        .build()
        .ok()?;
    Some(ElicitRequestParams::FormElicitationParams {
        meta: None,
        message: message(review),
        requested_schema: schema,
    })
}

/// What the person reviews: the same lines `itda approve` prints. Clients
/// show the first lines first, so nothing is wasted on a blank one.
fn message(review: &Approval) -> String {
    let mut text = format!(
        "{} wants to change {} ({})",
        review.action_title, review.site_title, review.site_id
    );
    if !review.account_id.is_empty() {
        text.push_str(&format!(" as account {}", review.account_id));
    }
    text.push(':');
    for input in &review.inputs {
        text.push_str(&format!("\n{}: {}", input.label, input.value));
    }
    text
}
