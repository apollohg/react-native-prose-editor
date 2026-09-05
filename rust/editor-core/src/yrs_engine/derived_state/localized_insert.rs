use super::insert_admission::{LocalizedInsertAdmission, LocalizedInsertAdmissionRequest};
use super::localized_index::{canonical_marks_sha256, node_path_sha256};
#[cfg(test)]
use super::observability::LOCALIZED_INSERT_ADMISSION_WORK;
use super::DerivedStateCache;
use crate::boundary::ResourceLimits;
#[cfg(test)]
use crate::model::Mark;
#[cfg(test)]
use crate::schema::Schema;
use crate::yrs_engine;
use crate::yrs_engine::prepared_admission::DerivedStateAuthority;
use crate::yrs_engine::{scalar_offset_to_utf16, ResolvedPoint, ResolvedSelection};
use sha2::Digest;
use std::sync::Arc;
use yrs::types::xml::XmlFragmentRef;
use yrs::ReadTxn;

impl DerivedStateCache {
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn localized_insert_admission_for_test(
        &self,
        document_position: u32,
        text: &str,
        marks: &[Mark],
        schema: &Schema,
        resource_limits: &ResourceLimits,
        max_length: Option<u32>,
        yrs_state_epoch: u64,
    ) -> Option<LocalizedInsertAdmission> {
        let schema_fingerprint = crate::schema::schema_fingerprint(schema);
        self.build_localized_insert_admission(
            LocalizedInsertAdmissionRequest {
                request_id: 0,
                base_document_revision: self.document_revision,
                origin: yrs_engine::TransactionOrigin::LocalInput,
                inserted_at: yrs_engine::RevisionedPosition {
                    offset: document_position,
                    kind: yrs_engine::EditorOffsetKind::Scalar,
                    affinity: yrs_engine::Affinity::After,
                },
                document_position,
                text,
                marks,
                selection_intent: yrs_engine::SelectionIntent::UseOperationResult,
                history_policy: yrs_engine::HistoryPolicy::Auto,
            },
            &schema_fingerprint,
            resource_limits,
            &crate::yrs_engine::EditingLimits::default(),
            max_length,
            yrs_state_epoch,
            &self.mutation_lookup_seed,
            None,
        )
    }

