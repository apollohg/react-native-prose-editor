use super::commit::CompiledCommitAuthority;
use super::YrsDocumentEngine;
use crate::boundary::ResourceLimits;
use crate::model::Document;
use crate::position::PositionMap;
use crate::schema::Schema;
use crate::yrs_engine;
use crate::yrs_engine::compiler::{
    map_position, selectable_void_at, CompiledTransaction, SelectionPlan, StoredMarksPlan,
};
use crate::yrs_engine::TransactionOrigin;
use std::sync::Arc;

impl YrsDocumentEngine {
    pub(super) fn prepare_empty_skip_result(
        &self,
        request_id: u64,
        origin: TransactionOrigin,
        selection: &yrs_engine::ResolvedSelection,
        stored_marks: Option<&[crate::model::Mark]>,
        changed: bool,
        state_revision: u64,
    ) -> yrs_engine::OperationResult<yrs_engine::TypedTransactionResult> {
        let current = self
            .derived_state
            .as_ref()
            .ok_or_else(|| yrs_engine::OperationError::engine_not_ready(request_id))?;
        let legacy_selection = yrs_engine::derived_state::resolved_to_legacy(selection);
        let commands = crate::editor_state::command_applicability_with_known_node_count(
            &current.document,
            &self.schema,
            &legacy_selection,
            &self.resource_limits,
            current.document_node_count,
        );
        let active_state = crate::editor_state::active_state(
            &current.document,
            &self.schema,
            &legacy_selection,
            stored_marks,
            commands,
            &self.resource_limits,
        );
        let result = yrs_engine::TypedTransactionResult {
            request_id,
            origin,
            changed,
            document_revision: self.revision,
            state_revision,
            selection: selection.clone(),
            active_state,
            history_state: crate::editor_state::HistoryState {
                can_undo: self.can_undo(),
                can_redo: self.can_redo(),
            },
            render_update: yrs_engine::RenderUpdate::None,
        };
        self.admit_typed_result(request_id, &result)?;
        Ok(result)
    }

