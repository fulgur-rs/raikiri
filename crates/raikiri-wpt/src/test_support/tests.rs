use super::*;

/// The locator must keep pointing inside the sparse checkout's resources
/// directory: the end-to-end tests ignored without a checkout read the real
/// harness from there, and the panic below names this same path when absent.
#[test]
fn real_testharness_js_path_points_at_checkout_resources() {
    let path = real_testharness_js_path();
    assert!(path.ends_with("target/wpt/resources/testharness.js"));
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some("testharness.js")
    );
}

/// The reader returns the checkout file when the sparse checkout is present
/// and panics naming the missing path when it is absent (as in the coverage
/// pass, which never fetches the checkout). Both arms execute the helper so
/// its lines stay covered either way.
#[test]
fn read_real_testharness_js_reads_checkout_or_names_missing_path() {
    use std::panic::AssertUnwindSafe;

    let path = real_testharness_js_path();
    match std::fs::read_to_string(&path) {
        Ok(expected) => assert_eq!(read_real_testharness_js(), expected),
        Err(_) => {
            let payload = std::panic::catch_unwind(AssertUnwindSafe(read_real_testharness_js))
                .expect_err("reading without the checkout must panic");
            let message = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .expect("panic payload is a message");
            assert!(
                message.contains(&path.display().to_string()),
                "panic names the missing file: {message}"
            );
        }
    }
}
