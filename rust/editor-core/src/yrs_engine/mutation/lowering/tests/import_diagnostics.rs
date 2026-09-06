#[test]
fn import_lookup_child_count_overflow_preserves_frame_specific_diagnostics() {
    let mut structural = ImportLookupMaterializationCollector::new(
        98_001,
        BranchID::Root(Arc::from("root")),
        1,
        None,
    );
    structural.frames.last_mut().unwrap().structural_child_count = usize::MAX;
    structural.begin_fragment();
    assert_eq!(
        structural.finish().err().unwrap().message.as_ref(),
        "structural parent child count overflow"
    );

    let mut textblock = ImportLookupMaterializationCollector::new(
        98_002,
        BranchID::Root(Arc::from("root")),
        1,
        None,
    );
    assert!(textblock.begin_element(
        BranchID::Root(Arc::from("paragraph")),
        ImportElementAttributeWork::new(),
        false,
        true,
    ));
    textblock.frames.last_mut().unwrap().structural_child_count = usize::MAX;
    textblock.begin_fragment();
    assert_eq!(
        textblock.finish().err().unwrap().message.as_ref(),
        "Yrs textblock child count overflow"
    );
}
