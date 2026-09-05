use super::commit::CompiledCommitAuthority;
use super::{checked_operation_increment, YrsDocumentEngine};
use crate::selection::Selection;
use crate::yrs_engine;
use crate::yrs_engine::compiler::{
    selectable_void_at, CompilationContext, CompiledTransaction, RelativeSelectionPlan,
    SelectionPlan, StoredMarksPlan,
};
use crate::yrs_engine::derived_state::{
    operation_result_to_relative, stored_marks_after_selection_change, DerivedStateCache,
};
use yrs::{ReadTxn, StateVector, Transact};

impl YrsDocumentEngine {
    pub(super) fn apply_empty_skip_transaction(
        &mut self,
        transaction: yrs_engine::TypedTransaction,
        with_result: bool,
    ) -> yrs_engine::OperationResult<(
        yrs_engine::TransactionCommit,
        Option<yrs_engine::TypedTransactionResult>,
    )> {
        debug_assert!(transaction.operations.is_empty());
        debug_assert_eq!(transaction.history_policy, yrs_engine::HistoryPolicy::Skip);
        let request_id = transaction.request_id;
        let current = self
            .derived_state
            .as_ref()
            .ok_or_else(|| yrs_engine::OperationError::engine_not_ready(request_id))?;
        let context = CompilationContext {
            document: &current.document,
            selection: None,
            schema: &self.schema,
            resource_limits: &self.resource_limits,
            editing_limits: &self.editing_limits,
            document_revision: self.revision,
            max_length: self.max_length,
        };
        #[cfg(test)]
        yrs_engine::compiler::check_atomic_failpoint(
            request_id,
            yrs_engine::compiler::AtomicFailpoint::EnvelopeAdmission,
        )?;
        let admitted_input_bytes =
            yrs_engine::compiler::admit_transaction_envelope(context, &transaction)?;
        #[cfg(test)]
        yrs_engine::compiler::check_atomic_failpoint(
            request_id,
            yrs_engine::compiler::AtomicFailpoint::SemanticCompilation,
        )?;

        let txn = self.doc.transact();
        yrs_engine::compiler::admit_yrs_scan_work(
            request_id,
            admitted_input_bytes,
            current.document_text_bytes,
            &txn,
            &self.resource_limits,
        )?;
        let needs_rendered_text = match &transaction.selection_intent {
            yrs_engine::SelectionIntent::Set(yrs_engine::SelectionInput::Text { anchor, head }) => {
                anchor.kind == yrs_engine::EditorOffsetKind::Utf16
                    || head.kind == yrs_engine::EditorOffsetKind::Utf16
            }
            yrs_engine::SelectionIntent::Set(yrs_engine::SelectionInput::Node { at }) => {
                at.kind == yrs_engine::EditorOffsetKind::Utf16
            }
            _ => false,
        };
        let rendered_text = if needs_rendered_text {
            current.rendered_text.as_str()
        } else {
            ""
        };
        let fragment = txn
            .get_xml_fragment(self.fragment_name.as_str())
            .ok_or_else(|| {
                yrs_engine::OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "ready Yrs document fragment is missing",
                )
            })?;
        let resolve_point = |field: &'static str,
                             point: yrs_engine::RevisionedPosition|
         -> yrs_engine::OperationResult<u32> {
            yrs_engine::position::editor_offset_to_doc_pos(
                point.offset,
                point.kind,
                rendered_text,
                &current.position_map,
                &current.document,
            )
            .ok_or_else(|| {
                yrs_engine::OperationError::selection_position_invalid(
                    request_id,
                    field,
                    format!("{field} is outside the base document"),
                )
            })
        };
        let relative_point = |field: &'static str,
                              point: yrs_engine::RevisionedPosition|
         -> yrs_engine::OperationResult<yrs_engine::RelativePoint> {
            let document_position = resolve_point(field, point)?;
            yrs_engine::position::doc_pos_to_relative_point(
                &txn,
                &fragment,
                document_position,
                point.affinity,
                &self.schema,
            )
            .ok_or_else(|| {
                yrs_engine::OperationError::selection_position_invalid(
                    request_id,
                    field,
                    "selection cannot be represented with the requested Yrs affinity",
                )
            })
        };
        let mut prepared_next_selection = None;
        let next_relative = match &transaction.selection_intent {
            yrs_engine::SelectionIntent::Preserve
            | yrs_engine::SelectionIntent::UseOperationResult => current.relative_selection.clone(),
            yrs_engine::SelectionIntent::Set(yrs_engine::SelectionInput::Text { anchor, head }) => {
                let anchor_document = resolve_point("selection.anchor", *anchor)?;
                let head_document = if anchor == head {
                    anchor_document
                } else {
                    resolve_point("selection.head", *head)?
                };
                let normalized = Selection::text(anchor_document, head_document)
                    .normalized(&current.document, &current.position_map);
                debug_assert!(matches!(normalized, Selection::Text { .. }));
                let prepared_collapsed = if anchor == head {
                    let Selection::Text {
                        anchor: normalized_anchor,
                        head: normalized_head,
                    } = normalized
                    else {
                        unreachable!("text selection normalized to a non-text selection")
                    };
                    (normalized_anchor == anchor_document
                        && normalized_head == head_document
                        && normalized_anchor == normalized_head)
                        .then(|| {
                            let relative =
                                yrs_engine::position::admitted_doc_pos_to_relative_point(
                                    &txn,
                                    &fragment,
                                    normalized_anchor,
                                    anchor.affinity,
                                    &self.schema,
                                )?;
                            let scalar = current
                                .position_map
                                .doc_to_scalar(normalized_anchor, &current.document);
                            let utf16 = yrs_engine::position::scalar_offset_to_utf16(
                                &current.rendered_text,
                                scalar,
                            )?;
                            let resolved = yrs_engine::ResolvedPoint {
                                document: normalized_anchor,
                                scalar,
                                utf16,
                            };
                            Some((relative, resolved))
                        })
                        .flatten()
                } else {
                    None
                };
                if let Some((point, resolved)) = prepared_collapsed {
                    prepared_next_selection = Some(yrs_engine::ResolvedSelection::Text {
                        anchor: resolved,
                        head: resolved,
                    });
                    yrs_engine::RelativeSelection::Text {
                        anchor: point.clone(),
                        head: point,
                    }
                } else {
                    yrs_engine::RelativeSelection::Text {
                        anchor: relative_point("selection.anchor", *anchor)?,
                        head: relative_point("selection.head", *head)?,
                    }
                }
            }
            yrs_engine::SelectionIntent::Set(yrs_engine::SelectionInput::Node { at }) => {
                let document_position = resolve_point("selection.at", *at)?;
                let normalized = Selection::node(document_position)
                    .normalized(&current.document, &current.position_map);
                let Selection::Node { pos } = normalized else {
                    return Err(yrs_engine::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "node selection did not compile to a node selection",
                    ));
                };
                if !selectable_void_at(current.document.root(), pos, 0, &self.schema) {
                    return Err(yrs_engine::OperationError::selection_position_invalid(
                        request_id,
                        "selection.at",
                        "node selection must target a selectable void or atom node",
                    ));
                }
                yrs_engine::RelativeSelection::Node {
                    point: relative_point("selection.at", *at)?,
                }
            }
            yrs_engine::SelectionIntent::Set(yrs_engine::SelectionInput::All) => {
                yrs_engine::RelativeSelection::All
            }
        };
        let next_selection = match prepared_next_selection {
            Some(selection) => selection,
            None => current
                .resolve_relative_selection(&next_relative, &txn, &fragment, &self.schema)
                .ok_or_else(|| {
                    yrs_engine::OperationError::selection_position_invalid(
                        request_id,
                        "selection",
                        "selection cannot be represented in the Yrs document",
                    )
                })?,
        };
        drop(txn);
        let next_stored_marks = stored_marks_after_selection_change(
            current.stored_marks.as_deref(),
            &current.resolved_selection,
            &next_selection,
            &current.document,
            &self.schema,
        );
        let changed = next_relative != current.relative_selection
            || next_selection != current.resolved_selection
            || next_stored_marks != current.stored_marks;
        let next_state_revision = if changed {
            checked_operation_increment(request_id, self.state_revision, "stateRevision")?
        } else {
            self.state_revision
        };
        let result = with_result
            .then(|| {
                self.prepare_empty_skip_result(
                    request_id,
                    transaction.origin,
                    &next_selection,
                    next_stored_marks.as_deref(),
                    changed,
                    next_state_revision,
                )
            })
            .transpose()?;

        if changed {
            let current = self.derived_state.as_mut().ok_or_else(|| {
                yrs_engine::OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "ready Yrs engine lost derived state during selection admission",
                )
            })?;
            current.update_selection_state(
                next_relative,
                next_selection,
                next_stored_marks,
                next_state_revision,
            );
            self.state_revision = next_state_revision;
            self.last_committed_origin = Some(transaction.origin);
        }
        let commit = yrs_engine::TransactionCommit {
            request_id,
            changed,
            document_revision: self.revision,
            state_revision: self.state_revision,
            origin: transaction.origin,
        };
        Ok((commit, result))
    }
}

