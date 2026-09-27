#[test]
fn invalid_declarations_have_actionable_diagnostics() {
    trybuild::TestCases::new().compile_fail("tests/ui/*.rs");
}
