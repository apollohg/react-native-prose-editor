use std::collections::HashMap;

use serde_json::json;

use super::{
    canonicalize_yrs_document, canonicalize_yrs_document_with_evidence,
    reset_mark_set_hash_allocations_for_test, take_mark_set_hash_allocations_for_test,
    validate_canonical_marks, validate_canonical_marks_with_evidence,
    validate_importable_marks_with_evidence, validate_input_mark_set, validate_mark_set,
    DocumentValidator,
};
use crate::boundary::ResourceLimits;
use crate::model::{Document, Fragment, Mark, Node};
use crate::schema::content_rule::WorkBudget;
use crate::schema::presets::tiptap_schema;
use crate::schema::{MarkSpec, Schema};
use crate::serialize::{from_prosemirror_json, UnknownTypeMode};

#[test]
fn document_validation_preserves_nested_empty_and_opaque_work_metrics() {
    let schema = tiptap_schema();
    let document = from_prosemirror_json(
        &json!({
            "type": "doc",
            "content": [
                {
                    "type": "blockquote",
                    "content": [{
                        "type": "paragraph",
                        "content": [{"type": "text", "text": "nested"}]
                    }]
                },
                {
                    "type": "bulletList",
                    "content": [{
                        "type": "listItem",
                        "content": [{
                            "type": "paragraph",
                            "content": [{"type": "text", "text": "item"}]
                        }]
                    }]
                },
                {"type": "paragraph"},
                {"type": "futureBlock", "attrs": {"payload": "opaque"}},
                {"type": "horizontalRule"}
            ]
        }),
        &schema,
        UnknownTypeMode::Preserve,
    )
    .unwrap();
    let report =
        DocumentValidator::validate_report(&document, &schema, &ResourceLimits::default()).unwrap();

    assert_eq!(report.stats.node_count, 11);
    assert_eq!(report.stats.max_depth, 5);
    assert!(report.metrics.metadata_bytes > 0);
    assert_eq!(report.metrics.validation_work, 104);
}

#[test]
fn opaque_json_cannot_hide_a_projected_schema_node() {
    let unknown_schema = tiptap_schema();
    let opaque = from_prosemirror_json(
        &json!({
            "type": "doc",
            "content": [{ "type": "callout", "attrs": { "tone": "info" } }]
        }),
        &unknown_schema,
        UnknownTypeMode::Preserve,
    )
    .unwrap();
    let projected_schema = Schema::from_json(&json!({
        "nodes": [
            { "name": "doc", "content": "block+", "role": "doc" },
            {
                "name": "info-box", "content": "", "group": "block", "role": "block",
                "json": { "type": "callout", "attrs": { "tone": "info" } }
            },
            { "name": "text", "content": "", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();

    let error = DocumentValidator::validate(&opaque, &projected_schema, &ResourceLimits::default())
        .unwrap_err();
    assert!(error.message.contains("normalizes to a known schema node"));
}

#[test]
fn direct_content_match_preserves_invalid_child_type_order_work_and_error() {
    let schema = tiptap_schema();
    let document = from_prosemirror_json(
        &json!({
            "type": "doc",
            "content": [{
                "type": "bulletList",
                "content": [
                    {"type": "paragraph"},
                    {"type": "listItem", "content": [{"type": "paragraph"}]}
                ]
            }]
        }),
        &schema,
        UnknownTypeMode::Error,
    )
    .unwrap();
    let limits = ResourceLimits::default();
    let work_limit = limits.max_document_nodes.saturating_mul(128);
    let budget = WorkBudget::new(work_limit);
    let error =
        DocumentValidator::validate_with_budget(&document, &schema, &limits, &budget, work_limit)
            .unwrap_err();

    assert_eq!(error.code, "DOCUMENT_INVALID");
    assert_eq!(
        error.message,
        "node 'bulletList' content [paragraph, listItem] does not match its content expression"
    );
    assert_eq!(budget.consumed(work_limit), 15);
}

#[test]
fn canonicalization_preserves_exact_root_identity_for_an_already_canonical_tree() {
    let schema = tiptap_schema();
    let document = from_prosemirror_json(
        &json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [
                    {
                        "type": "text",
                        "text": "ordered",
                        "marks": [{"type": "bold"}, {"type": "italic"}]
                    },
                    {"type": "hardBreak"},
                    {"type": "text", "text": "tail"}
                ]
            }]
        }),
        &schema,
        UnknownTypeMode::Error,
    )
    .unwrap();

    let canonical = canonicalize_yrs_document(&document, &schema);

    assert_eq!(canonical, document);
    assert!(canonical.shares_root_storage_with(&document));
}

#[test]
fn root_bound_canonical_evidence_skips_the_separate_identity_predicate() {
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };

    let schema = tiptap_schema();
    let document = from_prosemirror_json(
        &json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{"type": "text", "text": "canonical"}]
            }]
        }),
        &schema,
        UnknownTypeMode::Error,
    )
    .unwrap();

    reset_full_pass_counts_for_test();
    let evidence = validate_canonical_marks_with_evidence(&document, &schema).unwrap();
    let canonical = canonicalize_yrs_document_with_evidence(&document, &schema, evidence);
    let counts = take_full_pass_counts_for_test();

    assert!(canonical.shares_root_storage_with(&document));
    assert_eq!(counts.canonical_mark_validation_attempts, 1);
    assert_eq!(counts.canonical_mark_validation_completions, 1);
    assert_eq!(counts.canonical_mark_nodes_visited, 3);
    assert_eq!(counts.canonical_identity_predicate_nodes_visited, 0);
}