    /// Callers may invoke this only after envelope admission, cached-view
    /// validation, document-byte charging, and Yrs scan admission.
    #[allow(dead_code)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn admit_existing_text_insert<T: ReadTxn>(
        &self,
        transaction: &yrs_engine::TypedTransaction,
        allow_prepared_command_boundary: bool,
        document_position: u32,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema_fingerprint: &str,
        resource_limits: &ResourceLimits,
        editing_limits: &yrs_engine::EditingLimits,
        max_length: Option<u32>,
        yrs_state_epoch: u64,
    ) -> Option<LocalizedInsertAdmission> {
        let authority = yrs_engine::prepared_admission::InstalledDerivedStateAuthority::new(self);
        let lookup_seed =
            DerivedStateAuthority::lookup_seed(&authority, transaction.request_id).ok()?;
        self.admit_existing_text_insert_with_authority(
            transaction,
            allow_prepared_command_boundary,
            document_position,
            txn,
            fragment,
            lookup_seed,
            authority.materialized_identity(),
            schema_fingerprint,
            resource_limits,
            editing_limits,
            max_length,
            yrs_state_epoch,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn admit_existing_text_insert_with_authority<T: ReadTxn>(
        &self,
        transaction: &yrs_engine::TypedTransaction,
        allow_prepared_command_boundary: bool,
        document_position: u32,
        txn: &T,
        fragment: &XmlFragmentRef,
        lookup_seed: &Arc<yrs_engine::mutation::MutationLookupSeed>,
        identity: Option<&yrs_engine::prepared_admission::MaterializedMutationIdentity>,
        schema_fingerprint: &str,
        resource_limits: &ResourceLimits,
        editing_limits: &yrs_engine::EditingLimits,
        max_length: Option<u32>,
        yrs_state_epoch: u64,
    ) -> Option<LocalizedInsertAdmission> {
        if transaction.base_document_revision != self.document_revision
            || transaction.selection_intent != yrs_engine::SelectionIntent::UseOperationResult
            || !(transaction.history_policy == yrs_engine::HistoryPolicy::Auto
                || (allow_prepared_command_boundary
                    && transaction.origin == yrs_engine::TransactionOrigin::LocalCommand
                    && transaction.history_policy == yrs_engine::HistoryPolicy::Boundary))
            || !matches!(
                transaction.origin,
                yrs_engine::TransactionOrigin::LocalInput
                    | yrs_engine::TransactionOrigin::LocalCommand
                    | yrs_engine::TransactionOrigin::LocalApi
            )
            || !self
                .render_blocks
                .matches_identity(&self.document, &self.schema_fingerprint)
            || !lookup_seed.matches(
                txn,
                fragment,
                &self.document,
                resource_limits,
                editing_limits,
                max_length,
                &self.schema_fingerprint,
                yrs_state_epoch,
                self.document_revision,
            )
        {
            return None;
        }
        let [yrs_engine::TypedOperation::InsertText { at, text, marks }] =
            transaction.operations.as_slice()
        else {
            return None;
        };
        self.build_localized_insert_admission(
            LocalizedInsertAdmissionRequest {
                request_id: transaction.request_id,
                base_document_revision: transaction.base_document_revision,
                origin: transaction.origin,
                inserted_at: *at,
                document_position,
                text,
                marks,
                selection_intent: transaction.selection_intent.clone(),
                history_policy: transaction.history_policy,
            },
            schema_fingerprint,
            resource_limits,
            editing_limits,
            max_length,
            yrs_state_epoch,
            lookup_seed,
            identity,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn build_localized_insert_admission(
        &self,
        request: LocalizedInsertAdmissionRequest<'_>,
        schema_fingerprint: &str,
        resource_limits: &ResourceLimits,
        editing_limits: &yrs_engine::EditingLimits,
        max_length: Option<u32>,
        yrs_state_epoch: u64,
        lookup_seed: &Arc<yrs_engine::mutation::MutationLookupSeed>,
        identity: Option<&yrs_engine::prepared_admission::MaterializedMutationIdentity>,
    ) -> Option<LocalizedInsertAdmission> {
        #[cfg(test)]
        LOCALIZED_INSERT_ADMISSION_WORK
            .set(LOCALIZED_INSERT_ADMISSION_WORK.get().saturating_add(1));
        let LocalizedInsertAdmissionRequest {
            request_id,
            base_document_revision,
            origin,
            inserted_at,
            document_position,
            text,
            marks,
            selection_intent,
            history_policy,
        } = request;
        let identity_matches = identity.is_none_or(|identity| {
            self.matches_materialized_mutation_identity(
                &self.canonical_artifact,
                identity.canonical_fingerprint,
                identity.canonical_serialized_len,
                resource_limits,
                &self.schema_fingerprint,
                self.document_revision,
                self.state_revision,
                yrs_state_epoch,
            )
        });
        if text.is_empty()
            || schema_fingerprint != self.schema_fingerprint
            || !identity_matches
            || (identity.is_none()
                && !self.validation_certificate.matches(
                    &self.canonical_artifact,
                    resource_limits,
                    &self.schema_fingerprint,
                    self.document_revision,
                    self.state_revision,
                    yrs_state_epoch,
                ))
        {
            return None;
        }
        let localized_text_index = self.localized_text_index.as_ref()?;
        if identity.is_none() && !localized_text_index.matches(&self.validation_certificate) {
            return None;
        }
        let leaf = localized_text_index
            .strict_inside(document_position)
            .copied()?;
        let block = self.position_map.block(leaf.block_index)?;
        let affected_top_level_index = usize::try_from(*block.node_path.first()?).ok()?;
        let live_leaf = leaf.resolve(&self.document, &self.position_map)?;
        let live_text = live_leaf.text_str()?;
        if <[u8; 32]>::from(sha2::Sha256::digest(live_text.as_bytes())) != leaf.text_sha256
            || canonical_marks_sha256(live_leaf.marks())? != leaf.marks_sha256
            || live_leaf.marks() != marks
            || live_leaf.node_size() != leaf.text_scalars
            || u32::try_from(live_leaf.text_str()?.encode_utf16().count()).ok()? != leaf.text_utf16
            || live_leaf.text_str()?.len() != leaf.text_utf8_bytes
        {
            return None;
        }
        let inserted_scalars = u32::try_from(text.chars().count()).ok()?;
        let inserted_utf16 = u32::try_from(text.encode_utf16().count()).ok()?;
        let next_raw_text_scalars = self
            .validation_certificate
            .raw_text_scalars
            .checked_add(u64::from(inserted_scalars))?;
        if max_length.is_some_and(|limit| next_raw_text_scalars > u64::from(limit)) {
            return None;
        }
        let next_raw_text_utf8_bytes = self
            .validation_certificate
            .raw_text_utf8_bytes
            .checked_add(text.len())?;
        let canonical_serialized_len = identity.map_or(
            self.validation_certificate.canonical_serialized_len,
            |identity| identity.canonical_serialized_len,
        );
        let canonical_fingerprint = identity.map_or_else(
            || self.validation_certificate.canonical_fingerprint,
            |identity| identity.canonical_fingerprint,
        );
        let escaped_limit = editing_limits
            .max_derived_output_bytes
            .checked_sub(canonical_serialized_len)?;
        let inserted_escaped_json_bytes = checked_json_string_body_len(text, escaped_limit)?;
        let next_canonical_serialized_len =
            canonical_serialized_len.checked_add(inserted_escaped_json_bytes)?;
        if next_canonical_serialized_len > editing_limits.max_derived_output_bytes {
            return None;
        }
        let scalar_at = self
            .position_map
            .doc_to_scalar(document_position, &self.document);
        let utf16_at = scalar_offset_to_utf16(&self.rendered_text, scalar_at)?;
        let next_document = document_position.checked_add(inserted_scalars)?;
        let next_scalar = scalar_at.checked_add(inserted_scalars)?;
        let next_utf16 = utf16_at.checked_add(inserted_utf16)?;
        let operation_result = ResolvedSelection::Text {
            anchor: ResolvedPoint {
                document: next_document,
                scalar: next_scalar,
                utf16: next_utf16,
            },
            head: ResolvedPoint {
                document: next_document,
                scalar: next_scalar,
                utf16: next_utf16,
            },
        };
        let history_undo_units = u64::from(inserted_utf16);
        if history_undo_units > editing_limits.max_undo_retained_units {
            return None;
        }
        Some(LocalizedInsertAdmission {
            leaf,
            block_path_len: block.node_path.len(),
            block_path_sha256: node_path_sha256(&block.node_path),
            affected_top_level_index,
            inserted_scalars,
            inserted_utf8_bytes: text.len(),
            inserted_utf16,
            inserted_escaped_json_bytes,
            next_raw_text_scalars,
            next_raw_text_utf8_bytes,
            next_canonical_serialized_len,
            next_rendered_scalars: self.rendered_scalars.checked_add(inserted_scalars)?,
            operation_result,
            history_undo_units,
            document_revision: self.document_revision,
            state_revision: self.state_revision,
            yrs_state_epoch,
            selection: self.resolved_selection.clone(),
            relative_selection: self.relative_selection.clone(),
            stored_marks_sha256: match self.stored_marks.as_deref() {
                Some(stored_marks) => Some(canonical_marks_sha256(stored_marks)?),
                None => None,
            },
            canonical_fingerprint,
            validation_certificate: self.validation_certificate.clone(),
            request_id,
            base_document_revision,
            origin,
            inserted_at,
            inserted_document_position: document_position,
            inserted_text_sha256: sha2::Sha256::digest(text.as_bytes()).into(),
            inserted_marks_sha256: canonical_marks_sha256(marks)?,
            selection_intent,
            history_policy,
            max_length,
            max_operations_per_transaction: editing_limits.max_operations_per_transaction,
            max_undo_groups: editing_limits.max_undo_groups,
            max_derived_output_bytes: editing_limits.max_derived_output_bytes,
            max_undo_retained_units: editing_limits.max_undo_retained_units,
            render_seal: Arc::clone(&self.render_blocks),
            lookup_seal: Arc::clone(lookup_seed),
        })
    }
}

pub(super) fn checked_json_string_body_len(text: &str, limit: usize) -> Option<usize> {
    let mut bytes = 0usize;
    for character in text.chars() {
        let amount = match character {
            '"' | '\\' | '\u{0008}' | '\u{000c}' | '\n' | '\r' | '\t' => 2,
            '\u{0000}'..='\u{001f}' => 6,
            other => other.len_utf8(),
        };
        bytes = bytes.checked_add(amount)?;
        if bytes > limit {
            return None;
        }
    }
    Some(bytes)
}
