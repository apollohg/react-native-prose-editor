use super::insert_admission::LocalizedInsertAdmission;
use super::observability::{
    record_active_state_cache_drop, record_active_state_public_result_clone,
};
#[cfg(test)]
use super::observability::{
    FORCE_ACTIVE_STATE_CACHE_ALLOCATION_FAILURE, FORCE_ACTIVE_STATE_CACHE_BUDGET,
    FORCE_ACTIVE_STATE_PUBLIC_MATERIALIZATION_FAILURE,
};
use super::validation::DocumentValidationCertificate;
use super::DerivedStateCache;
use crate::boundary::ResourceLimits;
use crate::editor_state::ActiveState;
use crate::model::{Document, Mark};
use crate::selection::Selection;
use crate::yrs_engine;
use crate::yrs_engine::canonical::CanonicalArtifact;
use crate::yrs_engine::prepared_admission::DerivedStateAuthority;
use crate::yrs_engine::{OperationResult, RelativeSelection, ResolvedSelection};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActiveStateStructuralSeal {
    pub(super) block_index: usize,
    pub(super) child_ordinal: u32,
    pub(super) leaf_doc_start: u32,
    pub(super) leaf_marks_sha256: [u8; 32],
    pub(super) block_path_len: usize,
    pub(super) block_path_sha256: [u8; 32],
    pub(super) affected_top_level_index: usize,
}

#[derive(Debug)]
pub(crate) struct CachedActiveState {
    pub(super) value: ActiveState,
    pub(super) retained_bytes: usize,
}

/// Deterministic deep retained-budget measure. It counts owned container slot
/// capacity and recursive string/JSON heap payload with checked arithmetic;
/// allocator-specific HashMap control bytes and global allocator bookkeeping
/// are intentionally outside this portable configured limit.
pub(super) struct ActiveStateRetainedMeter<'a> {
    pub(super) limits: &'a ResourceLimits,
    pub(super) bytes: usize,
    pub(super) items: usize,
}

impl ActiveStateRetainedMeter<'_> {
    pub(super) fn add_bytes(&mut self, amount: usize) -> Option<()> {
        self.bytes = self.bytes.checked_add(amount)?;
        (self.bytes <= self.limits.max_input_bytes).then_some(())
    }

    pub(super) fn add_items(&mut self, amount: usize) -> Option<()> {
        self.items = self.items.checked_add(amount)?;
        (self.items <= self.limits.max_document_nodes).then_some(())
    }

    pub(super) fn string_heap(&mut self, value: &String) -> Option<()> {
        self.add_items(1)?;
        self.add_bytes(value.capacity())
    }

    pub(super) fn json_heap(&mut self, value: &serde_json::Value, depth: usize) -> Option<()> {
        let mut pending = vec![(value, depth)];
        while let Some((value, depth)) = pending.pop() {
            if depth > self.limits.max_document_depth {
                return None;
            }
            self.add_items(1)?;
            match value {
                serde_json::Value::Null
                | serde_json::Value::Bool(_)
                | serde_json::Value::Number(_) => {}
                serde_json::Value::String(value) => self.add_bytes(value.capacity())?,
                serde_json::Value::Array(values) => {
                    self.add_bytes(
                        values
                            .capacity()
                            .checked_mul(std::mem::size_of::<serde_json::Value>())?,
                    )?;
                    let child_depth = depth.checked_add(1)?;
                    pending.extend(values.iter().map(|value| (value, child_depth)));
                }
                serde_json::Value::Object(values) => {
                    self.add_bytes(
                        values
                            .len()
                            .checked_mul(std::mem::size_of::<(String, serde_json::Value)>())?,
                    )?;
                    let child_depth = depth.checked_add(1)?;
                    for (key, value) in values {
                        self.string_heap(key)?;
                        pending.push((value, child_depth));
                    }
                }
            }
        }
        Some(())
    }
}