pub(super) struct SelectionCommitContext<'a> {
    pub(super) current: Option<&'a DerivedStateCache>,
    pub(super) schema: &'a crate::schema::Schema,
    pub(super) history: &'a mut yrs_engine::history::YrsHistory,
    pub(super) document_revision: u64,
    pub(super) state_revision: u64,
}

pub(super) struct PreparedSelectionCommit {
    pub(super) state: Option<(DerivedStateCache, u64)>,
    pub(super) boundary: Option<yrs_engine::history::PreparedBoundary>,
}

impl YrsDocumentEngine {
    pub(super) fn prepare_selection_commit(
        context: SelectionCommitContext<'_>,
        compiled: &CompiledTransaction,
        commit_authority: &CompiledCommitAuthority<'_, '_>,
        had_active_state_certificate: bool,
    ) -> yrs_engine::OperationResult<PreparedSelectionCommit> {
        let boundary_state =
            (compiled.history_policy == yrs_engine::HistoryPolicy::Boundary).then(|| {
                if commit_authority.state_vector().is_empty() {
                    Vec::new()
                } else {
                    commit_authority
                        .txn()
                        .encode_state_as_update_v1(&StateVector::default())
                }
            });
        let current = context.current.ok_or_else(|| {
            yrs_engine::OperationError::engine_invariant_failed(
                compiled.request_id,
                None,
                "ready Yrs engine has no derived state",
            )
        })?;
        let (next_relative_selection, next_resolved_selection) =
            if matches!(compiled.selection_plan, SelectionPlan::Preserve) {
                (
                    current.relative_selection.clone(),
                    current.resolved_selection.clone(),
                )
            } else {
                let selection = match &compiled.selection_plan {
                    SelectionPlan::Explicit(selection) | SelectionPlan::Mapped(selection) => {
                        selection
                    }
                    SelectionPlan::Preserve => unreachable!(),
                };
                let planned_relative_selection = match &compiled.relative_selection_plan {
                    RelativeSelectionPlan::Precomputed { relative, .. } => relative.clone(),
                    RelativeSelectionPlan::OperationResult => operation_result_to_relative(
                        commit_authority.txn(),
                        commit_authority.fragment(),
                        selection,
                        context.schema,
                    ),
                    RelativeSelectionPlan::Unsealed
                    | RelativeSelectionPlan::Preserve
                    | RelativeSelectionPlan::PreserveWithFallback(_) => {
                        return Err(yrs_engine::OperationError::engine_invariant_failed(
                            compiled.request_id,
                            None,
                            "selection-only transaction has no materializable relative selection",
                        ));
                    }
                };
                let resolved_selection = current
                    .resolve_relative_selection(
                        &planned_relative_selection,
                        commit_authority.txn(),
                        commit_authority.fragment(),
                        context.schema,
                    )
                    .ok_or_else(|| {
                        yrs_engine::OperationError::selection_position_invalid(
                            compiled.request_id,
                            "selection",
                            "selection cannot be represented in the Yrs document",
                        )
                    })?;
                (planned_relative_selection, resolved_selection)
            };
        let StoredMarksPlan::Set(planned_stored_marks) = &compiled.stored_marks_plan else {
            unreachable!()
        };
        let next_stored_marks = planned_stored_marks.clone();
        let state_changed = next_relative_selection != current.relative_selection
            || next_resolved_selection != current.resolved_selection
            || next_stored_marks != current.stored_marks;
        let next_state_revision = state_changed
            .then(|| {
                checked_operation_increment(
                    compiled.request_id,
                    context.state_revision,
                    "stateRevision",
                )
            })
            .transpose()?;
        let prepared_boundary = boundary_state
            .map(|encoded| {
                context
                    .history
                    .prepare_boundary(compiled.request_id, encoded)
            })
            .transpose()?;
        if !state_changed {
            return Ok(PreparedSelectionCommit {
                state: None,
                boundary: prepared_boundary,
            });
        }
        let next_state_revision =
            next_state_revision.expect("changed state has an admitted next revision");
        debug_assert_eq!(current.document_revision, context.document_revision);
        let mut next = current.clone_with_fallible_localized_index();
        if had_active_state_certificate {
            yrs_engine::derived_state::record_active_state_cache_drop();
        }
        next.update_selection_state(
            next_relative_selection,
            next_resolved_selection,
            next_stored_marks,
            next_state_revision,
        );
        Ok(PreparedSelectionCommit {
            state: Some((next, next_state_revision)),
            boundary: prepared_boundary,
        })
    }

