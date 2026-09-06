#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InitialContent {
    Empty,
    Json(String),
    Html(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EditorInitialization {
    Local {
        initial_content: InitialContent,
    },
    Room {
        scope: DocumentScope,
        snapshot: Option<DocumentSnapshot>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CollaborationLimits {
    pub(crate) max_frames_per_message: usize,
    pub(crate) max_frame_bytes: usize,
    pub(crate) max_aggregate_response_bytes: usize,
    pub(crate) max_awareness_peers: usize,
    pub(crate) max_awareness_peer_bytes: usize,
    pub(crate) max_awareness_bytes: usize,
    pub(crate) max_pending_outbox_messages: usize,
    pub(crate) max_pending_outbox_bytes: usize,
    pub(crate) max_pending_dependency_update_bytes: usize,
    pub(crate) max_pending_dependency_update_work: usize,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CollaborationLimitOverrides {
    #[serde(
        default,
        deserialize_with = "crate::boundary::deserialize_non_null_option"
    )]
    max_frames_per_message: Option<usize>,
    #[serde(
        default,
        deserialize_with = "crate::boundary::deserialize_non_null_option"
    )]
    max_frame_bytes: Option<usize>,
    #[serde(
        default,
        deserialize_with = "crate::boundary::deserialize_non_null_option"
    )]
    max_aggregate_response_bytes: Option<usize>,
    #[serde(
        default,
        deserialize_with = "crate::boundary::deserialize_non_null_option"
    )]
    max_awareness_peers: Option<usize>,
    #[serde(
        default,
        deserialize_with = "crate::boundary::deserialize_non_null_option"
    )]
    max_awareness_peer_bytes: Option<usize>,
    #[serde(
        default,
        deserialize_with = "crate::boundary::deserialize_non_null_option"
    )]
    max_awareness_bytes: Option<usize>,
    #[serde(
        default,
        deserialize_with = "crate::boundary::deserialize_non_null_option"
    )]
    max_pending_outbox_messages: Option<usize>,
    #[serde(
        default,
        deserialize_with = "crate::boundary::deserialize_non_null_option"
    )]
    max_pending_outbox_bytes: Option<usize>,
    #[serde(
        default,
        deserialize_with = "crate::boundary::deserialize_non_null_option"
    )]
    max_pending_dependency_update_bytes: Option<usize>,
    #[serde(
        default,
        deserialize_with = "crate::boundary::deserialize_non_null_option"
    )]
    max_pending_dependency_update_work: Option<usize>,
}

impl Default for CollaborationLimits {
    fn default() -> Self {
        Self {
            max_frames_per_message: 64,
            max_frame_bytes: 10 * 1024 * 1024,
            max_aggregate_response_bytes: 10 * 1024 * 1024,
            max_awareness_peers: 1_024,
            max_awareness_peer_bytes: 64 * 1024,
            max_awareness_bytes: 10 * 1024 * 1024,
            max_pending_outbox_messages: 256,
            max_pending_outbox_bytes: 10 * 1024 * 1024,
            max_pending_dependency_update_bytes: 10 * 1024 * 1024,
            max_pending_dependency_update_work: 1_000_000,
        }
    }
}

impl CollaborationLimits {
    pub(crate) fn resolve(overrides: CollaborationLimitOverrides) -> Result<Self, SessionError> {
        let defaults = Self::default();
        let limits = Self {
            max_frames_per_message: overrides
                .max_frames_per_message
                .unwrap_or(defaults.max_frames_per_message),
            max_frame_bytes: overrides
                .max_frame_bytes
                .unwrap_or(defaults.max_frame_bytes),
            max_aggregate_response_bytes: overrides
                .max_aggregate_response_bytes
                .unwrap_or(defaults.max_aggregate_response_bytes),
            max_awareness_peers: overrides
                .max_awareness_peers
                .unwrap_or(defaults.max_awareness_peers),
            max_awareness_peer_bytes: overrides
                .max_awareness_peer_bytes
                .unwrap_or(defaults.max_awareness_peer_bytes),
            max_awareness_bytes: overrides
                .max_awareness_bytes
                .unwrap_or(defaults.max_awareness_bytes),
            max_pending_outbox_messages: overrides
                .max_pending_outbox_messages
                .unwrap_or(defaults.max_pending_outbox_messages),
            max_pending_outbox_bytes: overrides
                .max_pending_outbox_bytes
                .unwrap_or(defaults.max_pending_outbox_bytes),
            max_pending_dependency_update_bytes: overrides
                .max_pending_dependency_update_bytes
                .unwrap_or(defaults.max_pending_dependency_update_bytes),
            max_pending_dependency_update_work: overrides
                .max_pending_dependency_update_work
                .unwrap_or(defaults.max_pending_dependency_update_work),
        };
        limits.validate()?;
        Ok(limits)
    }

