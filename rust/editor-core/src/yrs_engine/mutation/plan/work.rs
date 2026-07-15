pub(super) fn binary_partition_work(len: usize) -> usize {
    if len == 0 {
        0
    } else {
        usize::BITS as usize - len.leading_zeros() as usize
    }
}
pub(super) fn fenwick_prefix(tree: &[u32], mut index: usize) -> Option<u32> {
    let mut total = 0u32;
    while index > 0 {
        total = total.checked_add(tree[index])?;
        index &= index - 1;
    }
    Some(total)
}

pub(super) fn fenwick_add(tree: &mut [u32], mut index: usize) -> Option<()> {
    while index < tree.len() {
        tree[index] = tree[index].checked_add(1)?;
        let step = index & index.wrapping_neg();
        index = index.checked_add(step)?;
    }
    Some(())
}

pub(super) fn work_overflow(
    request_id: u64,
    operation_index: usize,
    limit: usize,
) -> OperationError {
    OperationError::operation_limit_exceeded(
        request_id,
        Some(operation_index),
        "maxActionsPerTransaction",
        u64::try_from(limit).unwrap_or(u64::MAX),
        u64::MAX,
    )
}
pub(super) fn scan_overflow(
    request_id: u64,
    operation_index: usize,
    limit: usize,
) -> OperationError {
    OperationError::operation_limit_exceeded(
        request_id,
        Some(operation_index),
        "maxInputBytes",
        u64::try_from(limit).unwrap_or(u64::MAX),
        u64::MAX,
    )
}
