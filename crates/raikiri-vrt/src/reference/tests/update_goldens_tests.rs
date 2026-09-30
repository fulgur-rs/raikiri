//! Golden-update mode edge: existing `expected/` is replaced.
//!
//! The integration harness already pins update mode from an empty starting
//! state. This test pins the documented delete-first behavior when the page
//! count shrinks (stale `page-0001.png` must disappear).

use super::*;

#[test]
fn update_goldens_replaces_existing_expected_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let fixture_dir = tmp.path();
    std::fs::write(fixture_dir.join("input.html"), b"<p>hi</p>").unwrap();
    let expected = fixture_dir.join("expected");
    std::fs::create_dir_all(&expected).unwrap();
    // Stale two-page goldens; pipeline will now produce one page.
    std::fs::write(expected.join("page-0000.png"), b"stale-a").unwrap();
    std::fs::write(expected.join("page-0001.png"), b"stale-b").unwrap();

    let fresh = crate::encode_png(&[0, 128, 0, 255, 0, 128, 0, 255], 2, 1);
    let fresh_clone = fresh.clone();
    // Call the private helper directly: it deletes `expected/` first, so no
    // env-var juggling (and no global lock) is needed.
    update_goldens(fixture_dir, &[fresh_clone]);

    assert!(
        !expected.join("page-0001.png").exists(),
        "stale page must be removed"
    );
    let written = std::fs::read(expected.join("page-0000.png")).unwrap();
    assert_eq!(written, fresh);
}
