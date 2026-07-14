use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use yrs::any::Any;
use yrs::branch::{Branch, BranchID};
use yrs::types::text::{Text, TextRef};
use yrs::types::xml::{
    Xml, XmlElementRef, XmlFragment, XmlFragmentRef, XmlOut, XmlTextPrelim, XmlTextRef,
};
use yrs::types::Attrs;
use yrs::{GetString, ReadTxn, TransactionMut};

use crate::model::{Fragment, Mark, Node};
use crate::schema::{NodeRole, Schema};

use super::{OperationError, OperationResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TargetSignature {
    target: BranchID,
    path: Vec<(BranchID, u32)>,
    initial_len_utf16: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParentSignature {
    parent: BranchID,
    tag: Arc<str>,
    path: Vec<(BranchID, u32)>,
    child_count: u32,
    initial_child_index: u32,
    left_neighbor: Option<BranchID>,
    right_neighbor: Option<BranchID>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct YrsMutationPlan {
    pub actions: Vec<YrsMutationAction>,
    compilation_work: usize,
    work_limit: usize,
}

#[cfg(test)]
impl YrsMutationPlan {
    pub(crate) fn compilation_work_for_test(&self) -> usize {
        self.compilation_work
    }

    pub(crate) fn set_work_limit_for_test(&mut self, limit: usize) {
        self.work_limit = limit;
    }

    pub(crate) fn single_action_for_test(action: YrsMutationAction) -> Self {
        Self {
            actions: vec![action],
            compilation_work: 0,
            work_limit: usize::MAX,
        }
    }
}

#[derive(Debug, Clone)]
#[allow(clippy::enum_variant_names)] // Names mirror the admitted typed operation vocabulary.
pub(crate) enum YrsMutationAction {
    CreateText {
        parent: XmlElementRef,
        child_index: u32,
        text: String,
        scalar_len: u32,
        len_utf16: u32,
        attrs: Attrs,
        follow_up: Vec<CreatedTextAction>,
        signature: ParentSignature,
        operation_index: usize,
    },
    InsertText {
        target: XmlTextRef,
        index_utf16: u32,
        text: String,
        len_utf16: u32,
        attrs: Attrs,
        signature: TargetSignature,
        operation_index: usize,
    },
    DeleteText {
        target: XmlTextRef,
        index_utf16: u32,
        len_utf16: u32,
        signature: TargetSignature,
        operation_index: usize,
    },
    FormatText {
        target: XmlTextRef,
        index_utf16: u32,
        len_utf16: u32,
        attrs: Attrs,
        signature: TargetSignature,
        operation_index: usize,
    },
}

#[derive(Debug, Clone)]
pub(crate) enum CreatedTextAction {
    Insert {
        index_utf16: u32,
        text: String,
        len_utf16: u32,
        attrs: Attrs,
        operation_index: usize,
    },
    Delete {
        index_utf16: u32,
        len_utf16: u32,
        operation_index: usize,
    },
    Format {
        index_utf16: u32,
        len_utf16: u32,
        attrs: Attrs,
        operation_index: usize,
    },
}

impl CreatedTextAction {
    fn operation_index(&self) -> usize {
        match self {
            Self::Insert {
                operation_index, ..
            }
            | Self::Delete {
                operation_index, ..
            }
            | Self::Format {
                operation_index, ..
            } => *operation_index,
        }
    }
}

impl YrsMutationAction {
    fn target(&self) -> &XmlTextRef {
        match self {
            Self::CreateText { .. } => unreachable!("create actions have an element parent"),
            Self::InsertText { target, .. }
            | Self::DeleteText { target, .. }
            | Self::FormatText { target, .. } => target,
        }
    }

    fn signature(&self) -> &TargetSignature {
        match self {
            Self::CreateText { .. } => unreachable!("create actions have a parent signature"),
            Self::InsertText { signature, .. }
            | Self::DeleteText { signature, .. }
            | Self::FormatText { signature, .. } => signature,
        }
    }

    fn operation_index(&self) -> usize {
        match self {
            Self::CreateText {
                operation_index, ..
            }
            | Self::InsertText {
                operation_index, ..
            }
            | Self::DeleteText {
                operation_index, ..
            }
            | Self::FormatText {
                operation_index, ..
            } => *operation_index,
        }
    }
}

#[derive(Debug)]
struct ResolvedText {
    kind: ResolvedTargetKind,
    gap_before: u32,
    text: String,
    scalar_len: u32,
}

#[derive(Debug)]
enum ResolvedTargetKind {
    Existing {
        target: XmlTextRef,
        signature: TargetSignature,
    },
    Missing {
        parent: XmlElementRef,
        child_index: u32,
        signature: ParentSignature,
        create_action: Option<usize>,
    },
}

enum LocatedTarget {
    Existing {
        start: u32,
        target: XmlTextRef,
        text: String,
        scalar_len: u32,
        signature: TargetSignature,
    },
    Missing {
        start: u32,
        parent: XmlElementRef,
        child_index: u32,
        signature: ParentSignature,
    },
}

#[derive(Debug)]
pub(crate) struct MutationCompiler {
    request_id: u64,
    targets: Vec<ResolvedText>,
    actions: Vec<YrsMutationAction>,
    charged_work: usize,
    pending_traversal_work: usize,
    action_limit: usize,
    scan_work: usize,
    scan_limit: usize,
    created_gap_shifts: HashMap<BranchID, Vec<u32>>,
    #[cfg(test)]
    virtual_delete_visits: usize,
}

impl MutationCompiler {
    pub(crate) fn new<T: ReadTxn>(
        request_id: u64,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        action_limit: usize,
        scan_limit: usize,
        scan_work: usize,
    ) -> OperationResult<Self> {
        let mut located = Vec::new();
        let mut traversal_work = 0usize;
        collect_text_targets(
            request_id,
            txn,
            (0u32..).zip(fragment.children(txn)),
            <XmlFragmentRef as AsRef<Branch>>::as_ref(fragment).id(),
            &[],
            0,
            schema,
            &mut traversal_work,
            &mut located,
        )?;
        let mut targets = Vec::with_capacity(located.len());
        let mut previous_end = 0u32;
        for located in located {
            let (start, scalar_len) = match &located {
                LocatedTarget::Existing {
                    start, scalar_len, ..
                } => (*start, *scalar_len),
                LocatedTarget::Missing { start, .. } => (*start, 0),
            };
            let gap_before = start.checked_sub(previous_end).ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "Yrs text targets overlap in document order",
                )
            })?;
            previous_end = start.checked_add(scalar_len).ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "Yrs text target position overflow",
                )
            })?;
            let target = match located {
                LocatedTarget::Existing {
                    target,
                    text,
                    signature,
                    ..
                } => ResolvedText {
                    kind: ResolvedTargetKind::Existing { signature, target },
                    gap_before,
                    text,
                    scalar_len,
                },
                LocatedTarget::Missing {
                    parent,
                    child_index,
                    signature,
                    ..
                } => ResolvedText {
                    kind: ResolvedTargetKind::Missing {
                        signature,
                        parent,
                        child_index,
                        create_action: None,
                    },
                    gap_before,
                    text: String::new(),
                    scalar_len: 0,
                },
            };
            targets.push(target);
        }
        Ok(Self {
            request_id,
            targets,
            actions: Vec::new(),
            charged_work: 0,
            pending_traversal_work: traversal_work,
            action_limit,
            scan_work,
            scan_limit,
            created_gap_shifts: HashMap::new(),
            #[cfg(test)]
            virtual_delete_visits: 0,
        })
    }

    pub(crate) fn insert(
        &mut self,
        operation_index: usize,
        position: u32,
        text: &str,
        marks: &[Mark],
    ) -> OperationResult<()> {
        self.charge_operation_work(operation_index, self.targets.len())?;
        let attrs = marks_to_attrs(marks);
        self.charge_scan_work(operation_index, text.len())?;
        let (text_scalar_len, text_utf16) =
            checked_text_lengths(self.request_id, Some(operation_index), text)?;
        let action_work = 1usize
            .checked_add(text.len())
            .and_then(|work| work.checked_add(usize::try_from(text_utf16).ok()?))
            .and_then(|work| work.checked_add(attrs_work(&attrs)))
            .ok_or_else(|| work_overflow(self.request_id, operation_index, self.action_limit))?;
        self.charge_operation_work(operation_index, action_work)?;
        let ResolvedInsertion {
            target_index,
            scalar_index,
        } = self.resolve_insertion(operation_index, position)?;
        self.charge_scan_work(operation_index, self.targets[target_index].text.len())?;
        let index_utf16 = scalar_to_utf16(
            self.request_id,
            operation_index,
            &self.targets[target_index].text,
            scalar_index,
        )?;
        let missing_gap_work = match &self.targets[target_index].kind {
            ResolvedTargetKind::Missing {
                signature,
                create_action: None,
                ..
            } => {
                let fenwick_len = usize::try_from(signature.child_count)
                    .ok()
                    .and_then(|len| len.checked_add(2))
                    .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
                binary_partition_work(fenwick_len)
                    .checked_mul(2)
                    .and_then(|work| {
                        work.checked_add(
                            if self.created_gap_shifts.contains_key(&signature.parent) {
                                0
                            } else {
                                fenwick_len
                            },
                        )
                    })
                    .ok_or_else(|| {
                        work_overflow(self.request_id, operation_index, self.action_limit)
                    })?
            }
            _ => 0,
        };
        self.charge_operation_work(operation_index, missing_gap_work)?;

        match &mut self.targets[target_index].kind {
            ResolvedTargetKind::Existing { target, signature } => {
                self.actions.push(YrsMutationAction::InsertText {
                    target: target.clone(),
                    index_utf16,
                    text: text.to_owned(),
                    len_utf16: text_utf16,
                    attrs,
                    signature: signature.clone(),
                    operation_index,
                });
            }
            ResolvedTargetKind::Missing {
                parent,
                child_index,
                signature,
                create_action,
            } => {
                if let Some(action_index) = *create_action {
                    let YrsMutationAction::CreateText { follow_up, .. } =
                        &mut self.actions[action_index]
                    else {
                        return Err(OperationError::engine_invariant_failed(
                            self.request_id,
                            Some(operation_index),
                            "created Yrs text target action index is invalid",
                        ));
                    };
                    follow_up.push(CreatedTextAction::Insert {
                        index_utf16,
                        text: text.to_owned(),
                        len_utf16: text_utf16,
                        attrs,
                        operation_index,
                    });
                } else {
                    let fenwick_len = usize::try_from(signature.child_count)
                        .ok()
                        .and_then(|len| len.checked_add(2))
                        .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
                    let gap = usize::try_from(signature.initial_child_index)
                        .ok()
                        .and_then(|index| index.checked_add(1))
                        .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
                    let prior_creates_before_gap = {
                        let shifts = self
                            .created_gap_shifts
                            .entry(signature.parent.clone())
                            .or_insert_with(|| vec![0; fenwick_len]);
                        let prior = fenwick_prefix(shifts, gap).ok_or_else(|| {
                            invalid_action_range(self.request_id, operation_index)
                        })?;
                        fenwick_add(shifts, gap).ok_or_else(|| {
                            invalid_action_range(self.request_id, operation_index)
                        })?;
                        prior
                    };
                    let execution_child_index = child_index
                        .checked_add(prior_creates_before_gap)
                        .ok_or_else(|| {
                        invalid_action_range(self.request_id, operation_index)
                    })?;
                    *create_action = Some(self.actions.len());
                    self.actions.push(YrsMutationAction::CreateText {
                        parent: parent.clone(),
                        child_index: execution_child_index,
                        text: text.to_owned(),
                        scalar_len: text_scalar_len,
                        len_utf16: text_utf16,
                        attrs,
                        follow_up: Vec::new(),
                        signature: signature.clone(),
                        operation_index,
                    });
                }
            }
        }
        let mutation_work = self.targets[target_index]
            .text
            .len()
            .checked_mul(2)
            .and_then(|work| work.checked_add(text.len()))
            .ok_or_else(|| scan_overflow(self.request_id, operation_index, self.scan_limit))?;
        self.charge_scan_work(operation_index, mutation_work)?;
        insert_scalar(
            self.request_id,
            operation_index,
            &mut self.targets[target_index].text,
            scalar_index,
            text,
        )?;
        self.targets[target_index].scalar_len = self.targets[target_index]
            .scalar_len
            .checked_add(text_scalar_len)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        Ok(())
    }

    pub(crate) fn delete(
        &mut self,
        operation_index: usize,
        from: u32,
        to: u32,
        boundaries: &[u32],
    ) -> OperationResult<()> {
        if from == to {
            return Ok(());
        }
        self.charge_operation_work(
            operation_index,
            self.targets
                .len()
                .checked_add(boundaries.len())
                .ok_or_else(|| {
                    work_overflow(self.request_id, operation_index, self.action_limit)
                })?,
        )?;
        let spans = self.covered_spans(operation_index, from, to, boundaries)?;
        for span in spans.iter().rev() {
            #[cfg(test)]
            {
                self.virtual_delete_visits += 1;
            }
            self.charge_operation_work(
                operation_index,
                1usize
                    .checked_add(usize::try_from(span.len_utf16).unwrap_or(usize::MAX))
                    .ok_or_else(|| {
                        work_overflow(self.request_id, operation_index, self.action_limit)
                    })?,
            )?;
            match &self.targets[span.target].kind {
                ResolvedTargetKind::Existing { target, signature } => {
                    self.actions.push(YrsMutationAction::DeleteText {
                        target: target.clone(),
                        index_utf16: span.index_utf16,
                        len_utf16: span.len_utf16,
                        signature: signature.clone(),
                        operation_index,
                    });
                }
                ResolvedTargetKind::Missing {
                    create_action: Some(action_index),
                    ..
                } => {
                    let YrsMutationAction::CreateText { follow_up, .. } =
                        &mut self.actions[*action_index]
                    else {
                        return Err(OperationError::engine_invariant_failed(
                            self.request_id,
                            Some(operation_index),
                            "created Yrs text target action index is invalid",
                        ));
                    };
                    follow_up.push(CreatedTextAction::Delete {
                        index_utf16: span.index_utf16,
                        len_utf16: span.len_utf16,
                        operation_index,
                    });
                }
                ResolvedTargetKind::Missing { .. } => {
                    return Err(OperationError::engine_invariant_failed(
                        self.request_id,
                        Some(operation_index),
                        "delete resolved to an uncreated Yrs text target",
                    ));
                }
            }
        }
        for span in spans.iter().rev() {
            let mutation_work = self.targets[span.target]
                .text
                .len()
                .checked_mul(3)
                .ok_or_else(|| scan_overflow(self.request_id, operation_index, self.scan_limit))?;
            self.charge_scan_work(operation_index, mutation_work)?;
            remove_scalar_range(
                self.request_id,
                operation_index,
                &mut self.targets[span.target].text,
                span.from_scalar,
                span.to_scalar,
            )?;
            self.targets[span.target].scalar_len -= span.to_scalar - span.from_scalar;
        }
        Ok(())
    }

    pub(crate) fn format(
        &mut self,
        operation_index: usize,
        from: u32,
        to: u32,
        boundaries: &[u32],
        attrs: Attrs,
    ) -> OperationResult<()> {
        if from == to {
            return Ok(());
        }
        self.charge_operation_work(
            operation_index,
            self.targets
                .len()
                .checked_add(boundaries.len())
                .ok_or_else(|| {
                    work_overflow(self.request_id, operation_index, self.action_limit)
                })?,
        )?;
        for span in self.covered_spans(operation_index, from, to, boundaries)? {
            let action_work = 1usize
                .checked_add(usize::try_from(span.len_utf16).unwrap_or(usize::MAX))
                .and_then(|work| work.checked_add(attrs_work(&attrs)))
                .ok_or_else(|| {
                    work_overflow(self.request_id, operation_index, self.action_limit)
                })?;
            self.charge_operation_work(operation_index, action_work)?;
            match &self.targets[span.target].kind {
                ResolvedTargetKind::Existing { target, signature } => {
                    self.actions.push(YrsMutationAction::FormatText {
                        target: target.clone(),
                        index_utf16: span.index_utf16,
                        len_utf16: span.len_utf16,
                        attrs: attrs.clone(),
                        signature: signature.clone(),
                        operation_index,
                    });
                }
                ResolvedTargetKind::Missing {
                    create_action: Some(action_index),
                    ..
                } => {
                    let YrsMutationAction::CreateText { follow_up, .. } =
                        &mut self.actions[*action_index]
                    else {
                        return Err(OperationError::engine_invariant_failed(
                            self.request_id,
                            Some(operation_index),
                            "created Yrs text target action index is invalid",
                        ));
                    };
                    follow_up.push(CreatedTextAction::Format {
                        index_utf16: span.index_utf16,
                        len_utf16: span.len_utf16,
                        attrs: attrs.clone(),
                        operation_index,
                    });
                }
                ResolvedTargetKind::Missing { .. } => {
                    return Err(OperationError::engine_invariant_failed(
                        self.request_id,
                        Some(operation_index),
                        "format resolved to an uncreated Yrs text target",
                    ));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn replace(
        &mut self,
        operation_index: usize,
        from: u32,
        to: u32,
        boundaries: &[u32],
        content: &Fragment,
    ) -> OperationResult<()> {
        let pieces = inline_text_pieces(self.request_id, operation_index, content)?;
        let piece_materialization_work = pieces
            .iter()
            .try_fold(0usize, |total, (text, _)| total.checked_add(text.len()))
            .ok_or_else(|| scan_overflow(self.request_id, operation_index, self.scan_limit))?;
        self.charge_scan_work(operation_index, piece_materialization_work)?;
        self.delete(operation_index, from, to, boundaries)?;
        let mut position = from;
        for (text, marks) in pieces {
            if text.is_empty() {
                continue;
            }
            self.insert(operation_index, position, &text, &marks)?;
            self.charge_scan_work(operation_index, text.len())?;
            position = position
                .checked_add(checked_scalar_len(
                    self.request_id,
                    Some(operation_index),
                    &text,
                )?)
                .ok_or_else(|| {
                    OperationError::operation_invalid(
                        self.request_id,
                        operation_index,
                        "content",
                        "replacement position overflow",
                    )
                })?;
        }
        Ok(())
    }

    pub(crate) fn finish(
        mut self,
        operation_index: Option<usize>,
    ) -> OperationResult<YrsMutationPlan> {
        self.charged_work = self
            .charged_work
            .checked_add(self.pending_traversal_work)
            .ok_or_else(|| {
                OperationError::operation_limit_exceeded(
                    self.request_id,
                    operation_index,
                    "maxActionsPerTransaction",
                    u64::try_from(self.action_limit).unwrap_or(u64::MAX),
                    u64::MAX,
                )
            })?;
        self.pending_traversal_work = 0;
        if self.charged_work > self.action_limit {
            return Err(OperationError::operation_limit_exceeded(
                self.request_id,
                operation_index,
                "maxActionsPerTransaction",
                u64::try_from(self.action_limit).unwrap_or(u64::MAX),
                u64::try_from(self.charged_work).unwrap_or(u64::MAX),
            ));
        }
        Ok(YrsMutationPlan {
            actions: self.actions,
            compilation_work: self.charged_work,
            work_limit: self.action_limit,
        })
    }

    fn charge_operation_work(
        &mut self,
        operation_index: usize,
        amount: usize,
    ) -> OperationResult<()> {
        let amount = amount
            .checked_add(self.pending_traversal_work)
            .ok_or_else(|| work_overflow(self.request_id, operation_index, self.action_limit))?;
        self.pending_traversal_work = 0;
        self.charged_work = self
            .charged_work
            .checked_add(amount)
            .ok_or_else(|| work_overflow(self.request_id, operation_index, self.action_limit))?;
        if self.charged_work > self.action_limit {
            return Err(OperationError::operation_limit_exceeded(
                self.request_id,
                Some(operation_index),
                "maxActionsPerTransaction",
                u64::try_from(self.action_limit).unwrap_or(u64::MAX),
                u64::try_from(self.charged_work).unwrap_or(u64::MAX),
            ));
        }
        Ok(())
    }

    fn charge_scan_work(&mut self, operation_index: usize, amount: usize) -> OperationResult<()> {
        self.scan_work = self
            .scan_work
            .checked_add(amount)
            .ok_or_else(|| scan_overflow(self.request_id, operation_index, self.scan_limit))?;
        if self.scan_work > self.scan_limit {
            return Err(OperationError::operation_limit_exceeded(
                self.request_id,
                Some(operation_index),
                "maxInputBytes",
                u64::try_from(self.scan_limit).unwrap_or(u64::MAX),
                u64::try_from(self.scan_work).unwrap_or(u64::MAX),
            ));
        }
        Ok(())
    }

    pub(crate) fn charge_boundary_text(
        &mut self,
        operation_index: usize,
        bytes: usize,
    ) -> OperationResult<()> {
        self.charge_scan_work(operation_index, bytes)
    }

    pub(crate) fn charge_boundary_node(&mut self, operation_index: usize) -> OperationResult<()> {
        self.charge_operation_work(operation_index, 1)
    }

    #[cfg(test)]
    pub(crate) fn total_mutation_work_for_test(&self) -> usize {
        self.charged_work + self.pending_traversal_work
    }

    #[cfg(test)]
    pub(crate) fn target_positions_for_test(&self) -> OperationResult<Vec<(u32, u32)>> {
        self.positions()
    }

    #[cfg(test)]
    pub(crate) fn virtual_delete_visits_for_test(&self) -> usize {
        self.virtual_delete_visits
    }

    fn positions(&self) -> OperationResult<Vec<(u32, u32)>> {
        let mut positions = Vec::with_capacity(self.targets.len());
        let mut cursor = 0u32;
        for target in &self.targets {
            cursor = cursor.checked_add(target.gap_before).ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    self.request_id,
                    None,
                    "mutation target position overflow",
                )
            })?;
            let start = cursor;
            cursor = cursor.checked_add(target.scalar_len).ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    self.request_id,
                    None,
                    "mutation target length overflow",
                )
            })?;
            positions.push((start, cursor));
        }
        Ok(positions)
    }

    fn resolve_insertion(
        &self,
        operation_index: usize,
        position: u32,
    ) -> OperationResult<ResolvedInsertion> {
        let positions = self.positions()?;
        for (index, &(start, end)) in positions.iter().enumerate() {
            if position >= start && position <= end {
                return Ok(ResolvedInsertion {
                    target_index: index,
                    scalar_index: position - start,
                });
            }
        }
        Err(OperationError::position_invalid(
            self.request_id,
            operation_index,
            "at",
            "text insertion does not resolve to an existing Yrs XML text target",
        ))
    }

    fn covered_spans(
        &mut self,
        operation_index: usize,
        from: u32,
        to: u32,
        boundaries: &[u32],
    ) -> OperationResult<Vec<ResolvedSpan>> {
        if boundaries.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(OperationError::engine_invariant_failed(
                self.request_id,
                Some(operation_index),
                "semantic text boundaries must be strictly increasing",
            ));
        }
        let positions = self.positions()?;
        let mut spans = Vec::new();
        let mut covered = 0u32;
        for (target, &(start, end)) in positions.iter().enumerate() {
            let overlap_from = from.max(start);
            let overlap_to = to.min(end);
            if overlap_from >= overlap_to {
                continue;
            }
            let lower = boundaries.partition_point(|boundary| *boundary <= overlap_from);
            let upper = boundaries.partition_point(|boundary| *boundary < overlap_to);
            let local_boundaries = &boundaries[lower..upper];
            let search_work = binary_partition_work(boundaries.len())
                .checked_mul(2)
                .and_then(|work| work.checked_add(local_boundaries.len()))
                .ok_or_else(|| {
                    work_overflow(self.request_id, operation_index, self.action_limit)
                })?;
            self.charge_operation_work(operation_index, search_work)?;
            let mut cuts = Vec::with_capacity(local_boundaries.len() + 2);
            cuts.push(overlap_from);
            cuts.extend_from_slice(local_boundaries);
            cuts.push(overlap_to);
            for pair in cuts.windows(2) {
                let local_from = pair[0] - start;
                let local_to = pair[1] - start;
                let coordinate_work =
                    self.targets[target]
                        .text
                        .len()
                        .checked_mul(2)
                        .ok_or_else(|| {
                            scan_overflow(self.request_id, operation_index, self.scan_limit)
                        })?;
                self.charge_scan_work(operation_index, coordinate_work)?;
                let index_utf16 = scalar_to_utf16(
                    self.request_id,
                    operation_index,
                    &self.targets[target].text,
                    local_from,
                )?;
                let end_utf16 = scalar_to_utf16(
                    self.request_id,
                    operation_index,
                    &self.targets[target].text,
                    local_to,
                )?;
                spans.push(ResolvedSpan {
                    target,
                    from_scalar: local_from,
                    to_scalar: local_to,
                    index_utf16,
                    len_utf16: end_utf16 - index_utf16,
                });
            }
            covered = covered
                .checked_add(overlap_to - overlap_from)
                .ok_or_else(|| {
                    OperationError::operation_invalid(
                        self.request_id,
                        operation_index,
                        "range",
                        "mutation range work overflow",
                    )
                })?;
        }
        if covered != to - from {
            return Err(OperationError::operation_invalid(
                self.request_id,
                operation_index,
                "range",
                "text mutation range crosses structural XML content",
            ));
        }
        Ok(spans)
    }
}