#[test]
fn canonical_evidence_rejects_an_equal_but_distinct_root_and_uses_the_fallback() {
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };

    let schema = tiptap_schema();
    let input = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{"type": "text", "text": "same value"}]
        }]
    });
    let source = from_prosemirror_json(&input, &schema, UnknownTypeMode::Error).unwrap();
    let equal_distinct = from_prosemirror_json(&input, &schema, UnknownTypeMode::Error).unwrap();
    assert_eq!(source, equal_distinct);
    assert!(!source.shares_root_storage_with(&equal_distinct));
    let evidence = validate_canonical_marks_with_evidence(&source, &schema).unwrap();

    reset_full_pass_counts_for_test();
    let canonical = canonicalize_yrs_document_with_evidence(&equal_distinct, &schema, evidence);
    let counts = take_full_pass_counts_for_test();

    assert!(canonical.shares_root_storage_with(&equal_distinct));
    assert!(!canonical.shares_root_storage_with(&source));
    assert_eq!(counts.canonical_identity_predicate_nodes_visited, 3);
}

#[test]
fn canonical_evidence_rejects_the_same_root_under_a_differently_ranked_schema() {
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };

    let source_schema = tiptap_schema();
    let mut reversed_marks = source_schema.all_marks().cloned().collect::<Vec<_>>();
    reversed_marks.swap(0, 1);
    let reversed_schema = Schema::new(source_schema.all_nodes().cloned().collect(), reversed_marks);
    let input = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "ranked",
                "marks": [{"type": "bold"}, {"type": "italic"}]
            }]
        }]
    });
    let document = from_prosemirror_json(&input, &source_schema, UnknownTypeMode::Error).unwrap();
    let evidence = validate_canonical_marks_with_evidence(&document, &source_schema).unwrap();

    reset_full_pass_counts_for_test();
    let canonical = canonicalize_yrs_document_with_evidence(&document, &reversed_schema, evidence);
    let counts = take_full_pass_counts_for_test();

    assert!(!canonical.shares_root_storage_with(&document));
    assert_eq!(counts.canonical_identity_predicate_nodes_visited, 3);
    assert_eq!(
        crate::serialize::to_prosemirror_json(&canonical, &reversed_schema),
        json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "ranked",
                    "marks": [{"type": "italic"}, {"type": "bold"}]
                }]
            }]
        })
    );
}

#[test]
fn canonical_evidence_detects_structural_rewrite_triggers_at_any_depth() {
    let schema = tiptap_schema();
    let cases = [
        json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [
                    {"type": "text", "text": ""},
                    {"type": "hardBreak"}
                ]
            }]
        }),
        json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [
                    {"type": "text", "text": "a"},
                    {"type": "text", "text": "b"}
                ]
            }]
        }),
        json!({
            "type": "doc",
            "content": [{
                "type": "blockquote",
                "content": [{
                    "type": "paragraph",
                    "content": [
                        {"type": "text", "text": "nested"},
                        {"type": "text", "text": " tail"}
                    ]
                }]
            }]
        }),
    ];

    for input in cases {
        let document = from_prosemirror_json(&input, &schema, UnknownTypeMode::Error).unwrap();
        let evidence = validate_canonical_marks_with_evidence(&document, &schema).unwrap();
        assert!(!evidence.is_canonical);
        let canonical = canonicalize_yrs_document_with_evidence(&document, &schema, evidence);
        assert!(!canonical.shares_root_storage_with(&document));
    }
}

