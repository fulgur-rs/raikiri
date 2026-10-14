use super::*;

#[test]
fn only_running_elements_are_laid_out() {
    let cache = LayoutCache::new(&RunningIndex::default());
    let none = cache.get_or_try_make(NodeId(2), 10.0, || -> Result<_, ()> {
        unreachable!("only running elements are laid out")
    });
    assert!(matches!(none, Ok(None)));
}
