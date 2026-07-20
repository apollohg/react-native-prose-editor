/// Single-source event collector for the exact ready lookup payload. The
/// ordinary builder and validated codec traversal both drive this state
/// machine, so the import receipt cannot drift from fallback semantics.
pub(crate) struct ImportLookupMaterializationCollector {
    request_id: u64,
    failed: Option<OperationError>,
    pending_traversal_work: usize,
    target_count: usize,
    target_materialization_work: HashMap<BranchID, usize>,
    path_parent_widths: HashMap<BranchID, usize>,
    frames: Vec<ImportLookupFrame>,
}

struct ImportLookupFrame {
    ancestor_depth: usize,
    structural_child_count: usize,
    kind: ImportLookupFrameKind,
}

enum ImportLookupFrameKind {
    Structural {
        parent_id: BranchID,
        branch_depth: usize,
    },
    Textblock {
        parent_id: BranchID,
        path_len: usize,
        previous_was_text: bool,
    },
    Fragment,
}

#[derive(Default)]
pub(crate) struct ImportElementAttributeWork {
    work: Option<usize>,
    attr_count: usize,
    key_bytes: usize,
    failure: Option<&'static str>,
}

impl ImportElementAttributeWork {
    pub(crate) fn new() -> Self {
        Self {
            work: Some(0),
            ..Self::default()
        }
    }

    pub(crate) fn observe(&mut self, key: &str, value: &Any) {
        let Some(work) = self.work else {
            return;
        };
        self.attr_count = match self.attr_count.checked_add(1) {
            Some(value) => value,
            None => {
                self.work = None;
                self.failure = Some("XML attribute traversal work overflow");
                return;
            }
        };
        self.key_bytes = match self.key_bytes.checked_add(key.len()) {
            Some(value) => value,
            None => {
                self.work = None;
                self.failure = Some("XML attribute traversal work overflow");
                return;
            }
        };
        self.work = work
            .checked_add(key.len())
            .and_then(|work| work.checked_add(any_traversal_work(value)?));
        if self.work.is_none() {
            self.failure = Some("XML attribute traversal work overflow");
        }
    }

    pub(crate) fn failure(&self) -> Option<&'static str> {
        self.failure
    }

    fn finish(self) -> Result<usize, &'static str> {
        if let Some(failure) = self.failure {
            return Err(failure);
        }
        let partitions = binary_partition_work(self.attr_count);
        self.work
            .and_then(|work| {
                work.checked_add(
                    self.attr_count
                .checked_mul(partitions)?
                .checked_add(self.key_bytes.checked_mul(partitions)?)?,
                )
            })
            .ok_or("XML attribute sort work overflow")
    }
}

#[derive(Default)]
pub(crate) struct ImportTextCaptureWork {
    work: Option<usize>,
    scalar_len: u32,
    utf16_len: u32,
    failure: Option<&'static str>,
}

impl ImportTextCaptureWork {
    pub(crate) fn new() -> Self {
        Self {
            work: Some(0),
            ..Self::default()
        }
    }