fn binary_partition_work(len: usize) -> usize {
    if len == 0 {
        0
    } else {
        usize::BITS as usize - len.leading_zeros() as usize
    }
}

fn fenwick_prefix(tree: &[u32], mut index: usize) -> Option<u32> {
    let mut total = 0u32;
    while index > 0 {
        total = total.checked_add(tree[index])?;
        index &= index - 1;
    }
    Some(total)
}

fn fenwick_add(tree: &mut [u32], mut index: usize) -> Option<()> {
    while index < tree.len() {
        tree[index] = tree[index].checked_add(1)?;
        let step = index & index.wrapping_neg();
        index = index.checked_add(step)?;
    }
    Some(())
}

#[derive(Debug)]
struct ResolvedSpan {
    target: usize,
    from_scalar: u32,
    to_scalar: u32,
    index_utf16: u32,
    len_utf16: u32,
}

struct ResolvedInsertion {
    target_index: usize,
    scalar_index: u32,
}

pub(crate) fn preflight_mutation_plan<T: ReadTxn>(
    request_id: u64,
    plan: &YrsMutationPlan,
    txn: &T,
) -> OperationResult<()> {
    preflight_mutation_plan_impl(request_id, plan, txn).map(|_| ())
}

fn preflight_mutation_plan_impl<T: ReadTxn>(
    request_id: u64,
    plan: &YrsMutationPlan,
    txn: &T,
) -> OperationResult<usize> {
    use std::collections::HashMap;

    let mut virtual_lengths = HashMap::<BranchID, u32>::new();
    let mut created_gaps = HashMap::<BranchID, Vec<u32>>::new();
    let mut validated_targets = std::collections::HashSet::<BranchID>::new();
    let mut path_children = HashMap::<BranchID, Vec<BranchID>>::new();
    let mut indexed_work = 0usize;
    for action in &plan.actions {
        if let YrsMutationAction::CreateText {
            parent,
            child_index,
            text,
            scalar_len,
            len_utf16,
            follow_up,
            signature,
            ..
        } = action
        {
            if !path_children.contains_key(&signature.parent) {
                indexed_work = indexed_work
                    .checked_add(signature.path.len())
                    .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
                validate_parent_identity(
                    request_id,
                    action.operation_index(),
                    parent,
                    signature,
                    txn,
                    &mut path_children,
                )?;
            }
            validate_parent_gap(
                request_id,
                action.operation_index(),
                signature,
                &path_children[&signature.parent],
            )?;
            indexed_work = indexed_work
                .checked_add(2)
                .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
            let fenwick_len = usize::try_from(signature.child_count)
                .ok()
                .and_then(|len| len.checked_add(2))
                .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
            let gap = usize::try_from(signature.initial_child_index)
                .ok()
                .and_then(|index| index.checked_add(1))
                .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
            let creates_index_is_new = !created_gaps.contains_key(&signature.parent);
            let prior = created_gaps
                .entry(signature.parent.clone())
                .or_insert_with(|| vec![0; fenwick_len]);
            let shift = fenwick_prefix(prior, gap)
                .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
            fenwick_add(prior, gap)
                .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
            indexed_work = indexed_work
                .checked_add(
                    binary_partition_work(fenwick_len)
                        .checked_mul(2)
                        .and_then(|work| {
                            work.checked_add(if creates_index_is_new { fenwick_len } else { 0 })
                        })
                        .ok_or_else(|| {
                            invalid_action_range(request_id, action.operation_index())
                        })?,
                )
                .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
            let expected_execution_index = signature
                .initial_child_index
                .checked_add(shift)
                .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
            if *child_index != expected_execution_index {
                return Err(invalid_action_range(request_id, action.operation_index()));
            }
            if text.is_empty() || *scalar_len == 0 || *len_utf16 == 0 {
                return Err(invalid_action_range(request_id, action.operation_index()));
            }
            let mut length = *len_utf16;
            for follow in follow_up {
                let operation_index = follow.operation_index();
                match follow {
                    CreatedTextAction::Insert {
                        index_utf16,
                        len_utf16,
                        ..
                    } => {
                        if *index_utf16 > length || *len_utf16 == 0 {
                            return Err(invalid_action_range(request_id, operation_index));
                        }
                        length = length
                            .checked_add(*len_utf16)
                            .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
                    }
                    CreatedTextAction::Delete {
                        index_utf16,
                        len_utf16,
                        ..
                    }
                    | CreatedTextAction::Format {
                        index_utf16,
                        len_utf16,
                        ..
                    } => {
                        let end = index_utf16
                            .checked_add(*len_utf16)
                            .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
                        if end > length {
                            return Err(invalid_action_range(request_id, operation_index));
                        }
                        if matches!(follow, CreatedTextAction::Delete { .. }) {
                            length -= *len_utf16;
                        }
                    }
                }
            }
            continue;
        }
        let target = action.target();
        let signature = action.signature();
        if validated_targets.insert(signature.target.clone()) {
            indexed_work = indexed_work
                .checked_add(signature.path.len())
                .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
            validate_signature(
                request_id,
                action.operation_index(),
                target,
                signature,
                txn,
                &mut path_children,
            )?;
        }
        let length = virtual_lengths
            .entry(signature.target.clone())
            .or_insert(signature.initial_len_utf16);
        match action {
            YrsMutationAction::CreateText { .. } => unreachable!(),
            YrsMutationAction::InsertText {
                index_utf16,
                len_utf16,
                ..
            } => {
                if *index_utf16 > *length {
                    return Err(invalid_action_range(request_id, action.operation_index()));
                }
                *length = length
                    .checked_add(*len_utf16)
                    .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
            }
            YrsMutationAction::DeleteText {
                index_utf16,
                len_utf16,
                ..
            }
            | YrsMutationAction::FormatText {
                index_utf16,
                len_utf16,
                ..
            } => {
                let end = index_utf16
                    .checked_add(*len_utf16)
                    .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
                if end > *length {
                    return Err(invalid_action_range(request_id, action.operation_index()));
                }
                if matches!(action, YrsMutationAction::DeleteText { .. }) {
                    *length -= *len_utf16;
                }
            }
        }
    }
    let materialized_children = path_children
        .values()
        .try_fold(0usize, |total, children| total.checked_add(children.len()))
        .ok_or_else(|| {
            OperationError::engine_invariant_failed(
                request_id,
                None,
                "Yrs preflight child-index work overflow",
            )
        })?;
    let preflight_work = indexed_work
        .checked_add(materialized_children)
        .ok_or_else(|| {
            OperationError::engine_invariant_failed(
                request_id,
                None,
                "Yrs preflight indexed work overflow",
            )
        })?;
    let total_work = plan
        .compilation_work
        .checked_add(preflight_work)
        .ok_or_else(|| {
            OperationError::operation_limit_exceeded(
                request_id,
                None,
                "maxActionsPerTransaction",
                u64::try_from(plan.work_limit).unwrap_or(u64::MAX),
                u64::MAX,
            )
        })?;
    if total_work > plan.work_limit {
        return Err(OperationError::operation_limit_exceeded(
            request_id,
            None,
            "maxActionsPerTransaction",
            u64::try_from(plan.work_limit).unwrap_or(u64::MAX),
            u64::try_from(total_work).unwrap_or(u64::MAX),
        ));
    }
    Ok(preflight_work)
}

