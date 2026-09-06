#[cfg(test)]
mod localized_insert_tests {
    use serde_json::{json, Value};
    use yrs::{Doc, OffsetKind, Options, Transact, WriteTxn};

    use super::*;
    use crate::position::PositionMap;
    use crate::schema::presets::tiptap_schema;
    use crate::serialize::{from_prosemirror_json, UnknownTypeMode};
    use crate::transform::{apply_step_canonical_marks, Step};
    use crate::yrs_engine::codec::YrsDocumentCodec;
    use crate::yrs_engine::mutation::{execute_mutation_plan, preflight_mutation_plan};

    fn utf16_doc() -> Doc {
        Doc::with_options(Options {
            offset_kind: OffsetKind::Utf16,
            ..Options::default()
        })
    }

    fn seeded_document(source: &Value, schema: &Schema, limits: &ResourceLimits) -> Doc {
        let doc = utf16_doc();
        let codec = YrsDocumentCodec::new(schema, limits);
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        codec
            .apply_json(
                &fragment,
                &mut txn,
                &json!({ "type": schema.doc_node_type() }),
                source,
            )
            .unwrap();
        drop(txn);
        doc
    }

    #[test]
    fn localized_root_signature_accepts_projected_wire_tags() {
        let schema = projected_textblock_test_schema();
        let limits = ResourceLimits::default();
        let source = json!({
            "type": "doc",
            "content": [{
                "type": "callout",
                "attrs": { "tone": "info" },
                "content": [{ "type": "text", "text": "projected" }]
            }]
        });
        let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
        let doc = seeded_document(&source, &schema, &limits);
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();

        assert!(localized_root_structural_parent(
            719, &txn, &fragment, &document, &schema, &limits,
        )
        .unwrap()
        .is_some());
    }

    #[derive(Debug, PartialEq)]
    struct InsertActionView<'a> {
        signature: &'a TargetSignature,
        index_utf16: u32,
        text: &'a str,
        len_utf16: u32,
        attrs: &'a Attrs,
        operation_index: usize,
    }

    #[derive(Debug, PartialEq)]
    struct FormatActionView<'a> {
        signature: &'a TargetSignature,
        index_utf16: u32,
        len_utf16: u32,
        attrs: &'a Attrs,
        operation_index: usize,
    }

    #[derive(Debug, PartialEq)]
    enum TextActionView<'a> {
        Insert(InsertActionView<'a>),
        Format(FormatActionView<'a>),
    }

    fn action_signature(action: &YrsMutationAction) -> InsertActionView<'_> {
        let YrsMutationAction::InsertText {
            index_utf16,
            text,
            len_utf16,
            attrs,
            signature,
            operation_index,
            ..
        } = action
        else {
            panic!("expected InsertText action")
        };
        InsertActionView {
            signature,
            index_utf16: *index_utf16,
            text,
            len_utf16: *len_utf16,
            attrs,
            operation_index: *operation_index,
        }
    }

    fn text_action_view(action: &YrsMutationAction) -> TextActionView<'_> {
        match action {
            YrsMutationAction::InsertText { .. } => {
                TextActionView::Insert(action_signature(action))
            }
            YrsMutationAction::FormatText {
                index_utf16,
                len_utf16,
                attrs,
                signature,
                operation_index,
                ..
            } => TextActionView::Format(FormatActionView {
                signature,
                index_utf16: *index_utf16,
                len_utf16: *len_utf16,
                attrs,
                operation_index: *operation_index,
            }),
            _ => panic!("expected InsertText or FormatText action"),
        }
    }

    fn compile_pair_at_block_offset(
        source: &Value,
        schema: &Schema,
        block_index: usize,
        block_offset: u32,
        inserted: &str,
    ) -> (Doc, YrsMutationPlan, YrsMutationPlan, MutationCompilerBuild) {
        let limits = ResourceLimits::default();
        let document = from_prosemirror_json(source, schema, UnknownTypeMode::Preserve).unwrap();
        let position_map = PositionMap::build(&document, schema);
        let block = position_map.block(block_index).unwrap();
        let position = block.doc_start + block_offset;
        let doc = seeded_document(source, schema, &limits);
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let mut eager =
            MutationCompiler::new(702, &txn, &fragment, schema, 100_000, 100_000, 11).unwrap();
        let (mut localized, mode) = MutationCompiler::new_localized_insert_or_eager(
            702,
            &txn,
            &fragment,
            schema,
            100_000,
            100_000,
            11,
            LocalizedInsertLocator {
                document: &document,
                block_path: block.node_path.as_slice(),
                position,
            },
        )
        .unwrap();
        eager.insert(0, position, inserted, &[]).unwrap();
        localized.insert(0, position, inserted, &[]).unwrap();
        let eager = eager.finish(Some(0)).unwrap();
        let localized = localized.finish(Some(0)).unwrap();
        preflight_mutation_plan(702, &eager, &txn).unwrap();
        preflight_mutation_plan(702, &localized, &txn).unwrap();
        drop(txn);
        (doc, eager, localized, mode)
    }

    fn assert_insert_plans_equal(eager: &YrsMutationPlan, localized: &YrsMutationPlan) {
        assert_eq!(eager.actions.len(), 1);
        assert_eq!(localized.actions.len(), 1);
        assert_plans_equal(eager, localized);
    }

    fn assert_plans_equal(eager: &YrsMutationPlan, localized: &YrsMutationPlan) {
        assert_eq!(
            eager
                .actions
                .iter()
                .map(text_action_view)
                .collect::<Vec<_>>(),
            localized
                .actions
                .iter()
                .map(text_action_view)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            eager.compilation_work_for_test(),
            localized.compilation_work_for_test()
        );
        assert_eq!(
            eager.expected_preflight_work_for_test(),
            localized.expected_preflight_work_for_test()
        );
        assert_eq!(eager.scan_work, localized.scan_work);
        assert_eq!(
            eager.position_resolver_work_for_test(),
            localized.position_resolver_work_for_test()
        );
    }

    include!("localized/root_window.rs");

    include!("localized/root_window_seals.rs");

    include!("localized/insert.rs");

    include!("localized/promotion.rs");

    include!("localized/format.rs");

    include!("localized/seed_context.rs");
}
