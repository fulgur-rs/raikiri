use super::*;

#[test]
#[should_panic]
fn limits_cannot_exceed_the_fixed_scratch_capacity() {
    Storage::new(Limits {
        roots: MAX_PREPARED_ROOTS + 1,
        ..Limits::default()
    });
}