#[cfg(test)]
pub(crate) fn preflight_mutation_work_for_test<T: ReadTxn>(
    request_id: u64,
    plan: &YrsMutationPlan,
    txn: &T,
) -> OperationResult<usize> {
    preflight_mutation_plan_impl(request_id, plan, txn)
}

#[allow(dead_code)] // Task 7 calls this after installing atomic production application.
pub(crate) fn execute_mutation_plan(plan: YrsMutationPlan, txn: &mut TransactionMut<'_>) {
    for action in plan.actions {
        match action {
            YrsMutationAction::CreateText {
                parent,
                child_index,
                text,
                attrs,
                follow_up,
                ..
            } => {
                let target = parent.insert(txn, child_index, XmlTextPrelim::new(""));
                target.insert_with_attributes(txn, 0, &text, attrs);
                for follow in follow_up {
                    match follow {
                        CreatedTextAction::Insert {
                            index_utf16,
                            text,
                            attrs,
                            ..
                        } => target.insert_with_attributes(txn, index_utf16, &text, attrs),
                        CreatedTextAction::Delete {
                            index_utf16,
                            len_utf16,
                            ..
                        } => target.remove_range(txn, index_utf16, len_utf16),
                        CreatedTextAction::Format {
                            index_utf16,
                            len_utf16,
                            attrs,
                            ..
                        } => target.format(txn, index_utf16, len_utf16, attrs),
                    }
                }
            }
            YrsMutationAction::InsertText {
                target,
                index_utf16,
                text,
                attrs,
                ..
            } => target.insert_with_attributes(txn, index_utf16, &text, attrs),
            YrsMutationAction::DeleteText {
                target,
                index_utf16,
                len_utf16,
                ..
            } => target.remove_range(txn, index_utf16, len_utf16),
            YrsMutationAction::FormatText {
                target,
                index_utf16,
                len_utf16,
                attrs,
                ..
            } => target.format(txn, index_utf16, len_utf16, attrs),
        }
    }
}

