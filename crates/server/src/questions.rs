//! Durable asynchronous questions and the answer outbox. Credentials use secrets.rs instead.
use crate::app::{ApiError, ApiResult, App};
use axum::{Json, body::Bytes, extract::State};
use boblm_protocol::client::{
    AnswerQuestion, CommandBody, DiscussQuestionLater, ListQuestions, QuestionInfo, QuestionState,
    QuestionsReply,
};
use rmcp::schemars;
use std::sync::Arc;

#[derive(Debug, Clone, serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AskArgs {
    /// Fresh UUID. Reuse only for an identical explicit retry.
    pub question_id: String,
    pub question: String,
    #[serde(default)]
    pub choices: Vec<String>,
}
#[derive(Debug, Clone, serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct UpdateQuestionArgs {
    pub question_id: String,
    /// Fresh UUID for the revised prompt. Reuse only for an identical retry.
    pub replacement_id: String,
    pub question: String,
    #[serde(default)]
    pub choices: Vec<String>,
}
#[derive(Debug, Clone, serde::Deserialize, rmcp::schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DeleteQuestionArgs {
    pub question_id: String,
}
fn validate_question(question: &str, choices: &[String]) -> Result<(), String> {
    if question.trim().is_empty()
        || question.chars().count() > boblm_tuning::MAX_QUESTION_TEXT_CHARS
    {
        return Err(format!(
            "A question needs 1 to {} characters. Shorten it and try again.",
            boblm_tuning::MAX_QUESTION_TEXT_CHARS
        ));
    }
    if choices.len() > boblm_tuning::MAX_QUESTION_CHOICES
        || choices.iter().any(|c| {
            c.trim().is_empty() || c.chars().count() > boblm_tuning::MAX_QUESTION_CHOICE_CHARS
        })
    {
        return Err(format!(
            "Offer at most {} nonempty choices of at most {} characters each. Shorten the choices and try again.",
            boblm_tuning::MAX_QUESTION_CHOICES,
            boblm_tuning::MAX_QUESTION_CHOICE_CHARS
        ));
    }
    Ok(())
}
async fn question_session(app: &App, root: &str, id: &str) -> Result<String, ApiError> {
    let store = app.store.clone();
    let id = id.to_owned();
    let root = root.to_owned();
    tokio::task::spawn_blocking(move || {
        let reader = store.reader()?;
        let owner = reader
            .question_owner_session(&id)?
            .ok_or_else(|| boblm_store::Error::QuestionMissing)?;
        let session = reader
            .sessions()?
            .into_iter()
            .find(|session| session.id == owner)
            .ok_or_else(|| boblm_store::Error::QuestionMissing)?;
        let same_root = session.id == root
            || session.root_session_id.as_deref() == Some(root.as_str())
            || session.parent_session_id.as_deref() == Some(root.as_str());
        if !same_root {
            return Err(boblm_store::Error::QuestionMissing);
        }
        Ok(owner)
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(|_| {
        ApiError::invalid(
            "This question does not belong to this chat or its subagents, or it no longer exists. Reload the questions dialog.",
        )
    })
}

fn wire(q: boblm_store::Question) -> Result<QuestionInfo, ApiError> {
    Ok(QuestionInfo {
        id: q.id,
        session_id: q.session,
        question: q.question,
        state: if q.discuss_later != 0 {
            QuestionState::DiscussLater
        } else if q.answer.is_some() {
            QuestionState::Answered
        } else {
            QuestionState::Pending
        },
        choices: serde_json::from_str(&q.choices).map_err(ApiError::internal)?,
        answer: q.answer,
        created_at: q.created_at,
        answered_at: q.answered_at,
    })
}
impl App {
    pub async fn ask(&self, session: &str, args: AskArgs) -> Result<serde_json::Value, String> {
        let id = boblm_runner::canonical_uuid(&args.question_id).map_err(|_| {
            "questionId must be a fresh UUID. Reuse it only for an identical retry.".to_string()
        })?;
        validate_question(&args.question, &args.choices)?;
        self.handle(session).await.map_err(|e| e.to_string())?;
        let (store, session) = (self.store.clone(), session.to_owned());
        let q = tokio::task::spawn_blocking(move || {
            store.ask(
                &id,
                &session,
                &args.question,
                &args.choices,
                crate::schedule::now_ms(),
            )
        })
        .await
        .map_err(|e| {
            format!("Could not record the question: {e}. Try again with the same questionId.")
        })?
        .map_err(|e| e.to_string())?;
        serde_json::to_value(wire(q).map_err(|e|e.to_string())?).map_err(|e|format!("Could not return the recorded question: {e}. Inspect the questions dialog before retrying."))
    }
    pub async fn update_question(
        &self,
        session: &str,
        args: UpdateQuestionArgs,
    ) -> Result<serde_json::Value, String> {
        let id = boblm_runner::canonical_uuid(&args.question_id).map_err(|e| e.to_string())?;
        let replacement =
            boblm_runner::canonical_uuid(&args.replacement_id).map_err(|e| e.to_string())?;
        validate_question(&args.question, &args.choices)?;
        self.handle(session).await.map_err(|e| e.to_string())?;
        let target = question_session(self, session, &id)
            .await
            .map_err(|e| e.to_string())?;
        let (store, session) = (self.store.clone(), target);
        let reply = serde_json::json!({"questionId": replacement, "replacedQuestionId": id});
        tokio::task::spawn_blocking(move || {
            store.update_question(
                &id,
                &replacement,
                &session,
                &args.question,
                &args.choices,
                crate::schedule::now_ms(),
            )
        })
        .await
        .map_err(|e| {
            format!("Could not update the question: {e}. Retry with the same replacementId.")
        })?
        .map_err(|e| e.to_string())?;
        Ok(reply)
    }
    pub async fn delete_question(
        &self,
        session: &str,
        args: DeleteQuestionArgs,
    ) -> Result<serde_json::Value, String> {
        let _delivery = self.question_delivery.lock().await;
        let id = boblm_runner::canonical_uuid(&args.question_id).map_err(|e| e.to_string())?;
        self.handle(session).await.map_err(|e| e.to_string())?;
        let target = question_session(self, session, &id)
            .await
            .map_err(|e| e.to_string())?;
        let (store, session) = (self.store.clone(), target);
        let reply = serde_json::json!({"questionId": id, "deleted": true});
        tokio::task::spawn_blocking(move || {
            store.delete_question(&id, &session, crate::schedule::now_ms())
        })
        .await
        .map_err(|e| {
            format!("Could not delete the question: {e}. Retry with the same questionId.")
        })?
        .map_err(|e| e.to_string())?;
        Ok(reply)
    }
    /// Stable answer IDs make an interrupted handoff safe to retry without replaying held input.
    pub async fn deliver_question_answers(&self) {
        // A deletion either suppresses the outbox item or follows its handoff.
        // Never deliver a stale snapshot after deletion has returned successfully.
        let _delivery = self.question_delivery.lock().await;
        let store = self.store.clone();
        let pending =
            tokio::task::spawn_blocking(move || store.reader()?.pending_question_answers()).await;
        let pending = match pending {
            Ok(Ok(pending)) => pending,
            Ok(Err(error)) => {
                eprintln!("server: could not read pending question answers: {error}");
                return;
            }
            Err(error) => {
                eprintln!("server: could not read pending question answers: {error}");
                return;
            }
        };
        for q in pending {
            let discussion = q.discuss_later != 0;
            let message_id = crate::schedule::derived_uuid(&format!(
                "question-{}:{}",
                if discussion { "discussion" } else { "answer" },
                q.id
            ));
            let content = if discussion {
                format!(
                    "Discussion requested for your question ({}):\n\n{}\n\nThe owner chose Discuss later: \"I'm not sure. Let's discuss once there are no more open question prompts.\" This chat's prompt queue is now empty. Discuss this question with the owner; no answer was supplied.",
                    q.id, q.question
                )
            } else if let Some(answer) = &q.answer {
                format!(
                    "Answer to your question ({}):\n\n{}\n\nOwner's answer:\n{}",
                    q.id, q.question, answer
                )
            } else {
                continue;
            };
            let result = async {
                if discussion {
                    let (store, session) = (self.store.clone(), q.session.clone());
                    if tokio::task::spawn_blocking(move || {
                        store.reader()?.pending_question_count(&session)
                    })
                    .await
                    .map_err(ApiError::internal)??
                        > 0
                    {
                        return Ok(());
                    }
                }
                self.handle(&q.session)
                    .await?
                    .send_automated(&message_id, &content)
                    .await?;
                let store = self.store.clone();
                tokio::task::spawn_blocking(move || store.accept_question_delivery(&q.id))
                    .await
                    .map_err(ApiError::internal)??;
                Ok::<_, ApiError>(())
            }
            .await;
            if let Err(error) = result {
                eprintln!("server: could not deliver a question answer: {error}");
            }
        }
    }
}
pub async fn list(State(app): State<Arc<App>>, bytes: Bytes) -> ApiResult<QuestionsReply> {
    let _: ListQuestions = super::app::body(&bytes)?;
    let store = app.store.clone();
    let (rows, count) = tokio::task::spawn_blocking(move || {
        let reader = store.reader()?;
        Ok::<_, boblm_store::Error>((
            reader.all_questions()?,
            reader.pending_question_count_all()?,
        ))
    })
    .await
    .map_err(ApiError::internal)??;
    let mut reply = QuestionsReply {
        pending_count: u32::try_from(count).map_err(ApiError::internal)?,
        questions: rows.into_iter().map(wire).collect::<Result<_, _>>()?,
    };
    while serde_json::to_vec(&reply)
        .map_err(ApiError::internal)?
        .len()
        > boblm_tuning::MAX_BODY_BYTES
    {
        let Some(index) = reply
            .questions
            .iter()
            .rposition(|q| !matches!(q.state, QuestionState::Pending))
        else {
            return Err(ApiError::invalid(
                "These pending questions exceed the response limit. Ask agents to use shorter question text or fewer choices.",
            ));
        };
        reply.questions.remove(index);
    }
    Ok(Json(reply))
}
pub async fn answer(State(app): State<Arc<App>>, bytes: Bytes) -> ApiResult<QuestionsReply> {
    let request: AnswerQuestion = super::app::body(&bytes)?;
    request.validate().map_err(ApiError::invalid)?;
    let id = boblm_runner::canonical_uuid(&request.question_id)?;
    app.handle(&request.session_id).await?;
    let target = question_session(&app, &request.session_id, &id).await?;
    let store = app.store.clone();
    let q = tokio::task::spawn_blocking(move || {
        store.answer_question(&id, &target, &request.answer, crate::schedule::now_ms())
    })
    .await
    .map_err(ApiError::internal)??;
    // The scheduler also drains this outbox, including after a lost HTTP connection or restart.
    app.deliver_question_answers().await;
    let store = app.store.clone();
    let count = tokio::task::spawn_blocking(move || store.reader()?.pending_question_count_all())
        .await
        .map_err(ApiError::internal)??;
    Ok(Json(QuestionsReply {
        pending_count: u32::try_from(count).map_err(ApiError::internal)?,
        questions: vec![wire(q)?],
    }))
}

pub async fn discuss_later(State(app): State<Arc<App>>, bytes: Bytes) -> ApiResult<QuestionsReply> {
    let request: DiscussQuestionLater = super::app::body(&bytes)?;
    let id = boblm_runner::canonical_uuid(&request.question_id)?;
    app.handle(&request.session_id).await?;
    let target = question_session(&app, &request.session_id, &id).await?;
    let store = app.store.clone();
    let q = tokio::task::spawn_blocking(move || {
        store.discuss_question_later(&id, &target, crate::schedule::now_ms())
    })
    .await
    .map_err(ApiError::internal)??;
    app.deliver_question_answers().await;
    let store = app.store.clone();
    let count = tokio::task::spawn_blocking(move || store.reader()?.pending_question_count_all())
        .await
        .map_err(ApiError::internal)??;
    Ok(Json(QuestionsReply {
        pending_count: u32::try_from(count).map_err(ApiError::internal)?,
        questions: vec![wire(q)?],
    }))
}