#[test]
fn normalization_evidence_never_short_circuits_later_mark_validation_errors() {
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };

    let schema = tiptap_schema();
    let document = Document::new(Node::element(
        "doc".into(),
        HashMap::new(),
        Fragment::from(vec![Node::element(
            "paragraph".into(),
            HashMap::new(),
            Fragment::from(vec![
                Node::text(String::new(), Vec::new()),
                Node::text(
                    "invalid".into(),
                    vec![Mark::new("notInSchema".into(), HashMap::new())],
                ),
            ]),
        )]),
    ));

    reset_full_pass_counts_for_test();
    let error = validate_canonical_marks_with_evidence(&document, &schema).unwrap_err();
    let counts = take_full_pass_counts_for_test();

    assert_eq!(error.code, "UNKNOWN_MARK");
    assert_eq!(error.message, "unknown mark 'notInSchema'");
    assert_eq!(counts.canonical_mark_validation_attempts, 1);
    assert_eq!(counts.canonical_mark_validation_completions, 0);
    assert_eq!(counts.canonical_mark_nodes_visited, 4);
}

#[test]
fn noncanonical_mark_order_preserves_its_exact_error_instead_of_minting_evidence() {
    let schema = tiptap_schema();
    let document = from_prosemirror_json(
        &json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "ordered",
                    "marks": [{"type": "italic"}, {"type": "bold"}]
                }]
            }]
        }),
        &schema,
        UnknownTypeMode::Error,
    )
    .unwrap();

    let error = match validate_canonical_marks_with_evidence(&document, &schema) {
        Ok(_) => panic!("noncanonical order must not mint canonicality evidence"),
        Err(error) => error,
    };

    assert_eq!(error.code, "DOCUMENT_INVALID");
    assert_eq!(
        error.message,
        "mark order does not match ProseMirror schema rank"
    );
    assert_eq!(
        error.details,
        Some(json!({"field": "marks", "reason": "nonCanonicalOrder"}))
    );
}

#[test]
fn canonicalization_identity_proof_preserves_equal_comparator_mark_order() {
    let schema = tiptap_schema();
    let input = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "stable",
                "marks": [
                    {"type": "link", "attrs": {"href": "/first"}},
                    {"type": "link", "attrs": {"href": "/second"}}
                ]
            }]
        }]
    });
    let document = from_prosemirror_json(&input, &schema, UnknownTypeMode::Error).unwrap();

    let canonical = canonicalize_yrs_document(&document, &schema);

    assert!(canonical.shares_root_storage_with(&document));
    assert_eq!(
        crate::serialize::to_prosemirror_json(&canonical, &schema),
        input
    );
}

#[test]
fn canonicalization_identity_proof_falls_back_for_every_tree_rewrite_trigger() {
    let schema = tiptap_schema();
    let bold = json!({"type": "bold"});
    let italic = json!({"type": "italic"});
    let cases = [
        (
            "schema mark order",
            json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{
                        "type": "text",
                        "text": "marked",
                        "marks": [italic.clone(), bold.clone()]
                    }]
                }]
            }),
            json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{
                        "type": "text",
                        "text": "marked",
                        "marks": [bold.clone(), italic.clone()]
                    }]
                }]
            }),
        ),
        (
            "empty text child",
            json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [
                        {"type": "text", "text": ""},
                        {"type": "text", "text": "tail"}
                    ]
                }]
            }),
            json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{"type": "text", "text": "tail"}]
                }]
            }),
        ),
        (
            "adjacent equal marks",
            json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [
                        {"type": "text", "text": "a", "marks": [bold.clone()]},
                        {"type": "text", "text": "b", "marks": [bold.clone()]}
                    ]
                }]
            }),
            json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{"type": "text", "text": "ab", "marks": [bold.clone()]}]
                }]
            }),
        ),
        (
            "adjacent equivalent marks in different orders",
            json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [
                        {
                            "type": "text",
                            "text": "a",
                            "marks": [bold.clone(), italic.clone()]
                        },
                        {
                            "type": "text",
                            "text": "b",
                            "marks": [italic.clone(), bold.clone()]
                        }
                    ]
                }]
            }),
            json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{
                        "type": "text",
                        "text": "ab",
                        "marks": [bold.clone(), italic.clone()]
                    }]
                }]
            }),
        ),
        (
            "nested descendant requiring ancestor rebuild",
            json!({
                "type": "doc",
                "content": [{
                    "type": "blockquote",
                    "content": [{
                        "type": "paragraph",
                        "content": [
                            {"type": "text", "text": "nested"},
                            {"type": "text", "text": " tail"}
                        ]
                    }]
                }]
            }),
            json!({
                "type": "doc",
                "content": [{
                    "type": "blockquote",
                    "content": [{
                        "type": "paragraph",
                        "content": [{"type": "text", "text": "nested tail"}]
                    }]
                }]
            }),
        ),
    ];

    for (name, input, expected) in cases {
        let document = from_prosemirror_json(&input, &schema, UnknownTypeMode::Error).unwrap();
        let canonical = canonicalize_yrs_document(&document, &schema);
        let actual = crate::serialize::to_prosemirror_json(&canonical, &schema);

        assert!(!canonical.shares_root_storage_with(&document), "{name}");
        assert_eq!(actual, expected, "{name}");
        assert_eq!(
            serde_json::to_vec(&actual).unwrap(),
            serde_json::to_vec(&expected).unwrap(),
            "{name}"
        );
    }
}