pub(crate) fn estimate_update_v1_growth(
    request_id: u64,
    plan: &YrsMutationPlan,
) -> OperationResult<usize> {
    let mut total = 0usize;
    for action in &plan.actions {
        // Includes worst-case client/clock varints, parent/type refs, item headers,
        // content lengths, format sentinels and delete-set clocks.
        let mut action_bytes = 512usize;
        match action {
            YrsMutationAction::CreateText {
                text,
                attrs,
                follow_up,
                ..
            } => {
                // XML type creation needs a parent reference and its own item header in
                // addition to the subsequently inserted attributed text items.
                action_bytes = checked_estimate_add(
                    request_id,
                    action.operation_index(),
                    action_bytes,
                    Some(512),
                )?;
                action_bytes = checked_estimate_add(
                    request_id,
                    action.operation_index(),
                    action_bytes,
                    text.len().checked_mul(4),
                )?;
                action_bytes = checked_estimate_add(
                    request_id,
                    action.operation_index(),
                    action_bytes,
                    attrs_estimate(attrs).and_then(|bytes| bytes.checked_mul(8)),
                )?;
                for follow in follow_up {
                    action_bytes = estimate_created_text_action(request_id, action_bytes, follow)?;
                }
            }
            YrsMutationAction::InsertText { text, attrs, .. } => {
                action_bytes = checked_estimate_add(
                    request_id,
                    action.operation_index(),
                    action_bytes,
                    text.len().checked_mul(4),
                )?;
                action_bytes = checked_estimate_add(
                    request_id,
                    action.operation_index(),
                    action_bytes,
                    attrs_estimate(attrs).and_then(|bytes| bytes.checked_mul(8)),
                )?;
            }
            YrsMutationAction::DeleteText { len_utf16, .. } => {
                action_bytes = checked_estimate_add(
                    request_id,
                    action.operation_index(),
                    action_bytes,
                    usize::try_from(*len_utf16)
                        .ok()
                        .and_then(|len| len.checked_mul(16)),
                )?;
            }
            YrsMutationAction::FormatText {
                len_utf16, attrs, ..
            } => {
                action_bytes = checked_estimate_add(
                    request_id,
                    action.operation_index(),
                    action_bytes,
                    usize::try_from(*len_utf16)
                        .ok()
                        .and_then(|len| len.checked_mul(16)),
                )?;
                action_bytes = checked_estimate_add(
                    request_id,
                    action.operation_index(),
                    action_bytes,
                    attrs_estimate(attrs).and_then(|bytes| bytes.checked_mul(16)),
                )?;
            }
        }
        total = checked_estimate_add(
            request_id,
            action.operation_index(),
            total,
            Some(action_bytes),
        )?;
    }
    Ok(total)
}