    pub(crate) const fn hard_ceiling() -> Self {
        Self {
            max_frames_per_message: 1_024,
            max_frame_bytes: 64 * 1024 * 1024,
            max_aggregate_response_bytes: 64 * 1024 * 1024,
            max_awareness_peers: 10_000,
            max_awareness_peer_bytes: 1024 * 1024,
            max_awareness_bytes: 64 * 1024 * 1024,
            max_pending_outbox_messages: 4_096,
            max_pending_outbox_bytes: 64 * 1024 * 1024,
            max_pending_dependency_update_bytes: 64 * 1024 * 1024,
            max_pending_dependency_update_work: 8_000_000,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), SessionError> {
        let ceilings = Self::hard_ceiling();
        for (field, actual, ceiling) in [
            (
                "maxFramesPerMessage",
                self.max_frames_per_message,
                ceilings.max_frames_per_message,
            ),
            (
                "maxFrameBytes",
                self.max_frame_bytes,
                ceilings.max_frame_bytes,
            ),
            (
                "maxAggregateResponseBytes",
                self.max_aggregate_response_bytes,
                ceilings.max_aggregate_response_bytes,
            ),
            (
                "maxAwarenessPeers",
                self.max_awareness_peers,
                ceilings.max_awareness_peers,
            ),
            (
                "maxAwarenessPeerBytes",
                self.max_awareness_peer_bytes,
                ceilings.max_awareness_peer_bytes,
            ),
            (
                "maxAwarenessBytes",
                self.max_awareness_bytes,
                ceilings.max_awareness_bytes,
            ),
            (
                "maxPendingOutboxMessages",
                self.max_pending_outbox_messages,
                ceilings.max_pending_outbox_messages,
            ),
            (
                "maxPendingOutboxBytes",
                self.max_pending_outbox_bytes,
                ceilings.max_pending_outbox_bytes,
            ),
            (
                "maxPendingDependencyUpdateBytes",
                self.max_pending_dependency_update_bytes,
                ceilings.max_pending_dependency_update_bytes,
            ),
            (
                "maxPendingDependencyUpdateWork",
                self.max_pending_dependency_update_work,
                ceilings.max_pending_dependency_update_work,
            ),
        ] {
            if actual == 0 || actual > ceiling {
                return Err(SessionError::new(
                    ErrorDomain::Boundary,
                    "INVALID_RESOURCE_LIMIT",
                    format!("{field} must be a positive integer no greater than {ceiling}"),
                )
                .with_limit(ceiling, actual)
                .with_details(serde_json::json!({ "field": field })));
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) const fn fields() -> [&'static str; 10] {
        [
            "maxFramesPerMessage",
            "maxFrameBytes",
            "maxAggregateResponseBytes",
            "maxAwarenessPeers",
            "maxAwarenessPeerBytes",
            "maxAwarenessBytes",
            "maxPendingOutboxMessages",
            "maxPendingOutboxBytes",
            "maxPendingDependencyUpdateBytes",
            "maxPendingDependencyUpdateWork",
        ]
    }

    #[cfg(test)]
    pub(crate) fn value(&self, field: &str) -> usize {
        match field {
            "maxFramesPerMessage" => self.max_frames_per_message,
            "maxFrameBytes" => self.max_frame_bytes,
            "maxAggregateResponseBytes" => self.max_aggregate_response_bytes,
            "maxAwarenessPeers" => self.max_awareness_peers,
            "maxAwarenessPeerBytes" => self.max_awareness_peer_bytes,
            "maxAwarenessBytes" => self.max_awareness_bytes,
            "maxPendingOutboxMessages" => self.max_pending_outbox_messages,
            "maxPendingOutboxBytes" => self.max_pending_outbox_bytes,
            "maxPendingDependencyUpdateBytes" => self.max_pending_dependency_update_bytes,
            "maxPendingDependencyUpdateWork" => self.max_pending_dependency_update_work,
            _ => unreachable!("unknown collaboration limit field"),
        }
    }

    /// Test-only field mutation by wire name, shared by the in-crate limits
    /// matrix and the staging integration-test support (which drives the
    /// Task 9 receive ceilings to their exact/one-over boundaries).
    pub(crate) fn set_for_test(&mut self, field: &str, value: usize) {
        match field {
            "maxFramesPerMessage" => self.max_frames_per_message = value,
            "maxFrameBytes" => self.max_frame_bytes = value,
            "maxAggregateResponseBytes" => self.max_aggregate_response_bytes = value,
            "maxAwarenessPeers" => self.max_awareness_peers = value,
            "maxAwarenessPeerBytes" => self.max_awareness_peer_bytes = value,
            "maxAwarenessBytes" => self.max_awareness_bytes = value,
            "maxPendingOutboxMessages" => self.max_pending_outbox_messages = value,
            "maxPendingOutboxBytes" => self.max_pending_outbox_bytes = value,
            "maxPendingDependencyUpdateBytes" => self.max_pending_dependency_update_bytes = value,
            "maxPendingDependencyUpdateWork" => self.max_pending_dependency_update_work = value,
            _ => unreachable!("unknown collaboration limit field"),
        }
    }

    #[cfg(test)]
    pub(crate) fn as_pairs_json(&self) -> serde_json::Value {
        serde_json::json!({
            "maxFramesPerMessage": self.max_frames_per_message,
            "maxFrameBytes": self.max_frame_bytes,
            "maxAggregateResponseBytes": self.max_aggregate_response_bytes,
            "maxAwarenessPeers": self.max_awareness_peers,
            "maxAwarenessPeerBytes": self.max_awareness_peer_bytes,
            "maxAwarenessBytes": self.max_awareness_bytes,
            "maxPendingOutboxMessages": self.max_pending_outbox_messages,
            "maxPendingOutboxBytes": self.max_pending_outbox_bytes,
            "maxPendingDependencyUpdateBytes": self.max_pending_dependency_update_bytes,
            "maxPendingDependencyUpdateWork": self.max_pending_dependency_update_work,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct EditorSessionConfig {
    pub(crate) schema_json: Option<String>,
    pub(crate) fragment_name: String,
    pub(crate) initialization: EditorInitialization,
    pub(crate) resource_limits: ResourceLimits,
    pub(crate) editing_limits: EditingLimits,
    pub(crate) collaboration_limits: CollaborationLimits,
    pub(crate) max_length: Option<u32>,
    pub(crate) read_only: bool,
    pub(crate) input_filter: Option<String>,
    pub(crate) allow_base64_images: bool,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RoomInitializationEnvelope {
    document_id: String,
    lineage_id: String,
    snapshot: Option<DocumentSnapshot>,
}

impl EditorSessionConfig {
    pub(crate) fn room_from_json(input: &str) -> Result<Self, SessionError> {
        let limits = ResourceLimits::default();
        let input = BoundedInput::new(input, InputKind::Config, &limits)?;
        let envelope: RoomInitializationEnvelope = serde_json::from_str(input.as_str())
            .map_err(|error| BoundaryError::parse("CONFIG_INVALID", error))?;
        Ok(Self {
            schema_json: None,
            fragment_name: "prosemirror".into(),
            initialization: EditorInitialization::Room {
                scope: DocumentScope {
                    document_id: envelope.document_id,
                    lineage_id: envelope.lineage_id,
                },
                snapshot: envelope.snapshot,
            },
            resource_limits: limits,
            editing_limits: EditingLimits::default(),
            collaboration_limits: CollaborationLimits::default(),
            max_length: None,
            read_only: false,
            input_filter: None,
            allow_base64_images: false,
        })
    }
}

#[cfg(test)]
impl EditorSessionConfig {
    pub(crate) fn local_for_test() -> Self {
        Self {
            schema_json: None,
            fragment_name: "prosemirror".into(),
            initialization: EditorInitialization::Local {
                initial_content: InitialContent::Empty,
            },
            resource_limits: ResourceLimits::default(),
            editing_limits: EditingLimits::default(),
            collaboration_limits: CollaborationLimits::default(),
            max_length: None,
            read_only: false,
            input_filter: None,
            allow_base64_images: false,
        }
    }
}