    pub(super) fn prepare_typed_result(
        &self,
        compiled: &CompiledTransaction,
        render_update: yrs_engine::RenderUpdate,
        commit_authority: &CompiledCommitAuthority<'_, '_>,
    ) -> yrs_engine::OperationResult<(
        yrs_engine::TypedTransactionResult,
        Option<Arc<yrs_engine::derived_state::CachedActiveState>>,
    )> {
        let current = self
            .derived_state
            .as_ref()
            .ok_or_else(|| yrs_engine::OperationError::engine_not_ready(compiled.request_id))?;
        let selection = match &compiled.selection_plan {
            SelectionPlan::Preserve => current.resolved_selection.clone(),
            SelectionPlan::Explicit(selection) | SelectionPlan::Mapped(selection) => {
                let (position_map, rendered_text) = compiled
                    .preview_derivations
                    .as_ref()
                    .map(|derivations| {
                        (
                            &derivations.position_map,
                            derivations.rendered_text.as_str(),
                        )
                    })
                    .unwrap_or((&current.position_map, current.rendered_text.as_str()));
                yrs_engine::derived_state::resolved_from_legacy_with_view(
                    &compiled.preview,
                    selection,
                    &self.schema,
                    position_map,
                    rendered_text,
                )
                .ok_or_else(|| {
                    yrs_engine::OperationError::engine_invariant_failed(
                        compiled.request_id,
                        None,
                        "compiled result selection cannot be resolved",
                    )
                })?
            }
        };
        let StoredMarksPlan::Set(stored_marks) = &compiled.stored_marks_plan else {
            return Err(yrs_engine::OperationError::engine_invariant_failed(
                compiled.request_id,
                None,
                "compiled result stored-mark plan is not sealed",
            ));
        };
        let legacy_selection = yrs_engine::derived_state::resolved_to_legacy(&selection);
        let document_node_count = compiled
            .preview_derivations
            .as_ref()
            .map(|derivations| derivations.document_node_count)
            .unwrap_or(current.document_node_count);
        let generic_active_state = || {
            yrs_engine::derived_state::record_active_state_generic_build();
            let commands = crate::editor_state::command_applicability_with_known_node_count(
                &compiled.preview,
                &self.schema,
                &legacy_selection,
                &self.resource_limits,
                document_node_count,
            );
            crate::editor_state::active_state(
                &compiled.preview,
                &self.schema,
                &legacy_selection,
                stored_marks.as_deref(),
                commands,
                &self.resource_limits,
            )
        };
        let (active_state, prepared_active_cache) =
            if let Some(transition) = &compiled.prepared_active_state_transition {
                yrs_engine::derived_state::record_active_state_cache_attempt();
                let structural = compiled.localized_insert_admission.as_ref().map(
                yrs_engine::derived_state::LocalizedInsertAdmission::active_state_structural_seal,
            );
                let validated = if let Some(structural) = structural.as_ref() {
                    current.validate_active_state_transition(
                        commit_authority.derived(),
                        transition,
                        structural,
                        &compiled.preview,
                        &selection,
                        stored_marks.as_deref(),
                        &self.resource_limits,
                        &self.editing_limits,
                        self.max_length,
                        self.yrs_state_epoch,
                    )
                } else {
                    None
                };
                match validated {
                    Some(cached) => {
                        yrs_engine::derived_state::record_active_state_candidate_build();
                        let warm_cached = cached.filter(|_| {
                            !yrs_engine::derived_state::active_state_cache_hit_fallback_forced()
                        });
                        let was_warm = warm_cached.is_some();
                        let cached = if let Some(cached) = warm_cached {
                            cached
                        } else {
                            yrs_engine::derived_state::record_active_state_cache_fallback();
                            let generic = generic_active_state();
                            match yrs_engine::derived_state::CachedActiveState::try_new(
                                generic,
                                &self.resource_limits,
                                &self.editing_limits,
                            ) {
                                Ok(cached) => cached,
                                Err(generic) => {
                                    let result = yrs_engine::TypedTransactionResult {
                                        request_id: compiled.request_id,
                                        origin: compiled.origin,
                                        changed: current.document != compiled.preview,
                                        document_revision: self.revision,
                                        state_revision: self.state_revision,
                                        selection,
                                        active_state: generic,
                                        history_state: crate::editor_state::HistoryState {
                                            can_undo: self.can_undo(),
                                            can_redo: self.can_redo(),
                                        },
                                        render_update,
                                    };
                                    self.admit_typed_result(compiled.request_id, &result)?;
                                    return Ok((result, None));
                                }
                            }
                        };
                        #[cfg(test)]
                        debug_assert_eq!(
                            cached.value(),
                            &crate::editor_state::active_state_for_debug_invariant(
                                &compiled.preview,
                                &self.schema,
                                &legacy_selection,
                                stored_marks.as_deref(),
                                &self.resource_limits,
                                document_node_count,
                            )
                        );
                        if let Some(active_state) =
                            cached.clone_public(&self.resource_limits, &self.editing_limits)
                        {
                            if was_warm {
                                yrs_engine::derived_state::record_active_state_cache_hit();
                            }
                            (active_state, Some(cached))
                        } else {
                            if was_warm {
                                yrs_engine::derived_state::record_active_state_cache_fallback();
                                (generic_active_state(), None)
                            } else {
                                let generic =
                                    yrs_engine::derived_state::CachedActiveState::try_into_value(
                                        cached,
                                    )
                                    .unwrap_or_else(|cached| cached.value().clone());
                                (generic, None)
                            }
                        }
                    }
                    None => {
                        yrs_engine::derived_state::record_active_state_cache_fallback();
                        (generic_active_state(), None)
                    }
                }
            } else {
                // Non-eligible result paths retain the existing generic behavior
                // and are outside the active-state cache lifecycle counters.
                let commands = crate::editor_state::command_applicability_with_known_node_count(
                    &compiled.preview,
                    &self.schema,
                    &legacy_selection,
                    &self.resource_limits,
                    document_node_count,
                );
                (
                    crate::editor_state::active_state(
                        &compiled.preview,
                        &self.schema,
                        &legacy_selection,
                        stored_marks.as_deref(),
                        commands,
                        &self.resource_limits,
                    ),
                    None,
                )
            };
        let result = yrs_engine::TypedTransactionResult {
            request_id: compiled.request_id,
            origin: compiled.origin,
            changed: current.document != compiled.preview,
            document_revision: self.revision,
            state_revision: self.state_revision,
            selection,
            active_state,
            history_state: crate::editor_state::HistoryState {
                can_undo: self.can_undo(),
                can_redo: self.can_redo(),
            },
            render_update,
        };
        self.admit_typed_result(compiled.request_id, &result)?;
        Ok((result, prepared_active_cache))
    }