fn checked_estimate_add(
    request_id: u64,
    operation_index: usize,
    current: usize,
    amount: Option<usize>,
) -> OperationResult<usize> {
    amount
        .and_then(|amount| current.checked_add(amount))
        .ok_or_else(|| {
            OperationError::operation_limit_exceeded(
                request_id,
                Some(operation_index),
                "estimatedUpdateV1Growth",
                u64::MAX,
                u64::MAX,
            )
        })
}

fn estimate_created_text_action(
    request_id: u64,
    mut current: usize,
    action: &CreatedTextAction,
) -> OperationResult<usize> {
    current = checked_estimate_add(request_id, action.operation_index(), current, Some(512))?;
    match action {
        CreatedTextAction::Insert { text, attrs, .. } => {
            current = checked_estimate_add(
                request_id,
                action.operation_index(),
                current,
                text.len().checked_mul(4),
            )?;
            checked_estimate_add(
                request_id,
                action.operation_index(),
                current,
                attrs_estimate(attrs).and_then(|bytes| bytes.checked_mul(8)),
            )
        }
        CreatedTextAction::Delete { len_utf16, .. } => checked_estimate_add(
            request_id,
            action.operation_index(),
            current,
            usize::try_from(*len_utf16)
                .ok()
                .and_then(|len| len.checked_mul(16)),
        ),
        CreatedTextAction::Format {
            len_utf16, attrs, ..
        } => {
            current = checked_estimate_add(
                request_id,
                action.operation_index(),
                current,
                usize::try_from(*len_utf16)
                    .ok()
                    .and_then(|len| len.checked_mul(16)),
            )?;
            checked_estimate_add(
                request_id,
                action.operation_index(),
                current,
                attrs_estimate(attrs).and_then(|bytes| bytes.checked_mul(16)),
            )
        }
    }
}