pub(super) fn active_state_retained_bytes(
    state: &ActiveState,
    resource_limits: &ResourceLimits,
) -> Option<usize> {
    let mut meter = ActiveStateRetainedMeter {
        limits: resource_limits,
        bytes: 0,
        items: 0,
    };
    meter.add_bytes(std::mem::size_of::<ActiveState>())?;
    for map in [&state.marks, &state.nodes, &state.commands] {
        meter.add_items(map.len())?;
        meter.add_bytes(
            map.capacity()
                .checked_mul(std::mem::size_of::<(String, bool)>())?,
        )?;
        for key in map.keys() {
            meter.string_heap(key)?;
        }
    }
    meter.add_items(state.mark_attrs.len())?;
    meter.add_bytes(
        state
            .mark_attrs
            .capacity()
            .checked_mul(std::mem::size_of::<(String, serde_json::Value)>())?,
    )?;
    for (key, value) in &state.mark_attrs {
        meter.string_heap(key)?;
        meter.json_heap(value, 1)?;
    }
    for strings in [&state.allowed_marks, &state.insertable_nodes] {
        meter.add_items(strings.len())?;
        meter.add_bytes(
            strings
                .capacity()
                .checked_mul(std::mem::size_of::<String>())?,
        )?;
        for value in strings {
            meter.string_heap(value)?;
        }
    }
    Some(meter.bytes)
}

impl CachedActiveState {
    // Returning the original owned state makes optional-cache failure
    // allocation-free; boxing this large Err would undermine that property.
    #[allow(clippy::result_large_err)]
    pub(crate) fn try_new(
        value: ActiveState,
        resource_limits: &ResourceLimits,
        editing_limits: &yrs_engine::EditingLimits,
    ) -> Result<Arc<Self>, ActiveState> {
        let Some(retained_bytes) = active_state_retained_bytes(&value, resource_limits) else {
            return Err(value);
        };
        let retained_budget = resource_limits
            .max_input_bytes
            .min(editing_limits.max_derived_output_bytes);
        #[cfg(test)]
        if FORCE_ACTIVE_STATE_CACHE_ALLOCATION_FAILURE.get() {
            return Err(value);
        }
        #[cfg(test)]
        let retained_budget = FORCE_ACTIVE_STATE_CACHE_BUDGET
            .get()
            .unwrap_or(retained_budget);
        if retained_bytes > retained_budget {
            return Err(value);
        }
        // This catches the optional certificate allocation under configured
        // limits. Deep `ActiveState` ownership is moved, not cloned. As across
        // the rest of this crate, an actual global allocator OOM during
        // `Arc::new` follows Rust's allocator behavior rather than becoming an
        // operation error.
        let mut allocation_probe = Vec::<u8>::new();
        if allocation_probe
            .try_reserve_exact(std::mem::size_of::<Self>())
            .is_err()
        {
            return Err(value);
        }
        Ok(Arc::new(Self {
            value,
            retained_bytes,
        }))
    }

    pub(super) fn fits_limits(
        &self,
        resource_limits: &ResourceLimits,
        editing_limits: &yrs_engine::EditingLimits,
    ) -> bool {
        self.retained_bytes <= resource_limits.max_input_bytes
            && self.retained_bytes <= editing_limits.max_derived_output_bytes
    }

    pub(crate) fn clone_public(
        &self,
        resource_limits: &ResourceLimits,
        editing_limits: &yrs_engine::EditingLimits,
    ) -> Option<ActiveState> {
        #[cfg(test)]
        if FORCE_ACTIVE_STATE_PUBLIC_MATERIALIZATION_FAILURE.get() {
            return None;
        }
        if !self.fits_limits(resource_limits, editing_limits) {
            return None;
        }
        record_active_state_public_result_clone();
        // The complete owned capacity was admitted above. Rust's global
        // allocator OOM behavior remains unchanged; configured exhaustion is
        // handled before this deep clone.
        Some(self.value.clone())
    }

    pub(crate) fn value(&self) -> &ActiveState {
        &self.value
    }