    pub(crate) fn observe(&mut self, value: &str, attrs: Option<&Attrs>) {
        if self.work.is_none() || value.is_empty() {
            return;
        }
        if value.is_ascii() {
            let Some(len) = u32::try_from(value.len()).ok() else {
                self.work = None;
                self.failure = Some("Yrs XML text length exceeds u32");
                return;
            };
            self.scalar_len = match self.scalar_len.checked_add(len) {
                Some(value) => value,
                None => {
                    self.work = None;
                    self.failure = Some("Yrs XML text scalar length overflow");
                    return;
                }
            };
            self.utf16_len = match self.utf16_len.checked_add(len) {
                Some(value) => value,
                None => {
                    self.work = None;
                    self.failure = Some("Yrs XML text UTF-16 length overflow");
                    return;
                }
            };
        } else {
            for scalar in value.chars() {
                let Some(scalar_len) = self.scalar_len.checked_add(1) else {
                    self.work = None;
                    self.failure = Some("Yrs XML text scalar length overflow");
                    return;
                };
                self.scalar_len = scalar_len;
                let scalar_utf16_len = if scalar.len_utf16() == 1 { 1 } else { 2 };
                let Some(utf16_len) = self.utf16_len.checked_add(scalar_utf16_len) else {
                    self.work = None;
                    self.failure = Some("Yrs XML text UTF-16 length overflow");
                    return;
                };
                self.utf16_len = utf16_len;
            }
        }
        let attrs_len = attrs.map_or(0, Attrs::len);
        let mut work = match self.work.and_then(|work| work.checked_add(attrs_len)) {
            Some(work) => work,
            None => {
                self.work = None;
                self.failure = Some("Yrs XML text materialization work overflow");
                return;
            }
        };
        let mut key_bytes = 0usize;
        if let Some(attrs) = attrs {
            for (key, value) in attrs {
                let Some(next_key_bytes) = key_bytes.checked_add(key.len()) else {
                    self.work = None;
                    self.failure = Some("Yrs XML text materialization work overflow");
                    return;
                };
                key_bytes = next_key_bytes;
                let Some(next_work) = work
                    .checked_add(key.len())
                    .and_then(|work| work.checked_add(super::plan::any_preflight_work(value)?))
                else {
                    self.work = None;
                    self.failure = Some("Yrs XML text materialization work overflow");
                    return;
                };
                work = next_work;
            }
        }
        let partitions = binary_partition_work(attrs_len);
        self.work = attrs_len
            .checked_mul(partitions)
            .and_then(|sort| sort.checked_add(key_bytes.checked_mul(partitions)?))
            .and_then(|sort| work.checked_add(sort))
            .and_then(|work| work.checked_add(value.len()))
            .and_then(|work| work.checked_add(1));
        if self.work.is_none() {
            self.failure = Some("Yrs XML text materialization work overflow");
        }
    }

    pub(crate) fn failure(&self) -> Option<&'static str> {
        self.failure
    }

    fn finish(self) -> Result<usize, &'static str> {
        if let Some(failure) = self.failure {
            return Err(failure);
        }
        self.work
            .ok_or("Yrs XML text materialization work overflow")
    }
}

impl ImportLookupMaterializationCollector {
    pub(crate) fn has_failed(&self) -> bool {
        self.failed.is_some()
    }

    pub(crate) fn new(
        request_id: u64,
        root_id: BranchID,
        root_width_hint: usize,
        target_capacity_hint: Option<usize>,
    ) -> Self {
        let seed_capacity = root_width_hint.saturating_mul(2).saturating_add(1);
        let target_capacity = target_capacity_hint.map_or(seed_capacity, |hint| hint.max(seed_capacity));
        let force_map_growth = lookup_seed_hydration_should_fail("mapGrowth");
        let initial_target_capacity = if force_map_growth { 0 } else { target_capacity };
        let initial_width_capacity = if force_map_growth { 0 } else { seed_capacity };
        let mut collector = Self {
            request_id,
            failed: None,
            pending_traversal_work: 0,
            target_count: 0,
            target_materialization_work: HashMap::new(),
            path_parent_widths: HashMap::new(),
            frames: Vec::new(),
        };
        if lookup_seed_hydration_should_fail("initialReservation")
            || collector
                .target_materialization_work
                .try_reserve(initial_target_capacity)
                .is_err()
            || collector
                .path_parent_widths
                .try_reserve(initial_width_capacity)
                .is_err()
            || collector.frames.try_reserve(1).is_err()
        {
            collector.fail("initialReservation");
            return collector;
        }
        collector.frames.push(ImportLookupFrame {
            ancestor_depth: 0,
            structural_child_count: 0,
            kind: ImportLookupFrameKind::Structural {
                parent_id: root_id,
                branch_depth: 0,
            },
        });
        collector
    }

    fn fail(&mut self, stage: &'static str) {
        if self.failed.is_none() {
            self.failed = Some(lookup_seed_allocation_error(self.request_id, stage));
        }
    }

    fn invariant(&mut self, message: &'static str) {
        if self.failed.is_none() {
            self.failed = Some(OperationError::engine_invariant_failed(
                self.request_id,
                None,
                message,
            ));
        }
    }

    pub(crate) fn invalidate(&mut self, message: &'static str) {
        self.invariant(message);
    }

    fn add_work(&mut self, amount: usize, message: &'static str) {
        if self.failed.is_some() {
            return;
        }
        match self.pending_traversal_work.checked_add(amount) {
            Some(work) => self.pending_traversal_work = work,
            None => self.invariant(message),
        }
    }