fn attrs_estimate(attrs: &Attrs) -> Option<usize> {
    attrs.iter().try_fold(0usize, |total, (key, value)| {
        total
            .checked_add(key.len())
            .and_then(|bytes| bytes.checked_add(value.to_string().len()))
            .and_then(|bytes| bytes.checked_add(32))
    })
}

fn attrs_work(attrs: &Attrs) -> usize {
    attrs
        .iter()
        .try_fold(0usize, |total, (key, value)| {
            total
                .checked_add(1)
                .and_then(|work| work.checked_add(key.len()))
                .and_then(|work| work.checked_add(value.to_string().len()))
        })
        .unwrap_or(usize::MAX)
}

fn work_overflow(request_id: u64, operation_index: usize, limit: usize) -> OperationError {
    OperationError::operation_limit_exceeded(
        request_id,
        Some(operation_index),
        "maxActionsPerTransaction",
        u64::try_from(limit).unwrap_or(u64::MAX),
        u64::MAX,
    )
}

fn scan_overflow(request_id: u64, operation_index: usize, limit: usize) -> OperationError {
    OperationError::operation_limit_exceeded(
        request_id,
        Some(operation_index),
        "maxInputBytes",
        u64::try_from(limit).unwrap_or(u64::MAX),
        u64::MAX,
    )
}

fn validate_signature<T: ReadTxn>(
    request_id: u64,
    operation_index: usize,
    target: &XmlTextRef,
    expected: &TargetSignature,
    txn: &T,
    path_children: &mut std::collections::HashMap<BranchID, Vec<BranchID>>,
) -> OperationResult<()> {
    let branch = <XmlTextRef as AsRef<Branch>>::as_ref(target);
    let path_matches = expected_path_matches(
        branch.id(),
        target.parent(),
        &expected.path,
        txn,
        path_children,
    );
    let actual_len = Some(Text::len(target, txn));
    if branch.is_deleted()
        || branch.id() != expected.target
        || !path_matches
        || actual_len != Some(expected.initial_len_utf16)
    {
        return Err(OperationError::engine_invariant_failed(
            request_id,
            Some(operation_index),
            format!(
                "resolved Yrs XML text target signature changed before mutation (deleted={}, id_match={}, path_match={}, expected_utf16={}, actual_len={actual_len:?})",
                branch.is_deleted(),
                branch.id() == expected.target,
                path_matches,
                expected.initial_len_utf16,
            ),
        ));
    }
    Ok(())
}

fn validate_parent_identity<T: ReadTxn>(
    request_id: u64,
    operation_index: usize,
    parent: &XmlElementRef,
    expected: &ParentSignature,
    txn: &T,
    path_children: &mut std::collections::HashMap<BranchID, Vec<BranchID>>,
) -> OperationResult<()> {
    let branch = <XmlElementRef as AsRef<Branch>>::as_ref(parent);
    let path_matches = expected_path_matches(
        branch.id(),
        parent.parent(),
        &expected.path,
        txn,
        path_children,
    );
    if branch.is_deleted()
        || branch.id() != expected.parent
        || parent.tag() != &expected.tag
        || !path_matches
    {
        return Err(OperationError::engine_invariant_failed(
            request_id,
            Some(operation_index),
            "resolved empty Yrs textblock signature changed before mutation",
        ));
    }
    path_children
        .entry(branch.id())
        .or_insert_with(|| parent.children(txn).map(|child| child.id()).collect());
    Ok(())
}