#[test]
fn validation_stats_capture_exact_reusable_work_metrics() {
    let schema = tiptap_schema();
    let document = from_prosemirror_json(
        &json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{"type": "text", "text": "a🙂", "marks": [{"type": "bold"}]}]
            }]
        }),
        &schema,
        UnknownTypeMode::Error,
    )
    .unwrap();

    let report = DocumentValidator::validate_report(&document, &schema, &ResourceLimits::default())
        .expect("fixture is valid");
    let stats = report.stats;

    assert_eq!(stats.node_count, 3);
    assert_eq!(stats.max_depth, 3);
    assert_eq!(report.metrics.metadata_bytes, 0);
    assert!(report.metrics.validation_work >= stats.node_count);
}

#[test]
fn canonical_mark_observability_counts_attempt_completion_and_nodes_at_entrypoint() {
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };
    use std::collections::HashMap;

    let schema = tiptap_schema();
    let valid = from_prosemirror_json(
        &json!({
            "type": "doc",
            "content": [{"type": "paragraph", "content": [{"type": "text", "text": "x"}]}]
        }),
        &schema,
        UnknownTypeMode::Error,
    )
    .unwrap();
    reset_full_pass_counts_for_test();
    validate_canonical_marks(&valid, &schema).unwrap();
    let counts = take_full_pass_counts_for_test();
    assert_eq!(counts.canonical_mark_validation_attempts, 1);
    assert_eq!(counts.canonical_mark_validation_completions, 1);
    assert_eq!(counts.canonical_mark_nodes_visited, 3);

    let invalid = Document::new(Node::element(
        "doc".into(),
        HashMap::new(),
        Fragment::from(vec![Node::element(
            "paragraph".into(),
            HashMap::new(),
            Fragment::from(vec![Node::text(
                "x".into(),
                vec![Mark::new("unknown".into(), HashMap::new())],
            )]),
        )]),
    ));
    reset_full_pass_counts_for_test();
    assert!(validate_canonical_marks(&invalid, &schema).is_err());
    let counts = take_full_pass_counts_for_test();
    assert_eq!(counts.canonical_mark_validation_attempts, 1);
    assert_eq!(counts.canonical_mark_validation_completions, 0);
    assert_eq!(counts.canonical_mark_nodes_visited, 3);
}

#[test]
fn validation_stats_preserve_exact_and_one_under_resource_boundaries() {
    let schema = tiptap_schema();
    let document = from_prosemirror_json(
        &json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{"type": "text", "text": "abc"}]
            }]
        }),
        &schema,
        UnknownTypeMode::Error,
    )
    .unwrap();
    let exact = ResourceLimits {
        max_document_nodes: 3,
        max_document_depth: 3,
        ..ResourceLimits::default()
    };
    assert!(DocumentValidator::validate(&document, &schema, &exact).is_ok());
    let mut one_under_nodes = exact.clone();
    one_under_nodes.max_document_nodes = 2;
    let node_error = DocumentValidator::validate(&document, &schema, &one_under_nodes).unwrap_err();
    assert_eq!(node_error.limit, Some(2));
    assert_eq!(node_error.actual, Some(3));
    let mut one_under_depth = exact;
    one_under_depth.max_document_depth = 2;
    let depth_error =
        DocumentValidator::validate(&document, &schema, &one_under_depth).unwrap_err();
    assert_eq!(depth_error.limit, Some(2));
    assert_eq!(depth_error.actual, Some(3));

    let opaque = from_prosemirror_json(
        &json!({
            "type": "doc",
            "content": [{
                "type": "futureBlock",
                "attrs": {"payload": "\\\"🙂\\n"}
            }]
        }),
        &schema,
        UnknownTypeMode::Preserve,
    )
    .unwrap();
    let baseline =
        DocumentValidator::validate_report(&opaque, &schema, &ResourceLimits::default()).unwrap();
    assert!(baseline.metrics.metadata_bytes > 0);
    let exact_input = ResourceLimits {
        max_input_bytes: baseline.metrics.metadata_bytes,
        ..ResourceLimits::default()
    };
    assert_eq!(
        DocumentValidator::validate_report(&opaque, &schema, &exact_input)
            .unwrap()
            .metrics
            .metadata_bytes,
        baseline.metrics.metadata_bytes
    );
    let mut one_under_input = exact_input;
    one_under_input.max_input_bytes -= 1;
    let input_error = DocumentValidator::validate(&opaque, &schema, &one_under_input).unwrap_err();
    assert_eq!(input_error.limit, Some(baseline.metrics.metadata_bytes - 1));
    assert_eq!(input_error.actual, Some(baseline.metrics.metadata_bytes));
}