    fn reserve_entry<K: Eq + std::hash::Hash, V>(
        request_id: u64,
        map: &mut HashMap<K, V>,
    ) -> Result<(), OperationError> {
        if map.len() < map.capacity() {
            return Ok(());
        }
        #[cfg(test)]
        LOOKUP_SEED_MAP_GROWTH_ATTEMPT_COUNT.set(
            LOOKUP_SEED_MAP_GROWTH_ATTEMPT_COUNT.get().saturating_add(1),
        );
        if lookup_seed_hydration_should_fail("mapGrowth") {
            return Err(lookup_seed_allocation_error(request_id, "mapGrowth"));
        }
        map.try_reserve(1)
            .map_err(|_| lookup_seed_allocation_error(request_id, "mapGrowth"))
    }

    fn before_child(&mut self, child_is_text: bool) -> Option<usize> {
        if self.failed.is_some() {
            return None;
        }
        #[cfg(test)]
        IMPORT_LOOKUP_EVENT_COUNT.set(IMPORT_LOOKUP_EVENT_COUNT.get().saturating_add(1));
        let frame = self.frames.last_mut()?;
        let child_count_overflow_message = match &frame.kind {
            ImportLookupFrameKind::Textblock { .. } => "Yrs textblock child count overflow",
            ImportLookupFrameKind::Structural { .. } | ImportLookupFrameKind::Fragment => {
                "structural parent child count overflow"
            }
        };
        frame.structural_child_count = match frame.structural_child_count.checked_add(1) {
            Some(value) => value,
            None => {
                self.invariant(child_count_overflow_message);
                return None;
            }
        };
        let path_len = match frame.ancestor_depth.checked_add(1) {
            Some(value) => value,
            None => {
                self.invariant("Yrs mutation path capture work overflow");
                return None;
            }
        };
        let missing_gap_work = if let ImportLookupFrameKind::Textblock {
            path_len,
            previous_was_text,
            ..
        } = &mut frame.kind
        {
            let missing = !*previous_was_text && !child_is_text;
            *previous_was_text = child_is_text;
            missing.then_some(*path_len)
        } else {
            None
        };
        self.add_work(1, "Yrs mutation target traversal work overflow");
        self.add_work(path_len, "Yrs mutation path capture work overflow");
        if self.failed.is_some() {
            return None;
        }
        if let Some(missing_gap_work) = missing_gap_work {
            self.add_work(
                missing_gap_work,
                "Yrs missing-gap signature work overflow",
            );
            self.target_count = match self.target_count.checked_add(1) {
                Some(value) => value,
                None => {
                    self.invariant("Yrs mutation target count overflow");
                    return None;
                }
            };
        }
        if self.failed.is_some() {
            return None;
        }
        Some(path_len)
    }

    pub(crate) fn observe_text(
        &mut self,
        target_id: BranchID,
        capture: ImportTextCaptureWork,
    ) {
        let Some(path_len) = self.before_child(true) else {
            return;
        };
        self.add_work(path_len, "Yrs text signature preflight work overflow");
        if self.failed.is_some() {
            return;
        }
        let capture_work = match capture.finish() {
            Ok(work) => work,
            Err(message) => {
                self.invariant(message);
                return;
            }
        };
        self.add_work(capture_work, "Yrs text materialization work overflow");
        if self.failed.is_some() {
            return;
        }
        if let Err(error) = Self::reserve_entry(self.request_id, &mut self.target_materialization_work) {
            self.failed = Some(error);
            return;
        }
        if self.target_materialization_work.insert(target_id, capture_work).is_some() {
            self.invariant("duplicate Yrs text materialization");
            return;
        }
        self.target_count = match self.target_count.checked_add(1) {
            Some(value) => value,
            None => {
                self.invariant("Yrs mutation target count overflow");
                return;
            }
        };
    }

