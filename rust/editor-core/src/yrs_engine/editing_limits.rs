use super::{OperationError, OperationResult, YrsEngineError, YrsEngineResult};

pub const HARD_MAX_OPERATIONS_PER_TRANSACTION: usize = 4_096;
pub const HARD_MAX_UNDO_GROUPS: usize = 2_000;
pub const HARD_MAX_UNDO_RETAINED_UNITS: u64 = 8_000_000;
pub const HARD_MAX_DERIVED_OUTPUT_BYTES: usize = 128 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditingLimits {
    pub max_operations_per_transaction: usize,
    pub max_undo_groups: usize,
    pub max_undo_retained_units: u64,
    pub max_derived_output_bytes: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
// Not reachable from production call paths after the Task 16C legacy runtime
// removal; exercised by crate tests.
#[allow(dead_code)]
pub struct EditingLimitOverrides {
    pub max_operations_per_transaction: Option<usize>,
    pub max_undo_groups: Option<usize>,
    pub max_undo_retained_units: Option<u64>,
    pub max_derived_output_bytes: Option<usize>,
}

impl Default for EditingLimits {
    fn default() -> Self {
        Self {
            max_operations_per_transaction: 256,
            max_undo_groups: 500,
            max_undo_retained_units: 1_000_000,
            max_derived_output_bytes: 32 * 1024 * 1024,
        }
    }
}

impl EditingLimits {
    // Not reachable from production call paths after the Task 16C legacy runtime
    // removal; exercised by crate tests.
    #[allow(dead_code)]
    pub fn resolve(overrides: EditingLimitOverrides) -> YrsEngineResult<Self> {
        let defaults = Self::default();
        let limits = Self {
            max_operations_per_transaction: overrides
                .max_operations_per_transaction
                .unwrap_or(defaults.max_operations_per_transaction),
            max_undo_groups: overrides
                .max_undo_groups
                .unwrap_or(defaults.max_undo_groups),
            max_undo_retained_units: overrides
                .max_undo_retained_units
                .unwrap_or(defaults.max_undo_retained_units),
            max_derived_output_bytes: overrides
                .max_derived_output_bytes
                .unwrap_or(defaults.max_derived_output_bytes),
        };

        limits.validate()?;

        Ok(limits)
    }

    pub(crate) fn validate(&self) -> YrsEngineResult<()> {
        validate_limit(
            "maxOperationsPerTransaction",
            self.max_operations_per_transaction as u64,
            HARD_MAX_OPERATIONS_PER_TRANSACTION as u64,
        )?;
        validate_limit(
            "maxUndoGroups",
            self.max_undo_groups as u64,
            HARD_MAX_UNDO_GROUPS as u64,
        )?;
        validate_limit(
            "maxUndoRetainedUnits",
            self.max_undo_retained_units,
            HARD_MAX_UNDO_RETAINED_UNITS,
        )?;
        validate_limit(
            "maxDerivedOutputBytes",
            self.max_derived_output_bytes as u64,
            HARD_MAX_DERIVED_OUTPUT_BYTES as u64,
        )?;

        Ok(())
    }
}

fn validate_limit(field: &'static str, actual: u64, ceiling: u64) -> YrsEngineResult<()> {
    if actual == 0 || actual > ceiling {
        return Err(YrsEngineError {
            code: "INVALID_RESOURCE_LIMIT",
            message: format!("{field} must be a positive integer no greater than {ceiling}"),
            limit: Some(usize::try_from(ceiling).unwrap_or(usize::MAX)),
            actual: Some(usize::try_from(actual).unwrap_or(usize::MAX)),
            details: Some(serde_json::json!({ "field": field })),
        });
    }
    Ok(())
}

#[derive(Debug, Default)]
#[allow(dead_code)] // Consumed by the transaction compiler introduced in the next task.
pub(crate) struct CheckedWork {
    operations: usize,
    actions: usize,
    output_bytes: usize,
    undo_units: u64,
}

#[allow(dead_code)] // Consumed by the transaction compiler introduced in the next task.
impl CheckedWork {
    pub(crate) fn charge_operations(
        &mut self,
        request_id: u64,
        amount: usize,
        limit: usize,
    ) -> OperationResult<()> {
        let actual = self.operations.checked_add(amount);
        self.operations = charge_usize(
            actual,
            request_id,
            None,
            "maxOperationsPerTransaction",
            limit,
            false,
        )?;
        Ok(())
    }

    pub(crate) fn charge_actions(
        &mut self,
        request_id: u64,
        operation_index: usize,
        amount: usize,
        limit: usize,
    ) -> OperationResult<()> {
        let actual = self.actions.checked_add(amount);
        self.actions = charge_usize(
            actual,
            request_id,
            Some(operation_index),
            "maxActionsPerTransaction",
            limit,
            false,
        )?;
        Ok(())
    }

    pub(crate) fn charge_output_bytes(
        &mut self,
        request_id: u64,
        operation_index: usize,
        amount: usize,
        limit: usize,
    ) -> OperationResult<()> {
        let actual = self.output_bytes.checked_add(amount);
        self.output_bytes = charge_usize(
            actual,
            request_id,
            Some(operation_index),
            "maxDerivedOutputBytes",
            limit,
            true,
        )?;
        Ok(())
    }

    pub(crate) fn charge_undo_units(
        &mut self,
        request_id: u64,
        amount: u64,
        limit: u64,
    ) -> OperationResult<()> {
        let checked_actual = self.undo_units.checked_add(amount);
        let actual = checked_actual.unwrap_or(u64::MAX);
        if checked_actual.is_none() || actual > limit {
            return Err(OperationError::operation_limit_exceeded(
                request_id,
                None,
                "maxUndoRetainedUnits",
                limit,
                actual,
            ));
        }
        self.undo_units = actual;
        Ok(())
    }
}

#[allow(dead_code)] // Consumed through CheckedWork in the next task.
fn charge_usize(
    actual: Option<usize>,
    request_id: u64,
    operation_index: Option<usize>,
    field: &'static str,
    limit: usize,
    document_limit: bool,
) -> OperationResult<usize> {
    let overflowed = actual.is_none();
    let actual = actual.unwrap_or(usize::MAX);
    if !overflowed && actual <= limit {
        return Ok(actual);
    }

    let limit = u64::try_from(limit).unwrap_or(u64::MAX);
    let actual = u64::try_from(actual).unwrap_or(u64::MAX);
    let error = if document_limit {
        OperationError::document_limit_exceeded(request_id, operation_index, field, limit, actual)
    } else {
        OperationError::operation_limit_exceeded(request_id, operation_index, field, limit, actual)
    };
    Err(error)
}

#[cfg(test)]
mod tests {
    use super::CheckedWork;

    #[test]
    fn checked_work_charges_exact_limits_and_rejects_one_over() {
        let mut work = CheckedWork::default();
        work.charge_operations(9, 2, 2).unwrap();
        let error = work.charge_operations(9, 1, 2).unwrap_err();
        assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some(2));
        assert_eq!(error.actual, Some(3));
        assert_eq!(
            error.details,
            Some(serde_json::json!({ "field": "maxOperationsPerTransaction" }))
        );

        let mut work = CheckedWork::default();
        work.charge_actions(9, 4, 3, 3).unwrap();
        let error = work.charge_actions(9, 4, 1, 3).unwrap_err();
        assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
        assert_eq!(error.operation_index, Some(4));
        assert_eq!(
            error.details,
            Some(serde_json::json!({ "field": "maxActionsPerTransaction" }))
        );

        let mut work = CheckedWork::default();
        work.charge_output_bytes(9, 4, 8, 8).unwrap();
        let error = work.charge_output_bytes(9, 4, 1, 8).unwrap_err();
        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(
            error.details,
            Some(serde_json::json!({ "field": "maxDerivedOutputBytes" }))
        );

        let mut work = CheckedWork::default();
        work.charge_undo_units(9, 5, 5).unwrap();
        let error = work.charge_undo_units(9, 1, 5).unwrap_err();
        assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
        assert_eq!(
            error.details,
            Some(serde_json::json!({ "field": "maxUndoRetainedUnits" }))
        );
    }

    #[test]
    fn checked_work_converts_counter_overflow_to_limit_errors() {
        let mut work = CheckedWork::default();
        work.charge_operations(9, usize::MAX, usize::MAX).unwrap();
        let error = work.charge_operations(9, 1, usize::MAX).unwrap_err();
        assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
        assert_eq!(error.actual, Some(u64::MAX));

        let mut work = CheckedWork::default();
        work.charge_undo_units(9, u64::MAX, u64::MAX).unwrap();
        let error = work.charge_undo_units(9, 1, u64::MAX).unwrap_err();
        assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
        assert_eq!(error.actual, Some(u64::MAX));
    }
}