fn mark(mark_type: &str) -> Mark {
    Mark::new(mark_type.to_string(), HashMap::new())
}

fn mark_with_attrs(mark_type: &str, attrs: &[(&str, serde_json::Value)]) -> Mark {
    Mark::new(
        mark_type.to_string(),
        attrs
            .iter()
            .map(|(name, value)| ((*name).to_string(), value.clone()))
            .collect(),
    )
}

fn schema_with_ten_marks() -> Schema {
    let base = tiptap_schema();
    let nodes = base.all_nodes().cloned().collect();
    let marks = (0..10)
        .map(|index| MarkSpec {
            name: format!("mark{index}"),
            html_tag: None,
            attrs: HashMap::new(),
            excludes: None,
            allow_undeclared_attrs: false,
        })
        .collect();
    Schema::new(nodes, marks)
}

#[test]
fn mark_set_duplicate_detection_uses_bounded_allocation_strategy_at_indices_1_7_8_9() {
    let schema = schema_with_ten_marks();
    for (duplicate_index, expected_allocations) in [(1, 0), (7, 0), (8, 1), (9, 1)] {
        let mut marks = (0..duplicate_index)
            .map(|index| mark(&format!("mark{index}")))
            .collect::<Vec<_>>();
        marks.push(mark("mark0"));

        reset_mark_set_hash_allocations_for_test();
        let error = validate_input_mark_set(&marks, &schema).unwrap_err();

        assert_eq!(error.code, "DOCUMENT_INVALID", "index {duplicate_index}");
        assert_eq!(
            error.message,
            "duplicate same-type marks cannot be represented by standard Yjs attributes",
            "index {duplicate_index}"
        );
        assert_eq!(
            error.details,
            Some(json!({
                "field": "marks",
                "markType": "mark0",
                "reason": "duplicateType",
            })),
            "index {duplicate_index}"
        );
        assert_eq!(
            take_mark_set_hash_allocations_for_test(),
            expected_allocations,
            "index {duplicate_index}"
        );
    }
}

#[test]
fn mark_set_hash_fallback_allocates_once_for_more_than_eight_unique_marks() {
    let schema = schema_with_ten_marks();
    let marks = (0..9)
        .map(|index| mark(&format!("mark{index}")))
        .collect::<Vec<_>>();

    reset_mark_set_hash_allocations_for_test();
    validate_input_mark_set(&marks, &schema).unwrap();

    assert_eq!(take_mark_set_hash_allocations_for_test(), 1);
}

#[test]
fn mark_sets_through_eight_entries_do_not_allocate_duplicate_storage() {
    let schema = schema_with_ten_marks();
    for mark_count in [0, 1, 8] {
        let marks = (0..mark_count)
            .map(|index| mark(&format!("mark{index}")))
            .collect::<Vec<_>>();

        reset_mark_set_hash_allocations_for_test();
        validate_input_mark_set(&marks, &schema).unwrap();

        assert_eq!(
            take_mark_set_hash_allocations_for_test(),
            0,
            "mark count {mark_count}"
        );
    }
}