    pub(crate) fn begin_element(
        &mut self,
        element_id: BranchID,
        attributes: ImportElementAttributeWork,
        is_void: bool,
        is_textblock: bool,
    ) -> bool {
        let Some(path_len) = self.before_child(false) else {
            return !is_void;
        };
        let attribute_work = match attributes.finish() {
            Ok(work) => work,
            Err(message) => {
                self.invariant(message);
                return !is_void;
            }
        };
        self.add_work(attribute_work, "XML attribute traversal work overflow");
        if self.failed.is_some() {
            return !is_void;
        }
        if is_void {
            return false;
        }
        if self.frames.try_reserve(1).is_err() {
            self.fail("mapGrowth");
            return true;
        }
        self.frames.push(ImportLookupFrame {
            ancestor_depth: path_len,
            structural_child_count: 0,
            kind: if is_textblock {
                ImportLookupFrameKind::Textblock {
                    parent_id: element_id,
                    path_len,
                    previous_was_text: false,
                }
            } else {
                ImportLookupFrameKind::Structural {
                    parent_id: element_id,
                    branch_depth: path_len,
                }
            },
        });
        true
    }

    pub(crate) fn begin_fragment(&mut self) {
        let Some(path_len) = self.before_child(false) else {
            return;
        };
        if self.frames.try_reserve(1).is_err() {
            self.fail("mapGrowth");
            return;
        }
        self.frames.push(ImportLookupFrame {
            ancestor_depth: path_len,
            structural_child_count: 0,
            kind: ImportLookupFrameKind::Fragment,
        });
    }

    fn publish_width(&mut self, parent_id: BranchID, width: usize) {
        if let Err(error) = Self::reserve_entry(self.request_id, &mut self.path_parent_widths) {
            self.failed = Some(error);
            return;
        }
        if self.path_parent_widths.insert(parent_id, width).is_some() {
            self.invariant("duplicate Yrs structural parent");
        }
    }

    pub(crate) fn end_container(&mut self) {
        if self.failed.is_some() {
            let _ = self.frames.pop();
            return;
        }
        let Some(frame) = self.frames.pop() else {
            self.invariant("lookup collector frame underflow");
            return;
        };
        match frame.kind {
            ImportLookupFrameKind::Structural {
                parent_id,
                branch_depth,
            } => {
                self.add_work(1, "structural parent traversal work overflow");
                self.add_work(branch_depth, "structural parent traversal work overflow");
                self.add_work(frame.structural_child_count, "structural parent traversal work overflow");
                if self.failed.is_none() {
                    self.publish_width(parent_id, frame.structural_child_count);
                }
            }
            ImportLookupFrameKind::Textblock {
                parent_id,
                path_len,
                previous_was_text,
            } => {
                self.add_work(frame.structural_child_count, "Yrs textblock materialization work overflow");
                self.add_work(frame.structural_child_count, "Yrs textblock materialization work overflow");
                self.add_work(path_len, "Yrs textblock materialization work overflow");
                if self.failed.is_some() {
                    return;
                }
                if !previous_was_text {
                    self.add_work(path_len, "Yrs missing-gap signature work overflow");
                    if self.failed.is_some() {
                        return;
                    }
                    self.target_count = match self.target_count.checked_add(1) {
                        Some(value) => value,
                        None => {
                            self.invariant("Yrs mutation target count overflow");
                            return;
                        }
                    };
                }
                self.add_work(1, "structural parent traversal work overflow");
                self.add_work(path_len, "structural parent traversal work overflow");
                self.add_work(frame.structural_child_count, "structural parent traversal work overflow");
                if self.failed.is_none() {
                    self.publish_width(parent_id, frame.structural_child_count);
                }
            }
            ImportLookupFrameKind::Fragment => {}
        }
    }

    fn finish_payload(mut self) -> OperationResult<MutationLookupPayload> {
        while !self.frames.is_empty() {
            self.end_container();
        }
        if let Some(error) = self.failed {
            return Err(error);
        }
        probe_lookup_seed_publication(
            self.request_id,
            "mapPublication",
            std::mem::size_of::<HashMap<BranchID, usize>>(),
        )?;
        let path_parent_widths = Arc::new(self.path_parent_widths);
        probe_lookup_seed_publication(
            self.request_id,
            "mapPublication",
            std::mem::size_of::<HashMap<BranchID, usize>>(),
        )?;
        let target_materialization_work = Arc::new(self.target_materialization_work);
        Ok(MutationLookupPayload {
            target_count: self.target_count,
            pending_traversal_work: self.pending_traversal_work,
            path_parent_widths,
            target_materialization_work,
        })
    }

    pub(crate) fn finish(self) -> OperationResult<ImportLookupMaterialization> {
        self.finish_payload().map(ImportLookupMaterialization::new)
    }
}