    pub(super) fn install_selection_commit(
        &mut self,
        compiled: &CompiledTransaction,
        prepared: PreparedSelectionCommit,
        mut result: Option<yrs_engine::TypedTransactionResult>,
    ) -> yrs_engine::OperationResult<(
        yrs_engine::TransactionCommit,
        Option<yrs_engine::TypedTransactionResult>,
    )> {
        let PreparedSelectionCommit {
            state,
            boundary: prepared_boundary,
        } = prepared;
        let Some((next, next_state_revision)) = state else {
            if let Some(prepared) = prepared_boundary {
                self.derived_state
                    .as_mut()
                    .expect("history boundary retains derived state")
                    .clear_active_state_certificate();
                self.history.commit_boundary(prepared);
            }
            let commit = yrs_engine::TransactionCommit {
                request_id: compiled.request_id,
                changed: false,
                document_revision: self.revision,
                state_revision: self.state_revision,
                origin: compiled.origin,
            };
            if let Some(result) = &mut result {
                result.changed = false;
                result.document_revision = self.revision;
                result.state_revision = self.state_revision;
                result.history_state = crate::editor_state::HistoryState {
                    can_undo: self.can_undo(),
                    can_redo: self.can_redo(),
                };
            }
            return Ok((commit, result));
        };
        self.derived_state = Some(next);
        self.state_revision = next_state_revision;
        self.last_committed_origin = Some(compiled.origin);
        if let Some(prepared) = prepared_boundary {
            self.history.commit_boundary(prepared);
        }
        let commit = yrs_engine::TransactionCommit {
            request_id: compiled.request_id,
            changed: true,
            document_revision: self.revision,
            state_revision: self.state_revision,
            origin: compiled.origin,
        };
        if let Some(result) = &mut result {
            result.changed = true;
            result.document_revision = self.revision;
            result.state_revision = self.state_revision;
            result.selection = self
                .derived_state
                .as_ref()
                .expect("selection-only result retains derived state")
                .resolved_selection
                .clone();
            result.history_state = crate::editor_state::HistoryState {
                can_undo: self.can_undo(),
                can_redo: self.can_redo(),
            };
        }
        return Ok((commit, result));
    }
}