#[test]
fn mark_set_validation_preserves_error_precedence_and_exact_errors() {
    let schema = tiptap_schema();

    let unknown_first = [mark("unknown"), mark("unknown")];
    let error = validate_input_mark_set(&unknown_first, &schema).unwrap_err();
    assert_eq!(error.code, "UNKNOWN_MARK");
    assert_eq!(error.message, "unknown mark 'unknown'");
    assert_eq!(error.details, None);

    let duplicate_before_bad_attrs = [
        mark("bold"),
        mark_with_attrs("bold", &[("invalid", json!(true))]),
    ];
    let error = validate_input_mark_set(&duplicate_before_bad_attrs, &schema).unwrap_err();
    assert_eq!(error.code, "DOCUMENT_INVALID");
    assert_eq!(
        error.message,
        "duplicate same-type marks cannot be represented by standard Yjs attributes"
    );
    assert_eq!(
        error.details,
        Some(json!({
            "field": "marks",
            "markType": "bold",
            "reason": "duplicateType",
        }))
    );

    let noncanonical_before_bad_attrs = [
        mark("italic"),
        mark_with_attrs("bold", &[("invalid", json!(true))]),
    ];
    let error = validate_mark_set(&noncanonical_before_bad_attrs, &schema, true).unwrap_err();
    assert_eq!(error.code, "DOCUMENT_INVALID");
    assert_eq!(
        error.message,
        "mark order does not match ProseMirror schema rank"
    );
    assert_eq!(
        error.details,
        Some(json!({"field": "marks", "reason": "nonCanonicalOrder"}))
    );

    let error = validate_input_mark_set(&[mark("link")], &schema).unwrap_err();
    assert_eq!(error.code, "REQUIRED_ATTRIBUTE_MISSING");
    assert_eq!(error.message, "'link' requires attribute 'href'");
    assert_eq!(error.details, None);

    let error = validate_input_mark_set(
        &[mark_with_attrs("bold", &[("invalid", json!(true))])],
        &schema,
    )
    .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_INVALID");
    assert_eq!(
        error.message,
        "'bold' contains undeclared attribute 'invalid'"
    );
    assert_eq!(error.details, None);
}

fn document_with_marked_text(marks: Vec<Mark>) -> Document {
    Document::new(Node::element(
        "doc".into(),
        HashMap::new(),
        Fragment::from(vec![Node::element(
            "paragraph".into(),
            HashMap::new(),
            Fragment::from(vec![Node::text("x".to_string(), marks)]),
        )]),
    ))
}

#[test]
fn importable_mark_validation_reports_order_the_canonicalizer_repairs() {
    let schema = tiptap_schema();
    let out_of_order = document_with_marked_text(vec![mark("italic"), mark("bold")]);

    // Strict validation is what the engine holds its own output to.
    let error = validate_canonical_marks_with_evidence(&out_of_order, &schema).unwrap_err();
    assert_eq!(
        error.message,
        "mark order does not match ProseMirror schema rank"
    );

    // An import instead reports it, so admission canonicalizes.
    let evidence = validate_importable_marks_with_evidence(&out_of_order, &schema).unwrap();
    let canonical = canonicalize_yrs_document_with_evidence(&out_of_order, &schema, evidence);
    assert_eq!(
        canonical
            .root()
            .content()
            .and_then(|content| content.iter().next().cloned())
            .and_then(|paragraph| paragraph
                .content()
                .and_then(|content| content.iter().next().cloned()))
            .expect("the canonical document keeps its text node")
            .marks()
            .iter()
            .map(|mark| mark.mark_type().to_string())
            .collect::<Vec<_>>(),
        vec!["bold".to_string(), "italic".to_string()],
        "the evidence must make admission sort the marks, not skip canonicalization"
    );

    // An already-canonical import is left exactly as it arrived.
    let in_order = document_with_marked_text(vec![mark("bold"), mark("italic")]);
    let evidence = validate_importable_marks_with_evidence(&in_order, &schema).unwrap();
    assert_eq!(
        canonicalize_yrs_document_with_evidence(&in_order, &schema, evidence),
        in_order
    );
}

#[test]
fn importable_mark_validation_still_refuses_unrepresentable_mark_sets() {
    let schema = tiptap_schema();

    let duplicate = document_with_marked_text(vec![mark("bold"), mark("bold")]);
    let error = validate_importable_marks_with_evidence(&duplicate, &schema).unwrap_err();
    assert_eq!(
        error.message,
        "duplicate same-type marks cannot be represented by standard Yjs attributes"
    );

    let unknown = document_with_marked_text(vec![mark("unknown")]);
    let error = validate_importable_marks_with_evidence(&unknown, &schema).unwrap_err();
    assert_eq!(error.code, "UNKNOWN_MARK");
}
