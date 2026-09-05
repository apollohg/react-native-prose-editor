use crate::model::{Document, Node};
use crate::schema::Schema;
use crate::yrs_engine::mutation::{
    LocalizedFormatCompiler, LocalizedRootWindowCompiler, MutationCompiler,
};
use crate::yrs_engine::{OperationError, OperationResult};

pub(super) trait TextBoundaryWorkCharger {
    fn charge_boundary_node(&mut self, operation_index: usize) -> OperationResult<()>;
    fn charge_boundary_text(
        &mut self,
        operation_index: usize,
        text_bytes: usize,
    ) -> OperationResult<()>;
}

impl TextBoundaryWorkCharger for MutationCompiler {
    fn charge_boundary_node(&mut self, operation_index: usize) -> OperationResult<()> {
        MutationCompiler::charge_boundary_node(self, operation_index)
    }

    fn charge_boundary_text(
        &mut self,
        operation_index: usize,
        text_bytes: usize,
    ) -> OperationResult<()> {
        MutationCompiler::charge_boundary_text(self, operation_index, text_bytes)
    }
}

impl TextBoundaryWorkCharger for LocalizedFormatCompiler {
    fn charge_boundary_node(&mut self, operation_index: usize) -> OperationResult<()> {
        self.charge_format_boundary_node(operation_index)
    }

    fn charge_boundary_text(
        &mut self,
        operation_index: usize,
        text_bytes: usize,
    ) -> OperationResult<()> {
        self.charge_format_boundary_text(operation_index, text_bytes)
    }
}

impl TextBoundaryWorkCharger for LocalizedRootWindowCompiler {
    fn charge_boundary_node(&mut self, operation_index: usize) -> OperationResult<()> {
        LocalizedRootWindowCompiler::charge_boundary_node(self, operation_index)
    }

    fn charge_boundary_text(
        &mut self,
        operation_index: usize,
        text_bytes: usize,
    ) -> OperationResult<()> {
        LocalizedRootWindowCompiler::charge_boundary_text(self, operation_index, text_bytes)
    }
}

pub(super) fn text_boundaries<C: TextBoundaryWorkCharger>(
    request_id: u64,
    operation_index: usize,
    document: &Document,
    schema: &Schema,
    lowering: &mut C,
) -> OperationResult<Vec<u32>> {
    fn visit<C: TextBoundaryWorkCharger>(
        request_id: u64,
        operation_index: usize,
        node: &Node,
        schema: &Schema,
        lowering: &mut C,
        position: &mut u32,
        output: &mut Vec<u32>,
    ) -> OperationResult<()> {
        lowering.charge_boundary_node(operation_index)?;
        if let Some(text) = node.text_str() {
            output.push(*position);
            lowering.charge_boundary_text(operation_index, text.len())?;
            let len = u32::try_from(text.chars().count()).map_err(|_| {
                OperationError::engine_invariant_failed(
                    request_id,
                    Some(operation_index),
                    "preview text scalar length exceeds u32",
                )
            })?;
            *position = position.checked_add(len).ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    Some(operation_index),
                    "preview text boundary overflow",
                )
            })?;
            output.push(*position);
            return Ok(());
        }
        if let Some(content) = node.content() {
            let is_document = node.node_type() == schema.doc_node_type();
            if !is_document {
                *position = position.checked_add(1).ok_or_else(|| {
                    OperationError::engine_invariant_failed(
                        request_id,
                        Some(operation_index),
                        "preview node boundary overflow",
                    )
                })?;
            }
            for child in content.iter() {
                visit(
                    request_id,
                    operation_index,
                    child,
                    schema,
                    lowering,
                    position,
                    output,
                )?;
            }
            if !is_document {
                *position = position.checked_add(1).ok_or_else(|| {
                    OperationError::engine_invariant_failed(
                        request_id,
                        Some(operation_index),
                        "preview node boundary overflow",
                    )
                })?;
            }
        } else {
            *position = position.checked_add(1).ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    Some(operation_index),
                    "preview leaf boundary overflow",
                )
            })?;
        }
        Ok(())
    }

    let mut output = Vec::new();
    let mut position = 0u32;
    visit(
        request_id,
        operation_index,
        document.root(),
        schema,
        lowering,
        &mut position,
        &mut output,
    )?;
    output.sort_unstable();
    output.dedup();
    Ok(output)
}