fn validate_parent_gap(
    request_id: u64,
    operation_index: usize,
    expected: &ParentSignature,
    children: &[BranchID],
) -> OperationResult<()> {
    let actual_child_count = u32::try_from(children.len()).ok();
    let left_neighbor = expected
        .initial_child_index
        .checked_sub(1)
        .and_then(|index| usize::try_from(index).ok())
        .and_then(|index| children.get(index))
        .cloned();
    let right_neighbor = usize::try_from(expected.initial_child_index)
        .ok()
        .and_then(|index| children.get(index))
        .cloned();
    if actual_child_count != Some(expected.child_count)
        || expected.initial_child_index > expected.child_count
        || left_neighbor != expected.left_neighbor
        || right_neighbor != expected.right_neighbor
    {
        return Err(OperationError::engine_invariant_failed(
            request_id,
            Some(operation_index),
            "resolved empty Yrs textblock gap signature changed before mutation",
        ));
    }
    Ok(())
}

fn invalid_action_range(request_id: u64, operation_index: usize) -> OperationError {
    OperationError::engine_invariant_failed(
        request_id,
        Some(operation_index),
        "resolved Yrs mutation action is outside its preflighted UTF-16 target range",
    )
}

fn expected_path_matches<T: ReadTxn>(
    mut child_id: BranchID,
    mut parent: Option<XmlOut>,
    expected: &[(BranchID, u32)],
    txn: &T,
    path_children: &mut std::collections::HashMap<BranchID, Vec<BranchID>>,
) -> bool {
    for (expected_parent, expected_index) in expected {
        let Some(node) = parent else {
            return false;
        };
        if node.id() != *expected_parent {
            return false;
        }
        let children = path_children
            .entry(node.id())
            .or_insert_with(|| match &node {
                XmlOut::Element(element) => element.children(txn).map(|child| child.id()).collect(),
                XmlOut::Fragment(fragment) => {
                    fragment.children(txn).map(|child| child.id()).collect()
                }
                XmlOut::Text(_) => Vec::new(),
            });
        let expected_index = match usize::try_from(*expected_index) {
            Ok(index) => index,
            Err(_) => return false,
        };
        if children.get(expected_index) != Some(&child_id) {
            return false;
        }
        child_id = node.id();
        parent = match node {
            XmlOut::Element(element) => element.parent(),
            XmlOut::Fragment(fragment) => fragment.parent(),
            XmlOut::Text(text) => text.parent(),
        };
    }
    parent.is_none()
}

#[allow(clippy::too_many_arguments)]
fn collect_text_targets<'a, T: ReadTxn>(
    request_id: u64,
    txn: &T,
    children: impl Iterator<Item = (u32, XmlOut)> + 'a,
    parent_id: BranchID,
    ancestor_path: &[(BranchID, u32)],
    mut position: u32,
    schema: &Schema,
    traversal_work: &mut usize,
    output: &mut Vec<LocatedTarget>,
) -> OperationResult<u32> {
    for (child_index, child) in children {
        *traversal_work = traversal_work.checked_add(1).ok_or_else(|| {
            OperationError::engine_invariant_failed(
                request_id,
                None,
                "Yrs mutation target traversal work overflow",
            )
        })?;
        let mut child_path = Vec::with_capacity(ancestor_path.len() + 1);
        child_path.push((parent_id.clone(), child_index));
        child_path.extend_from_slice(ancestor_path);
        *traversal_work = traversal_work
            .checked_add(child_path.len())
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "Yrs mutation path capture work overflow",
                )
            })?;
        match child {
            XmlOut::Text(text) => {
                *traversal_work =
                    traversal_work
                        .checked_add(child_path.len())
                        .ok_or_else(|| {
                            OperationError::engine_invariant_failed(
                                request_id,
                                None,
                                "Yrs text signature preflight work overflow",
                            )
                        })?;
                let materialized = AsRef::<TextRef>::as_ref(&text).get_string(txn);
                let (len, utf16_len) = checked_text_lengths(request_id, None, &materialized)?;
                output.push(LocatedTarget::Existing {
                    start: position,
                    signature: TargetSignature {
                        target: <XmlTextRef as AsRef<Branch>>::as_ref(&text).id(),
                        path: child_path,
                        initial_len_utf16: utf16_len,
                    },
                    target: text,
                    text: materialized,
                    scalar_len: len,
                });
                position = position.checked_add(len).ok_or_else(|| {
                    OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "Yrs mutation target position overflow",
                    )
                })?;
            }
            XmlOut::Element(element) => {
                if schema
                    .node(element.tag().as_ref())
                    .is_some_and(|spec| spec.is_void)
                    || matches!(
                        element.tag().as_ref(),
                        "__opaque" | "__opaque_json" | "__skip"
                    )
                {
                    position = position.checked_add(1).ok_or_else(|| {
                        OperationError::engine_invariant_failed(
                            request_id,
                            None,
                            "Yrs void target position overflow",
                        )
                    })?;
                } else {
                    position = position.checked_add(1).ok_or_else(|| {
                        OperationError::engine_invariant_failed(
                            request_id,
                            None,
                            "Yrs element target position overflow",
                        )
                    })?;
                    let is_textblock = schema
                        .node(element.tag().as_ref())
                        .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock));
                    if is_textblock {
                        let children = element.children(txn).collect::<Vec<_>>();
                        *traversal_work = traversal_work
                            .checked_add(children.len())
                            .and_then(|work| work.checked_add(children.len()))
                            .and_then(|work| work.checked_add(child_path.len()))
                            .ok_or_else(|| {
                                OperationError::engine_invariant_failed(
                                    request_id,
                                    None,
                                    "Yrs textblock materialization work overflow",
                                )
                            })?;
                        let child_count = u32::try_from(children.len()).map_err(|_| {
                            OperationError::engine_invariant_failed(
                                request_id,
                                None,
                                "Yrs textblock child count exceeds u32",
                            )
                        })?;
                        let mut previous_was_text = false;
                        for (index, child) in children.iter().cloned().enumerate() {
                            let index = u32::try_from(index).map_err(|_| {
                                OperationError::engine_invariant_failed(
                                    request_id,
                                    None,
                                    "Yrs textblock child index exceeds u32",
                                )
                            })?;
                            let child_is_text = matches!(child, XmlOut::Text(_));
                            if !previous_was_text && !child_is_text {
                                *traversal_work = traversal_work
                                    .checked_add(child_path.len())
                                    .ok_or_else(|| {
                                        OperationError::engine_invariant_failed(
                                            request_id,
                                            None,
                                            "Yrs missing-gap signature work overflow",
                                        )
                                    })?;
                                output.push(LocatedTarget::Missing {
                                    start: position,
                                    parent: element.clone(),
                                    child_index: index,
                                    signature: parent_signature_from_children(
                                        &element,
                                        &child_path,
                                        &children,
                                        child_count,
                                        index,
                                    ),
                                });
                            }
                            position = collect_text_targets(
                                request_id,
                                txn,
                                std::iter::once((index, child)),
                                <XmlElementRef as AsRef<Branch>>::as_ref(&element).id(),
                                &child_path,
                                position,
                                schema,
                                traversal_work,
                                output,
                            )?;
                            previous_was_text = child_is_text;
                        }
                        if !previous_was_text {
                            *traversal_work = traversal_work
                                .checked_add(child_path.len())
                                .ok_or_else(|| {
                                    OperationError::engine_invariant_failed(
                                        request_id,
                                        None,
                                        "Yrs missing-gap signature work overflow",
                                    )
                                })?;
                            output.push(LocatedTarget::Missing {
                                start: position,
                                parent: element.clone(),
                                child_index: child_count,
                                signature: parent_signature_from_children(
                                    &element,
                                    &child_path,
                                    &children,
                                    child_count,
                                    child_count,
                                ),
                            });
                        }
                    } else {
                        position = collect_text_targets(
                            request_id,
                            txn,
                            (0u32..).zip(element.children(txn)),
                            <XmlElementRef as AsRef<Branch>>::as_ref(&element).id(),
                            &child_path,
                            position,
                            schema,
                            traversal_work,
                            output,
                        )?;
                    }
                    position = position.checked_add(1).ok_or_else(|| {
                        OperationError::engine_invariant_failed(
                            request_id,
                            None,
                            "Yrs element target position overflow",
                        )
                    })?;
                }
            }
            XmlOut::Fragment(fragment) => {
                position = collect_text_targets(
                    request_id,
                    txn,
                    (0u32..).zip(fragment.children(txn)),
                    <XmlFragmentRef as AsRef<Branch>>::as_ref(&fragment).id(),
                    &child_path,
                    position,
                    schema,
                    traversal_work,
                    output,
                )?;
            }
        }
    }
    Ok(position)
}

