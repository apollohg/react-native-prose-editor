use super::outbound::OutboundUpdateSink;
use super::YrsDocumentEngine;
use crate::yrs_engine;
use crate::yrs_engine::compiler::CompiledTransaction;
use yrs::{ReadTxn, Transact};

impl YrsDocumentEngine {
    pub(super) fn apply_prepared_command_transaction(
        &mut self,
        transaction: yrs_engine::TypedTransaction,
        proof: yrs_engine::commands::PreparedCommandProof,
        with_result: bool,
        outbound: &mut OutboundUpdateSink<'_>,
    ) -> yrs_engine::OperationResult<(
        yrs_engine::TransactionCommit,
        Option<yrs_engine::TypedTransactionResult>,
    )> {
        let request_id = transaction.request_id;
        let yrs_engine::commands::PreparedCommandProof {
            document,
            selection,
            execution_admission,
        } = proof;
        execution_admission.pre_admit_seed_independent(
            &transaction,
            &document,
            &self.editing_limits,
        )?;
        let prepare_history_before_context = self
            .derived_state
            .as_ref()
            .is_some_and(|state| state.mutation_lookup_seed.is_unavailable());
        let prepared_history = if prepare_history_before_context {
            self.prepare_execution_command_history_admission(&execution_admission)?
        } else {
            None
        };
        let requires_identity = execution_admission.requires_materialized_identity();
        let prepared_execution = yrs_engine::prepared_admission::PreparedExecutionAdmission::new(
            execution_admission,
            prepared_history,
        );
        let mut context = self.prepare_mutation_lookup_seed(request_id)?;
        if requires_identity {
            self.prepare_mutation_identity(&mut context)?;
        }

        let (execution_admission, prepared_history) = prepared_execution.into_parts();
        let compiled = {
            let state = self
                .derived_state
                .as_ref()
                .ok_or_else(|| yrs_engine::OperationError::engine_not_ready(request_id))?;
            let txn = self.doc.transact();
            let fragment = txn
                .get_xml_fragment(self.fragment_name.as_str())
                .ok_or_else(|| {
                    yrs_engine::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "ready engine lost its prepared-command fragment",
                    )
                })?;
            let authority = context.authority(
                yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                    request_id,
                    installed: state,
                    txn: &txn,
                    fragment: &fragment,
                    fragment_name: &self.fragment_name,
                    schema_fingerprint: &self.schema_fingerprint,
                    resource_limits: &self.resource_limits,
                    editing_limits: &self.editing_limits,
                    max_length: self.max_length,
                    document_revision: self.revision,
                    state_revision: self.state_revision,
                    yrs_state_epoch: self.yrs_state_epoch,
                },
            )?;
            let semantic_admission = match execution_admission {
                yrs_engine::prepared_admission::ExecutionSemanticAdmission::Eager(admission) => {
                    admission
                }
                yrs_engine::prepared_admission::ExecutionSemanticAdmission::Deferred(deferred) => {
                    yrs_engine::compiler::finalize_deferred_admission(
                        &authority,
                        deferred,
                        yrs_engine::compiler::PreparedSemanticLiveContext {
                            transaction: &transaction,
                            expected_preview: &document,
                            canonical_schema: &self.canonical_schema,
                        },
                    )?
                }
            };
            let compiled = self.compile_finalized_prepared_typed_transaction_with_read_view(
                transaction,
                &semantic_admission,
                &document,
                &selection,
                prepared_history
                    .as_ref()
                    .map(|history| &history.candidate_derivations),
                &authority,
                &txn,
                &fragment,
            )?;
            if !self.compiled_command_matches_proof(&compiled, &document, &selection)? {
                return Err(yrs_engine::OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "prepared command diverged during Yrs compilation",
                ));
            }
            compiled
        };
        self.apply_compiled_transaction_with_history_and_context(
            compiled,
            with_result,
            prepared_history,
            Some(context),
            outbound,
        )
    }

    pub(super) fn apply_typed_transaction_with_staged_context(
        &mut self,
        transaction: yrs_engine::TypedTransaction,
        with_result: bool,
        outbound: &mut OutboundUpdateSink<'_>,
    ) -> yrs_engine::OperationResult<(
        yrs_engine::TransactionCommit,
        Option<yrs_engine::TypedTransactionResult>,
    )> {
        if transaction.operations.is_empty() {
            if transaction.history_policy == yrs_engine::HistoryPolicy::Skip {
                return self.apply_empty_skip_transaction(transaction, with_result);
            }
            let compiled = self.compile_typed_transaction(transaction)?;
            return self.apply_compiled_transaction_with_history(
                compiled,
                with_result,
                None,
                outbound,
            );
        }
        let request_id = transaction.request_id;
        let requires_identity = matches!(
            transaction.operations.as_slice(),
            [yrs_engine::TypedOperation::InsertText { .. }]
        );
        let mut context = self.prepare_mutation_lookup_seed(request_id)?;
        if requires_identity {
            self.prepare_mutation_identity(&mut context)?;
        }
        let compiled = self.with_compiled_base_authority(
            request_id,
            Some(&context),
            |authority, txn, fragment| {
                self.compile_typed_transaction_with_read_view(
                    transaction,
                    None,
                    authority,
                    txn,
                    fragment,
                )
            },
        )?;
        self.apply_compiled_transaction_with_context(compiled, context, with_result, outbound)
    }

    #[allow(dead_code)]
    pub fn apply_typed_transaction(
        &mut self,
        transaction: yrs_engine::TypedTransaction,
    ) -> yrs_engine::OperationResult<yrs_engine::TransactionCommit> {
        let (commit, _) = self.apply_typed_transaction_with_staged_context(
            transaction,
            false,
            &mut OutboundUpdateSink::detached(),
        )?;
        Ok(commit)
    }

    /// Production surface: one typed transaction with an optionally attached
    /// collaboration outbox for outbound update capture. Returns the commit
    /// and, when `with_result` is set, the full typed result envelope.
    pub(crate) fn apply_typed_transaction_with_outbox(
        &mut self,
        transaction: yrs_engine::TypedTransaction,
        with_result: bool,
        outbox: Option<&mut crate::collaboration_runtime::CollaborationOutbox>,
    ) -> yrs_engine::OperationResult<(
        yrs_engine::TransactionCommit,
        Option<yrs_engine::TypedTransactionResult>,
    )> {
        self.apply_typed_transaction_with_staged_context(
            transaction,
            with_result,
            &mut OutboundUpdateSink::from_optional_outbox(outbox),
        )
    }

    #[allow(dead_code)]
    pub fn apply_typed_transaction_with_result(
        &mut self,
        transaction: yrs_engine::TypedTransaction,
    ) -> yrs_engine::OperationResult<yrs_engine::TypedTransactionResult> {
        let request_id = transaction.request_id;
        let (_, result) = self.apply_typed_transaction_with_staged_context(
            transaction,
            true,
            &mut OutboundUpdateSink::detached(),
        )?;
        result.ok_or_else(|| {
            yrs_engine::OperationError::engine_invariant_failed(
                request_id,
                None,
                "rich typed transaction produced no result envelope",
            )
        })
    }

    /// Production probe: the conservative outbound bound one typed transaction
    /// would reserve (compile only, no commit).
    #[allow(dead_code)]
    pub(crate) fn probe_transaction_outbound_upper_bound(
        &self,
        transaction: yrs_engine::TypedTransaction,
    ) -> yrs_engine::OperationResult<usize> {
        Ok(self
            .compile_typed_transaction(transaction)?
            .outbound_update_upper_bound())
    }

    #[cfg(test)]
    pub(super) fn apply_compiled_transaction(
        &mut self,
        compiled: CompiledTransaction,
        with_result: bool,
    ) -> yrs_engine::OperationResult<(
        yrs_engine::TransactionCommit,
        Option<yrs_engine::TypedTransactionResult>,
    )> {
        self.apply_compiled_transaction_with_history(
            compiled,
            with_result,
            None,
            &mut OutboundUpdateSink::detached(),
        )
    }

    fn apply_compiled_transaction_with_context(
        &mut self,
        compiled: CompiledTransaction,
        context: yrs_engine::prepared_admission::PreparedMutationContext,
        with_result: bool,
        outbound: &mut OutboundUpdateSink<'_>,
    ) -> yrs_engine::OperationResult<(
        yrs_engine::TransactionCommit,
        Option<yrs_engine::TypedTransactionResult>,
    )> {
        self.apply_compiled_transaction_with_history_and_context(
            compiled,
            with_result,
            None,
            Some(context),
            outbound,
        )
    }

    pub(super) fn apply_compiled_transaction_with_history(
        &mut self,
        compiled: CompiledTransaction,
        with_result: bool,
        prepared_history: Option<yrs_engine::prepared_admission::PreparedCommandHistoryAdmission>,
        outbound: &mut OutboundUpdateSink<'_>,
    ) -> yrs_engine::OperationResult<(
        yrs_engine::TransactionCommit,
        Option<yrs_engine::TypedTransactionResult>,
    )> {
        self.apply_compiled_transaction_with_history_and_context(
            compiled,
            with_result,
            prepared_history,
            None,
            outbound,
        )
    }
}
