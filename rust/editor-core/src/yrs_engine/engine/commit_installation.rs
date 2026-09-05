use super::candidate_cache::PreparedCandidateCache;
#[cfg(test)]
use super::test_hooks::mark_compiled_commit_durable_write_for_test;
use super::YrsDocumentEngine;
use crate::yrs_engine;
use crate::yrs_engine::derived_state::DerivedStateCache;
use crate::yrs_engine::mutation::{execute_mutation_plan, YrsMutationPlan};
use crate::yrs_engine::TransactionOrigin;
use std::collections::HashSet;
use yrs::Transact;

pub(super) enum PreparedCompiledHistory {
    Recorded(yrs_engine::history::PreparedRecordedHistoryAdmission),
    Excluded(yrs_engine::history::PreparedExcludedHistoryAdmission),
}

pub(super) struct PreparedCompiledCommit {
    pub(super) request_id: u64,
    pub(super) origin: TransactionOrigin,
    pub(super) history_policy: yrs_engine::HistoryPolicy,
    pub(super) history: Option<PreparedCompiledHistory>,
    pub(super) mutation_plan: Option<YrsMutationPlan>,
    pub(super) history_update: Vec<u8>,
    pub(super) history_after: Option<yrs_engine::history::HistoryLocalState>,
    pub(super) next_derived_state: Option<DerivedStateCache>,
    pub(super) next_durable_client_ids: HashSet<u64>,
    pub(super) next_document_revision: u64,
    pub(super) next_state_revision: u64,
    pub(super) next_yrs_state_epoch: u64,
    pub(super) publish_active_state_install: bool,
    pub(super) publish_active_state_drop: bool,
    pub(super) result: Option<yrs_engine::TypedTransactionResult>,
    pub(super) next_candidate_cache: Option<PreparedCandidateCache>,
}

impl YrsDocumentEngine {
    pub(super) fn execute_prepared_yrs_write(&mut self, prepared: &mut PreparedCompiledCommit) {
        let yrs_origin = match prepared
            .history
            .as_ref()
            .expect("changed commit has prepared history execution")
        {
            PreparedCompiledHistory::Recorded(admission) => admission.yrs_origin(),
            PreparedCompiledHistory::Excluded(admission) => admission.yrs_origin(),
        };
        #[cfg(test)]
        mark_compiled_commit_durable_write_for_test();
        if matches!(prepared.history, Some(PreparedCompiledHistory::Recorded(_))) {
            let Some(PreparedCompiledHistory::Recorded(admission)) = prepared.history.take() else {
                unreachable!()
            };
            self.history.begin_prepared_recorded(admission);
        }
        let mutation_plan = prepared
            .mutation_plan
            .take()
            .expect("changed commit owns one deterministic mutation plan");
        let mut txn = self.doc.transact_mut_with(yrs_origin);
        execute_mutation_plan(mutation_plan, &mut txn);
    }

    pub(super) fn install_prepared_changed_commit(
        &mut self,
        mut prepared: PreparedCompiledCommit,
    ) -> (
        yrs_engine::TransactionCommit,
        Option<yrs_engine::TypedTransactionResult>,
    ) {
        if let Some(after) = prepared.history_after.take() {
            self.history.finish_capture(after, prepared.history_update);
        } else {
            let Some(PreparedCompiledHistory::Excluded(admission)) = prepared.history.take() else {
                unreachable!("excluded changed commit retains its prepared admission")
            };
            self.history
                .finish_prepared_excluded(admission, prepared.history_update);
            if prepared.history_policy == yrs_engine::HistoryPolicy::Boundary {
                self.history.force_next_capture_boundary();
            }
        }
        let next_derived_state = prepared
            .next_derived_state
            .take()
            .expect("changed commit owns prepared derived state");
        #[cfg(test)]
        let installed_unavailable_lookup_seed = next_derived_state
            .mutation_lookup_seed
            .is_unavailable_for_test();
        self.derived_state = Some(next_derived_state);
        if prepared.publish_active_state_install {
            yrs_engine::derived_state::record_active_state_cache_install();
        }
        if prepared.publish_active_state_drop {
            yrs_engine::derived_state::record_active_state_cache_drop();
        }
        #[cfg(test)]
        if installed_unavailable_lookup_seed {
            yrs_engine::mutation::record_unavailable_lookup_seed_install_for_test();
        }
        self.durable_client_ids = prepared.next_durable_client_ids;
        self.revision = prepared.next_document_revision;
        self.state_revision = prepared.next_state_revision;
        self.yrs_state_epoch = prepared.next_yrs_state_epoch;
        self.last_committed_origin = Some(prepared.origin);
        self.document_origin = prepared.origin.into();
        self.prepared_candidate_cache = prepared.next_candidate_cache.take();
        let commit = yrs_engine::TransactionCommit {
            request_id: prepared.request_id,
            changed: true,
            document_revision: prepared.next_document_revision,
            state_revision: prepared.next_state_revision,
            origin: prepared.origin,
        };
        (commit, prepared.result)
    }
}
