#![no_main]

use editor_core::boundary::ResourceLimits;
use editor_core::tiptap_schema;
use editor_core::yrs_engine::{
    DocumentScope, DocumentSnapshot, InitializationMode, YrsDocumentEngine, YrsEngineConfig,
};
use libfuzzer_sys::fuzz_target;

const MAX_FUZZ_INPUT_BYTES: usize = 64 * 1024;

fn fuzz_engine() -> YrsDocumentEngine {
    YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".to_string(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits {
            max_encoded_state_bytes: MAX_FUZZ_INPUT_BYTES,
            ..ResourceLimits::default()
        },
        scope: Some(DocumentScope {
            document_id: "fuzz-document".to_string(),
            lineage_id: "fuzz-lineage".to_string(),
        }),
    })
    .expect("small fuzz engine must initialize")
}

fn matching_snapshot(engine: &YrsDocumentEngine) -> DocumentSnapshot {
    engine
        .export_snapshot()
        .expect("small fuzz engine must export a matching snapshot")
}

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_FUZZ_INPUT_BYTES {
        return;
    }

    let mut engine = fuzz_engine();
    let mut snapshot = matching_snapshot(&engine);
    snapshot.encoded_state = data.to_vec();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = engine.restore_snapshot(&snapshot);
    }))
    .expect("snapshot restore must not unwind");
});