    pub(super) fn admit_typed_result(
        &self,
        request_id: u64,
        result: &yrs_engine::TypedTransactionResult,
    ) -> yrs_engine::OperationResult<()> {
        let actual = result.derived_output_bytes();
        if actual > self.editing_limits.max_derived_output_bytes {
            return Err(yrs_engine::OperationError::document_limit_exceeded(
                request_id,
                None,
                "maxDerivedOutputBytes",
                u64::try_from(self.editing_limits.max_derived_output_bytes).unwrap_or(u64::MAX),
                u64::try_from(actual).unwrap_or(u64::MAX),
            ));
        }
        let render_elements = match &result.render_update {
            yrs_engine::RenderUpdate::None => 0,
            yrs_engine::RenderUpdate::Patch(patch) => patch.blocks.iter().map(Vec::len).sum(),
            yrs_engine::RenderUpdate::Full(blocks) => blocks.iter().map(Vec::len).sum(),
        };
        let render_element_limit = self.resource_limits.max_document_nodes.saturating_mul(3);
        if render_elements > render_element_limit {
            return Err(yrs_engine::OperationError::document_limit_exceeded(
                request_id,
                None,
                "maxDocumentNodes",
                u64::try_from(render_element_limit).unwrap_or(u64::MAX),
                u64::try_from(render_elements).unwrap_or(u64::MAX),
            ));
        }
        let schema_marks = self.schema.all_marks().count();
        let schema_nodes = self.schema.all_nodes().count();
        let active_is_bounded = result.active_state.marks.len() <= schema_marks
            && result.active_state.mark_attrs.len() <= schema_marks
            && result.active_state.allowed_marks.len() <= schema_marks
            && result.active_state.nodes.len()
                <= self.resource_limits.max_document_depth.saturating_add(1)
            && result.active_state.insertable_nodes.len() <= schema_nodes
            && result.active_state.commands.len() <= 16;
        if !active_is_bounded {
            return Err(yrs_engine::OperationError::document_limit_exceeded(
                request_id,
                None,
                "maxSchemaNodes",
                u64::try_from(schema_nodes.max(schema_marks)).unwrap_or(u64::MAX),
                u64::MAX,
            ));
        }
        Ok(())
    }
}

pub(super) fn affinity_aware_mapped_selection(
    selection: &crate::selection::Selection,
    relative: &yrs_engine::RelativeSelection,
    map: &crate::transform::StepMap,
    preview: &Document,
    schema: &Schema,
    prepared_position_map: Option<&PositionMap>,
) -> crate::selection::Selection {
    let mapped = match (selection, relative) {
        (
            crate::selection::Selection::Text { anchor, head },
            yrs_engine::RelativeSelection::Text {
                anchor: relative_anchor,
                head: relative_head,
            },
        ) => crate::selection::Selection::text(
            map_position(map, *anchor, relative_anchor.affinity),
            map_position(map, *head, relative_head.affinity),
        ),
        (
            crate::selection::Selection::Node { pos },
            yrs_engine::RelativeSelection::Node { point },
        ) => crate::selection::Selection::node(map_position(map, *pos, point.affinity)),
        (crate::selection::Selection::All, yrs_engine::RelativeSelection::All) => {
            crate::selection::Selection::all()
        }
        _ => selection.map(map),
    };
    let owned_position_map;
    let position_map = if let Some(prepared) = prepared_position_map {
        prepared
    } else {
        yrs_engine::derived_state::record_preview_position_map_derivation();
        owned_position_map = PositionMap::build(preview, schema);
        &owned_position_map
    };
    let normalized = mapped.normalized(preview, position_map);
    match normalized {
        crate::selection::Selection::Node { pos }
            if !selectable_void_at(preview.root(), pos, 0, schema) =>
        {
            crate::selection::Selection::cursor(pos).normalized(preview, position_map)
        }
        selection => selection,
    }
}