fn parent_signature_from_children(
    parent: &XmlElementRef,
    path: &[(BranchID, u32)],
    children: &[XmlOut],
    child_count: u32,
    child_index: u32,
) -> ParentSignature {
    ParentSignature {
        parent: <XmlElementRef as AsRef<Branch>>::as_ref(parent).id(),
        tag: parent.tag().clone(),
        path: path.to_vec(),
        child_count,
        initial_child_index: child_index,
        left_neighbor: child_index
            .checked_sub(1)
            .and_then(|index| children.get(index as usize))
            .map(XmlOut::id),
        right_neighbor: children.get(child_index as usize).map(XmlOut::id),
    }
}

fn checked_scalar_len(
    request_id: u64,
    operation_index: Option<usize>,
    text: &str,
) -> OperationResult<u32> {
    u32::try_from(text.chars().count()).map_err(|_| {
        OperationError::engine_invariant_failed(
            request_id,
            operation_index,
            "text scalar length exceeds u32",
        )
    })
}

fn checked_text_lengths(
    request_id: u64,
    operation_index: Option<usize>,
    text: &str,
) -> OperationResult<(u32, u32)> {
    text.chars().try_fold((0u32, 0u32), |(scalars, utf16), ch| {
        let scalars = scalars.checked_add(1).ok_or_else(|| {
            OperationError::engine_invariant_failed(
                request_id,
                operation_index,
                "text scalar length exceeds u32",
            )
        })?;
        let utf16 = utf16
            .checked_add(u32::from(ch.len_utf16() as u16))
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    operation_index,
                    "text UTF-16 length exceeds u32",
                )
            })?;
        Ok((scalars, utf16))
    })
}

fn scalar_to_utf16(
    request_id: u64,
    operation_index: usize,
    text: &str,
    scalar: u32,
) -> OperationResult<u32> {
    super::scalar_offset_to_utf16(text, scalar).ok_or_else(|| {
        OperationError::engine_invariant_failed(
            request_id,
            Some(operation_index),
            "resolved scalar offset is outside its Yrs XML text target",
        )
    })
}

fn scalar_byte_index(text: &str, scalar: u32) -> Option<usize> {
    if scalar == 0 {
        return Some(0);
    }
    text.char_indices()
        .nth(usize::try_from(scalar).ok()?)
        .map(|(index, _)| index)
        .or_else(|| (text.chars().count() == scalar as usize).then_some(text.len()))
}

fn insert_scalar(
    request_id: u64,
    operation_index: usize,
    target: &mut String,
    scalar: u32,
    text: &str,
) -> OperationResult<()> {
    let byte = scalar_byte_index(target, scalar).ok_or_else(|| {
        OperationError::engine_invariant_failed(
            request_id,
            Some(operation_index),
            "resolved insertion scalar offset is outside virtual target",
        )
    })?;
    target.insert_str(byte, text);
    Ok(())
}

fn remove_scalar_range(
    request_id: u64,
    operation_index: usize,
    target: &mut String,
    from: u32,
    to: u32,
) -> OperationResult<()> {
    let from_byte = scalar_byte_index(target, from)
        .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
    let to_byte = scalar_byte_index(target, to)
        .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
    target.replace_range(from_byte..to_byte, "");
    Ok(())
}

fn marks_to_attrs(marks: &[Mark]) -> Attrs {
    marks
        .iter()
        .map(|mark| {
            let value = if mark.attrs().is_empty() {
                Any::Bool(true)
            } else {
                json_to_any(&Value::Object(mark.attrs().clone().into_iter().collect()))
            };
            (Arc::<str>::from(mark.mark_type()), value)
        })
        .collect()
}

pub(crate) fn mark_attr(mark: &Mark) -> Attrs {
    marks_to_attrs(std::slice::from_ref(mark))
}

pub(crate) fn removed_mark_attr(mark_type: &str) -> Attrs {
    Attrs::from([(Arc::<str>::from(mark_type), Any::Null)])
}

fn json_to_any(value: &Value) -> Any {
    match value {
        Value::Null => Any::Null,
        Value::Bool(value) => Any::Bool(*value),
        Value::Number(number) => number
            .as_i64()
            .map(Any::BigInt)
            .or_else(|| number.as_f64().map(Any::Number))
            .unwrap_or(Any::Null),
        Value::String(value) => Any::String(value.clone().into()),
        Value::Array(values) => Any::Array(values.iter().map(json_to_any).collect()),
        Value::Object(values) => Any::Map(Arc::new(
            values
                .iter()
                .map(|(key, value)| (key.clone(), json_to_any(value)))
                .collect(),
        )),
    }
}

fn inline_text_pieces(
    request_id: u64,
    operation_index: usize,
    fragment: &Fragment,
) -> OperationResult<Vec<(String, Vec<Mark>)>> {
    let mut output = Vec::new();
    for node in fragment.iter() {
        collect_inline_piece(request_id, operation_index, node, &mut output)?;
    }
    Ok(output)
}

fn collect_inline_piece(
    request_id: u64,
    operation_index: usize,
    node: &Node,
    output: &mut Vec<(String, Vec<Mark>)>,
) -> OperationResult<()> {
    if let Some(text) = node.text_str() {
        output.push((text.to_owned(), node.marks().to_vec()));
        return Ok(());
    }
    Err(OperationError::operation_invalid(
        request_id,
        operation_index,
        "content",
        "replacement content must resolve to existing XML text targets",
    ))
}