    pub(crate) fn try_into_value(cached: Arc<Self>) -> Result<ActiveState, Arc<Self>> {
        Arc::try_unwrap(cached).map(|cached| cached.value)
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes_for_test(&self) -> usize {
        self.retained_bytes
    }
}

#[derive(Debug, Clone)]
pub(super) struct ActiveStateBaseSeal {
    pub(super) request_id: u64,
    pub(super) document: Document,
    pub(super) canonical_artifact: CanonicalArtifact,
    pub(super) document_revision: u64,
    pub(super) state_revision: u64,
    pub(super) yrs_state_epoch: u64,
    pub(super) schema_fingerprint: String,
    pub(super) resource_limits: ResourceLimits,
    pub(super) editing_limits: yrs_engine::EditingLimits,
    pub(super) max_length: Option<u32>,
    pub(super) legacy_selection: Selection,
    pub(super) relative_selection: RelativeSelection,
    pub(super) resolved_selection: ResolvedSelection,
    pub(super) stored_marks: Option<Vec<Mark>>,
    pub(super) render_seal: Arc<crate::render::incremental::CachedRenderBlocks>,
    pub(super) lookup_seal: Arc<yrs_engine::mutation::MutationLookupSeed>,
    pub(super) validation_certificate: DocumentValidationCertificate,
    pub(super) structural: ActiveStateStructuralSeal,
}

impl ActiveStateBaseSeal {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn mint(
        request_id: u64,
        authority: &dyn DerivedStateAuthority,
        structural: ActiveStateStructuralSeal,
        resource_limits: &ResourceLimits,
        editing_limits: &yrs_engine::EditingLimits,
        max_length: Option<u32>,
        yrs_state_epoch: u64,
    ) -> OperationResult<Self> {
        let state = authority.installed();
        let lookup_seal = Arc::clone(authority.lookup_seed(request_id)?);
        Ok(Self {
            request_id,
            document: state.document.clone(),
            canonical_artifact: state.canonical_artifact.clone(),
            document_revision: state.document_revision,
            state_revision: state.state_revision,
            yrs_state_epoch,
            schema_fingerprint: state.schema_fingerprint.clone(),
            resource_limits: resource_limits.clone(),
            editing_limits: editing_limits.clone(),
            max_length,
            legacy_selection: state.legacy_selection.clone(),
            relative_selection: state.relative_selection.clone(),
            resolved_selection: state.resolved_selection.clone(),
            stored_marks: state.stored_marks.clone(),
            render_seal: Arc::clone(&state.render_blocks),
            lookup_seal,
            validation_certificate: state.validation_certificate.clone(),
            structural,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn matches(
        &self,
        authority: &dyn DerivedStateAuthority,
        structural: &ActiveStateStructuralSeal,
        resource_limits: &ResourceLimits,
        editing_limits: &yrs_engine::EditingLimits,
        max_length: Option<u32>,
        yrs_state_epoch: u64,
    ) -> bool {
        let state = authority.installed();
        let Ok(lookup_seed) = authority.lookup_seed(self.request_id) else {
            return false;
        };
        self.matches_with_lookup_seed(
            state,
            lookup_seed,
            structural,
            resource_limits,
            editing_limits,
            max_length,
            yrs_state_epoch,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn matches_installed(
        &self,
        authority: &dyn DerivedStateAuthority,
        structural: &ActiveStateStructuralSeal,
        resource_limits: &ResourceLimits,
        editing_limits: &yrs_engine::EditingLimits,
        max_length: Option<u32>,
        yrs_state_epoch: u64,
    ) -> bool {
        let state = authority.installed();
        self.matches_with_lookup_seed(
            state,
            &state.mutation_lookup_seed,
            structural,
            resource_limits,
            editing_limits,
            max_length,
            yrs_state_epoch,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn matches_with_lookup_seed(
        &self,
        state: &DerivedStateCache,
        lookup_seed: &Arc<yrs_engine::mutation::MutationLookupSeed>,
        structural: &ActiveStateStructuralSeal,
        resource_limits: &ResourceLimits,
        editing_limits: &yrs_engine::EditingLimits,
        max_length: Option<u32>,
        yrs_state_epoch: u64,
    ) -> bool {
        self.document.shares_root_storage_with(&state.document)
            && self.canonical_artifact.ptr_eq(&state.canonical_artifact)
            && self.document_revision == state.document_revision
            && self.state_revision == state.state_revision
            && self.yrs_state_epoch == yrs_state_epoch
            && self.schema_fingerprint == state.schema_fingerprint
            && self.resource_limits == *resource_limits
            && self.editing_limits == *editing_limits
            && self.max_length == max_length
            && self.legacy_selection == state.legacy_selection
            && self.relative_selection == state.relative_selection
            && self.resolved_selection == state.resolved_selection
            && self.stored_marks == state.stored_marks
            && Arc::ptr_eq(&self.render_seal, &state.render_blocks)
            && Arc::ptr_eq(&self.lookup_seal, lookup_seed)
            && self.validation_certificate == state.validation_certificate
            && self.structural == *structural
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ActiveStateCertificate {
    pub(super) base: ActiveStateBaseSeal,
    pub(super) cached: Arc<CachedActiveState>,
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedActiveStateTransition {
    pub(super) base: ActiveStateBaseSeal,
    pub(super) preview: Document,
    pub(super) result_selection: ResolvedSelection,
    pub(super) stored_marks: Option<Vec<Mark>>,
    pub(super) certificate: Option<Arc<ActiveStateCertificate>>,
}

#[cfg(test)]
impl PreparedActiveStateTransition {
    pub(crate) fn tamper_for_test(&mut self, claim: &str) {
        match claim {
            "documentRevision" => {
                self.base.document_revision = self.base.document_revision.saturating_add(1)
            }
            "stateRevision" => {
                self.base.state_revision = self.base.state_revision.saturating_add(1)
            }
            "epoch" => self.base.yrs_state_epoch = self.base.yrs_state_epoch.saturating_add(1),
            "schema" => self.base.schema_fingerprint.push('!'),
            "resource" => {
                self.base.resource_limits.max_document_nodes = self
                    .base
                    .resource_limits
                    .max_document_nodes
                    .saturating_add(1)
            }
            "editing" => {
                self.base.editing_limits.max_derived_output_bytes = self
                    .base
                    .editing_limits
                    .max_derived_output_bytes
                    .saturating_add(1)
            }
            "maxLength" => self.base.max_length = Some(self.base.max_length.unwrap_or(0) + 1),
            "selection" => self.base.resolved_selection = ResolvedSelection::All,
            "relativeSelection" => self.base.relative_selection = RelativeSelection::All,
            "legacySelection" => self.base.legacy_selection = Selection::all(),
            "storedMarks" => self.base.stored_marks = Some(Vec::new()),
            "structural" => {
                self.base.structural.leaf_doc_start =
                    self.base.structural.leaf_doc_start.saturating_add(1)
            }
            "resultSelection" => self.result_selection = ResolvedSelection::All,
            "preview" => self.preview = self.base.document.clone(),
            "render" => self.base.render_seal = Arc::new((*self.base.render_seal).clone()),
            "lookup" => self.base.lookup_seal = Arc::new((*self.base.lookup_seal).clone()),
            "validation" => {
                self.base.validation_certificate.state_revision = self
                    .base
                    .validation_certificate
                    .state_revision
                    .saturating_add(1)
            }
            "cachedPayloadIdentity" => {
                let certificate = self
                    .certificate
                    .as_ref()
                    .expect("warm transition certificate");
                self.certificate = Some(Arc::new(ActiveStateCertificate {
                    base: certificate.base.clone(),
                    cached: Arc::new(CachedActiveState {
                        value: certificate.cached.value.clone(),
                        retained_bytes: certificate.cached.retained_bytes,
                    }),
                }));
            }
            other => panic!("unknown active-state transition claim {other}"),
        }
    }
}

#[derive(Debug)]
pub(crate) struct PreparedActiveStateInstall {
    pub(super) request_id: u64,
    pub(super) preview: Document,
    pub(super) result_selection: ResolvedSelection,
    pub(super) stored_marks: Option<Vec<Mark>>,
    pub(super) cached: Arc<CachedActiveState>,
    pub(super) structural: ActiveStateStructuralSeal,
    pub(super) next_document_revision: u64,
    pub(super) next_state_revision: u64,
    pub(super) next_yrs_state_epoch: u64,
}

impl DerivedStateCache {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_active_state_transition(
        &self,
        request_id: u64,
        authority: &dyn DerivedStateAuthority,
        admission: &LocalizedInsertAdmission,
        preview: &Document,
        result_selection: &ResolvedSelection,
        stored_marks: Option<&[Mark]>,
        resource_limits: &ResourceLimits,
        editing_limits: &yrs_engine::EditingLimits,
        max_length: Option<u32>,
        yrs_state_epoch: u64,
    ) -> OperationResult<PreparedActiveStateTransition> {
        let structural = admission.active_state_structural_seal();
        let certificate = self
            .active_state_certificate
            .as_ref()
            .filter(|certificate| {
                certificate.base.matches_installed(
                    authority,
                    &structural,
                    resource_limits,
                    editing_limits,
                    max_length,
                    yrs_state_epoch,
                )
            })
            .map(Arc::clone);
        Ok(PreparedActiveStateTransition {
            base: ActiveStateBaseSeal::mint(
                request_id,
                authority,
                structural,
                resource_limits,
                editing_limits,
                max_length,
                yrs_state_epoch,
            )?,
            preview: preview.clone(),
            result_selection: result_selection.clone(),
            stored_marks: stored_marks.map(<[Mark]>::to_vec),
            certificate,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn validate_active_state_transition(
        &self,
        authority: &dyn DerivedStateAuthority,
        transition: &PreparedActiveStateTransition,
        structural: &ActiveStateStructuralSeal,
        preview: &Document,
        result_selection: &ResolvedSelection,
        stored_marks: Option<&[Mark]>,
        resource_limits: &ResourceLimits,
        editing_limits: &yrs_engine::EditingLimits,
        max_length: Option<u32>,
        yrs_state_epoch: u64,
    ) -> Option<Option<Arc<CachedActiveState>>> {
        let current_cache_matches = match (
            transition.certificate.as_ref(),
            self.active_state_certificate.as_ref(),
        ) {
            (Some(transition_certificate), Some(certificate)) => {
                Arc::ptr_eq(transition_certificate, certificate)
                    && certificate
                        .cached
                        .fits_limits(resource_limits, editing_limits)
                    && certificate.base.matches_installed(
                        authority,
                        structural,
                        resource_limits,
                        editing_limits,
                        max_length,
                        yrs_state_epoch,
                    )
            }
            (None, None) => true,
            _ => false,
        };
        (current_cache_matches
            && transition.base.matches(
                authority,
                structural,
                resource_limits,
                editing_limits,
                max_length,
                yrs_state_epoch,
            )
            && transition.preview.shares_root_storage_with(preview)
            && transition.result_selection == *result_selection
            && transition.stored_marks.as_deref() == stored_marks)
            .then(|| {
                transition
                    .certificate
                    .as_ref()
                    .map(|certificate| Arc::clone(&certificate.cached))
            })
    }

    pub(crate) fn prepare_active_state_install(
        transition: &PreparedActiveStateTransition,
        cached: Arc<CachedActiveState>,
        next_document_revision: u64,
        next_state_revision: u64,
        next_yrs_state_epoch: u64,
    ) -> PreparedActiveStateInstall {
        PreparedActiveStateInstall {
            request_id: transition.base.request_id,
            preview: transition.preview.clone(),
            result_selection: transition.result_selection.clone(),
            stored_marks: transition.stored_marks.clone(),
            cached,
            structural: transition.base.structural.clone(),
            next_document_revision,
            next_state_revision,
            next_yrs_state_epoch,
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_active_state_certificate(
        install: PreparedActiveStateInstall,
        authority: &dyn DerivedStateAuthority,
        resource_limits: &ResourceLimits,
        editing_limits: &yrs_engine::EditingLimits,
        max_length: Option<u32>,
        yrs_state_epoch: u64,
    ) -> Option<Arc<ActiveStateCertificate>> {
        let state = authority.installed();
        if !install.preview.shares_root_storage_with(&state.document)
            || install.result_selection != state.resolved_selection
            || install.stored_marks != state.stored_marks
            || install.next_document_revision != state.document_revision
            || install.next_state_revision != state.state_revision
            || install.next_yrs_state_epoch != yrs_state_epoch
        {
            return None;
        }
        Some(Arc::new(ActiveStateCertificate {
            base: ActiveStateBaseSeal::mint(
                install.request_id,
                authority,
                install.structural,
                resource_limits,
                editing_limits,
                max_length,
                yrs_state_epoch,
            )
            .ok()?,
            cached: install.cached,
        }))
    }

    pub(crate) fn install_active_state_certificate(
        &mut self,
        certificate: Arc<ActiveStateCertificate>,
    ) {
        self.active_state_certificate = Some(certificate);
    }

    pub(crate) fn clear_active_state_certificate(&mut self) {
        if self.active_state_certificate.take().is_some() {
            record_active_state_cache_drop();
        }
    }

    pub(crate) fn has_active_state_certificate(&self) -> bool {
        self.active_state_certificate.is_some()
    }

    #[cfg(test)]
    pub(crate) fn active_state_cache_for_test(&self) -> Option<Arc<CachedActiveState>> {
        self.active_state_certificate
            .as_ref()
            .map(|certificate| Arc::clone(&certificate.cached))
    }

    #[cfg(test)]
    pub(crate) fn remove_active_state_certificate_for_test(&mut self) {
        self.active_state_certificate = None;
    }

    #[cfg(test)]
    pub(crate) fn replace_active_state_certificate_identity_for_test(&mut self) {
        let certificate = self
            .active_state_certificate
            .as_ref()
            .expect("test requires an active-state certificate")
            .as_ref()
            .clone();
        self.active_state_certificate = Some(Arc::new(certificate));
    }

    #[cfg(test)]
    pub(crate) fn replace_active_state_payload_identity_for_test(&mut self) {
        let certificate = Arc::clone(
            self.active_state_certificate
                .as_ref()
                .expect("test requires an active-state certificate"),
        );
        self.active_state_certificate = Some(Arc::new(ActiveStateCertificate {
            base: certificate.base.clone(),
            cached: Arc::new(CachedActiveState {
                value: certificate.cached.value.clone(),
                retained_bytes: certificate.cached.retained_bytes,
            }),
        }));
    }
}
