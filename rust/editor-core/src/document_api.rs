#![allow(
    clippy::result_large_err,
    reason = "SessionError is the established unboxed six-domain boundary envelope"
)]

#[cfg(test)]
use crate::boundary::{parse_json_value_stack_safe, BoundaryError, BoundedInput, InputKind};
use crate::registry::{self, SessionId};
#[cfg(test)]
use crate::schema::presets::tiptap_schema;
use crate::schema::Schema;
use crate::serialize::FromHtmlOptions;
use crate::session::{
    DocumentState, EditorInitialization, EditorSession, EditorSessionConfig, InitialContent,
    SessionError, SessionPolicy,
};
use crate::yrs_engine::{
    InitializationMode, ReplacementHistory, TransactionCommit, TransactionOrigin,
    YrsDocumentEngine, YrsEngineConfig,
};

pub(crate) struct DocumentApiFacade;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
// Not reachable from production call paths after the legacy runtime removal;
// exercised by crate tests.
#[allow(dead_code)]
pub(crate) struct ContentSnapshot {
    html: String,
    json: serde_json::Value,
}

impl DocumentApiFacade {
    #[cfg(test)]
    pub(crate) fn create(config: EditorSessionConfig) -> Result<SessionId, SessionError> {
        registry::create_session(|| {
            let schema = resolve_schema(&config)?;
            Self::admit(config, schema)
        })
    }

    pub(crate) fn create_with_schema(
        config: EditorSessionConfig,
        schema: Schema,
    ) -> Result<SessionId, SessionError> {
        registry::create_session(|| Self::admit(config, schema))
    }

    // Not reachable from production call paths after the Task 16C legacy runtime
    // removal; exercised by crate tests.
    #[allow(dead_code)]
    pub(crate) fn get_json(id: SessionId) -> Result<serde_json::Value, SessionError> {
        with_session(id, |session| session.get_json())
    }

    // Not reachable from production call paths after the Task 16C legacy runtime
    // removal; exercised by crate tests.
    #[allow(dead_code)]
    pub(crate) fn get_html(id: SessionId) -> Result<String, SessionError> {
        with_session(id, |session| session.get_html())
    }

    // Not reachable from production call paths after the Task 16C legacy runtime
    // removal; exercised by crate tests.
    #[allow(dead_code)]
    pub(crate) fn get_content_snapshot(id: SessionId) -> Result<ContentSnapshot, SessionError> {
        with_session(id, |session| {
            Ok(ContentSnapshot {
                html: session.get_html()?,
                json: session.get_json()?,
            })
        })
    }

    /// Whole-document replacement from ProseMirror JSON. The session policy
    /// gate runs first; allowed rows lower to one same-store root transaction.
    // Not reachable from production call paths after the Task 16C legacy runtime
    // removal; exercised by crate tests.
    #[allow(dead_code)]
    pub(crate) fn write_json(
        id: SessionId,
        request_id: u64,
        json: &str,
        history: ReplacementHistory,
    ) -> Result<TransactionCommit, SessionError> {
        with_session(id, |session| {
            session.replace_document_json(request_id, json, history)
        })
    }

    /// Whole-document replacement from HTML under the same policy gate.
    // Not reachable from production call paths after the Task 16C legacy runtime
    // removal; exercised by crate tests.
    #[allow(dead_code)]
    pub(crate) fn write_html(
        id: SessionId,
        request_id: u64,
        html: &str,
        history: ReplacementHistory,
    ) -> Result<TransactionCommit, SessionError> {
        with_session(id, |session| {
            session.replace_document_html(request_id, html, history)
        })
    }

    fn admit(config: EditorSessionConfig, schema: Schema) -> Result<EditorSession, SessionError> {
        config.collaboration_limits.validate()?;
        let policy = SessionPolicy::from_config(&config);
        let (engine, document_state) = match &config.initialization {
            EditorInitialization::Local { initial_content } => {
                let mut engine = YrsDocumentEngine::new(engine_config(
                    &config,
                    schema,
                    InitializationMode::LocalEmpty,
                    None,
                ))?;
                match initial_content {
                    InitialContent::Empty => {}
                    InitialContent::Json(json) => {
                        engine.import_json(json, TransactionOrigin::DocumentImport)?;
                    }
                    InitialContent::Html(html) => {
                        engine.import_html(
                            html,
                            &FromHtmlOptions {
                                strict: false,
                                allow_base64_images: config.allow_base64_images,
                            },
                            TransactionOrigin::DocumentImport,
                        )?;
                    }
                }
                (engine, DocumentState::LocalReady)
            }
            EditorInitialization::Room { scope, snapshot } => {
                let engine_config = engine_config(
                    &config,
                    schema,
                    InitializationMode::AwaitRemote,
                    Some(scope.clone()),
                );
                match snapshot {
                    Some(snapshot) => (
                        YrsDocumentEngine::new_with_snapshot(engine_config, snapshot)?,
                        DocumentState::RoomReady,
                    ),
                    None => (
                        YrsDocumentEngine::new(engine_config)?,
                        DocumentState::AwaitRemote,
                    ),
                }
            }
        };

        EditorSession::new(engine, policy, document_state, config.collaboration_limits)
    }
}

fn engine_config(
    config: &EditorSessionConfig,
    schema: Schema,
    initialization_mode: InitializationMode,
    scope: Option<crate::yrs_engine::DocumentScope>,
) -> YrsEngineConfig {
    YrsEngineConfig {
        schema,
        fragment_name: config.fragment_name.clone(),
        initialization_mode,
        resource_limits: config.resource_limits.clone(),
        editing_limits: config.editing_limits.clone(),
        max_length: config.max_length,
        scope,
    }
}

#[cfg(test)]
fn resolve_schema(config: &EditorSessionConfig) -> Result<Schema, SessionError> {
    let Some(schema_json) = &config.schema_json else {
        return Ok(tiptap_schema());
    };
    let input = BoundedInput::new(schema_json, InputKind::Config, &config.resource_limits)?;
    let container_limit = crate::schema::MAX_SCHEMA_METADATA_DEPTH
        .checked_add(16)
        .ok_or_else(|| BoundaryError::new("SCHEMA_INVALID", "schema depth limit overflow"))?;
    let value = parse_json_value_stack_safe(
        input.as_str(),
        container_limit,
        crate::schema::MAX_SCHEMA_METADATA_DEPTH,
        "SCHEMA_INVALID",
        "SCHEMA_INVALID",
    )?;
    Schema::from_json_with_limits(value.as_value(), &config.resource_limits)
        .map_err(SessionError::from)
}

// Not reachable from production call paths after the legacy runtime removal;
// exercised by crate tests.
#[allow(dead_code)]
fn with_session<T>(
    id: SessionId,
    operation: impl FnOnce(&mut EditorSession) -> Result<T, SessionError>,
) -> Result<T, SessionError> {
    let slot = registry::get_session(id).ok_or_else(|| {
        SessionError::new(
            crate::session::ErrorDomain::Lifecycle,
            "ENGINE_DESTROYED",
            "editor session is not registered",
        )
    })?;
    slot.with_alive(|session| operation(session))
        .and_then(|value| value)
}

#[cfg(test)]
#[path = "document_api/session_initialization_test_support.rs"]
pub mod session_initialization_test_support;