pub(super) fn cached_transition_render_update(
    update: &crate::render::incremental::CachedRenderTransitionUpdate,
) -> yrs_engine::RenderUpdate {
    match update {
        crate::render::incremental::CachedRenderTransitionUpdate::None => {
            yrs_engine::RenderUpdate::None
        }
        crate::render::incremental::CachedRenderTransitionUpdate::Patch(patch) => {
            yrs_engine::RenderUpdate::Patch(patch.clone())
        }
        crate::render::incremental::CachedRenderTransitionUpdate::Full(blocks) => {
            yrs_engine::RenderUpdate::Full(blocks.clone())
        }
    }
}

pub(super) fn cached_render_operation_error(
    request_id: u64,
    resource_limits: &ResourceLimits,
    error: crate::render::incremental::CachedRenderError,
) -> yrs_engine::OperationError {
    match error {
        crate::render::incremental::CachedRenderError::ResourceLimitExceeded => {
            let limit = resource_limits.max_document_nodes.saturating_mul(3);
            yrs_engine::OperationError::document_limit_exceeded(
                request_id,
                None,
                "maxDocumentNodes",
                u64::try_from(limit).unwrap_or(u64::MAX),
                u64::try_from(limit.saturating_add(1)).unwrap_or(u64::MAX),
            )
        }
        crate::render::incremental::CachedRenderError::AllocationFailed
        | crate::render::incremental::CachedRenderError::PositionOverflow
        | crate::render::incremental::CachedRenderError::CacheInvariantViolation => {
            yrs_engine::OperationError::engine_invariant_failed(
                request_id,
                None,
                format!("cached render preparation failed: {error:?}"),
            )
        }
    }
}

impl YrsDocumentEngine {
    pub(super) fn prepare_commit_render_transition(
        &self,
        compiled: &CompiledTransaction,
    ) -> yrs_engine::OperationResult<crate::render::incremental::CachedRenderTransition> {
        let current = self
            .derived_state
            .as_ref()
            .ok_or_else(|| yrs_engine::OperationError::engine_not_ready(compiled.request_id))?;
        let generic_transition = || {
            current.render_blocks.transition(
                &current.document,
                &compiled.preview,
                &self.schema,
                &[],
                &self.resource_limits,
            )
        };
        let transition = if compiled.localized_semantic_used {
            crate::render::incremental::record_localized_render_transition_attempt();
            let specialized = compiled
                .prepared_derived_evidence
                .as_ref()
                .and_then(|evidence| {
                    evidence.prepare_localized_render_transition(
                        current,
                        &compiled.preview,
                        compiled.preview_derivations.as_ref()?,
                        &compiled.affected_top_level_blocks,
                        &self.schema,
                        &self.schema_fingerprint,
                        &self.resource_limits,
                        &self.editing_limits,
                        self.max_length,
                    )
                });
            match specialized {
                Some(Ok(transition)) => {
                    crate::render::incremental::record_localized_render_transition_success();
                    Ok(transition)
                }
                Some(Err(_)) => {
                    crate::render::incremental::record_localized_render_transition_fallback();
                    generic_transition()
                }
                None => {
                    crate::render::incremental::record_localized_render_transition_fallback();
                    generic_transition()
                }
            }
        } else {
            generic_transition()
        };
        transition.map_err(|error| {
            cached_render_operation_error(compiled.request_id, &self.resource_limits, error)
        })
    }
}
