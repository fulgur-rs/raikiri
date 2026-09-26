use crate::runtime::test_host::StubHost;
use crate::runtime::{DomRuntime, RuntimeError};

fn rt() -> DomRuntime {
    let (host, ..) = StubHost::page();
    DomRuntime::new(host).unwrap()
}

fn ok(rt: &mut DomRuntime, src: &str) {
    assert!(
        rt.evaluate(src).unwrap().to_boolean(),
        "expected true: {src}"
    );
}

#[test]
fn update_steps_never_create_an_attribute_for_an_empty_set() {
    let mut rt = rt();
    rt.evaluate("var e = document.createElement('e'); e.classList.remove('x');")
        .unwrap();
    ok(&mut rt, "!e.hasAttribute('class')");
}

#[test]
fn add_dedupes_existing_duplicates_when_serializing() {
    let mut rt = rt();
    rt.evaluate(
        "var e = document.createElement('e'); \
         e.setAttribute('class', 'a a b'); \
         e.classList.add('c');",
    )
    .unwrap();
    ok(&mut rt, "e.className === 'a b c'");
}

#[test]
fn length_item_indexing_and_value_getter() {
    let mut rt = rt();
    rt.evaluate("var e = document.createElement('e'); e.setAttribute('class', 'a b c');")
        .unwrap();
    ok(
        &mut rt,
        "e.classList.length === 3 && e.classList[1] === 'b' \
         && e.classList.item(9) === null && e.classList.value === 'a b c'",
    );
}

#[test]
fn replace_reorders_and_reports_absent_tokens() {
    let mut rt = rt();
    rt.evaluate("var e = document.createElement('e'); e.setAttribute('class', 'a b c');")
        .unwrap();
    ok(
        &mut rt,
        "e.classList.replace('b', 'z') && e.className === 'a z c' \
         && !e.classList.replace('nope', 'q')",
    );
}

/// Infra "replace within a list": when `newToken` already occurs earlier
/// than `oldToken`, the surviving token keeps its own (earlier) position
/// and `oldToken` is simply dropped, rather than `newToken` moving to
/// `oldToken`'s later position.
#[test]
fn replace_with_a_token_already_present_drops_the_old_one_in_place() {
    let mut rt = rt();
    rt.evaluate("var e = document.createElement('e'); e.setAttribute('class', 'a b c');")
        .unwrap();
    ok(
        &mut rt,
        "e.classList.replace('c', 'a') === true && e.className === 'a b'",
    );
}

/// DOM §7.1 `replace`: emptiness of *both* tokens is checked before
/// whitespace of either, so an empty `newToken` reports `SyntaxError` even
/// when `oldToken` is the one that actually contains whitespace.
#[test]
fn replace_checks_emptiness_of_both_tokens_before_whitespace_of_either() {
    let mut rt = rt();
    rt.evaluate("var e = document.createElement('e');").unwrap();
    ok(
        &mut rt,
        "try { e.classList.replace('a b', ''); false } \
         catch (t) { t.name === 'SyntaxError' }",
    );
}

/// DOM §7.1 `replace`: with both tokens non-empty, ASCII whitespace in
/// either one throws `InvalidCharacterError`.
#[test]
fn replace_rejects_a_token_containing_whitespace() {
    let mut rt = rt();
    rt.evaluate("var e = document.createElement('e'); e.setAttribute('class', 'a b');")
        .unwrap();
    ok(
        &mut rt,
        "(function () { \
           try { e.classList.replace('a b', 'c'); return false; } \
           catch (t) { return t.name === 'InvalidCharacterError' && e.className === 'a b'; } \
         })()",
    );
}

#[test]
fn toggle_with_and_without_force() {
    let mut rt = rt();
    rt.evaluate("var e = document.createElement('e'); e.setAttribute('class', 'a z c');")
        .unwrap();
    ok(
        &mut rt,
        "e.classList.toggle('a', true) === true \
         && e.classList.toggle('a', false) === false \
         && !e.classList.contains('a')",
    );
}

/// `force` has no WebIDL default value, but WebIDL overload resolution
/// still maps an explicitly-`undefined` optional argument to "not
/// present" -- the same as a genuinely missing one -- rather than running
/// it through `ToBoolean`. So `toggle('p', undefined)` behaves exactly
/// like `toggle('p')`, in both directions (token absent -> add, token
/// present -> remove), not like `toggle('p', false)`.
#[test]
fn toggle_treats_an_explicit_undefined_force_the_same_as_a_missing_one() {
    let mut rt = rt();
    rt.evaluate("var e = document.createElement('e');").unwrap();
    // Token absent, force missing entirely: adds (plain toggle-on).
    ok(
        &mut rt,
        "e.classList.toggle('p') === true && e.classList.contains('p')",
    );
    rt.evaluate("e.classList.remove('p');").unwrap();
    // Token absent, force explicitly `undefined`: still adds, the same as
    // a missing `force` -- not `false` (`ToBoolean(undefined)`), which
    // would instead leave `p` absent.
    ok(
        &mut rt,
        "e.classList.toggle('p', undefined) === true && e.classList.contains('p')",
    );
    // Token present, force explicitly `undefined`: still removes, the
    // same as a missing `force`.
    ok(
        &mut rt,
        "e.classList.toggle('p', undefined) === false && !e.classList.contains('p')",
    );
}

#[test]
fn supports_always_throws_a_type_error() {
    let mut rt = rt();
    rt.evaluate("var e = document.createElement('e');").unwrap();
    ok(
        &mut rt,
        "try { e.classList.supports('x'); false } catch (t) { t instanceof TypeError }",
    );
}

#[test]
fn value_setter_writes_the_attribute_directly_and_supports_iteration() {
    let mut rt = rt();
    rt.evaluate("var e = document.createElement('e'); e.classList.value = 'p q';")
        .unwrap();
    ok(
        &mut rt,
        "e.getAttribute('class') === 'p q' && [...e.classList].join() === 'p,q' \
         && e.classList === e.classList",
    );
}

/// The setter always writes, even to an empty string with no prior
/// attribute -- unlike `add`/`remove`'s update steps, it never skips
/// creating the attribute.
#[test]
fn value_setter_creates_the_attribute_even_for_an_empty_string() {
    let mut rt = rt();
    rt.evaluate("var e = document.createElement('e'); e.classList.value = '';")
        .unwrap();
    ok(
        &mut rt,
        "e.hasAttribute('class') && e.getAttribute('class') === ''",
    );
}

#[test]
fn to_string_matches_the_value_getter() {
    let mut rt = rt();
    rt.evaluate("var e = document.createElement('e'); e.setAttribute('class', 'a b');")
        .unwrap();
    ok(&mut rt, "e.classList.toString() === 'a b'");
}

#[test]
fn add_and_remove_validate_before_mutating_anything() {
    let mut rt = rt();
    rt.evaluate("var e = document.createElement('e'); e.setAttribute('class', 'a');")
        .unwrap();
    ok(
        &mut rt,
        "try { e.classList.add('b', 'no no'); false } \
         catch (t) { t.name === 'InvalidCharacterError' } ",
    );
    // Nothing was mutated: the valid 'b' before the invalid token was never applied.
    ok(&mut rt, "e.className === 'a'");
    ok(
        &mut rt,
        "try { e.classList.add(''); false } catch (t) { t.name === 'SyntaxError' }",
    );
}

#[test]
fn class_list_is_a_real_illegal_constructor_interface() {
    let mut rt = rt();
    ok(
        &mut rt,
        "try { new DOMTokenList(); false } catch (e) { e instanceof TypeError }",
    );
    ok(
        &mut rt,
        "document.createElement('e').classList instanceof DOMTokenList",
    );
}

/// A brand mismatch on a `DOMTokenList` prototype method is a `TypeError`,
/// not a panic.
#[test]
fn brand_mismatch_on_prototype_methods_is_a_type_error() {
    let mut rt = rt();
    let err = rt.evaluate("DOMTokenList.prototype.item.call({}, 0)");
    assert!(
        matches!(err, Err(RuntimeError::JavaScript(ref m)) if m.contains("TypeError")),
        "{err:?}"
    );
    let err = rt
        .evaluate("Object.getOwnPropertyDescriptor(DOMTokenList.prototype, 'length').get.call({})");
    assert!(
        matches!(err, Err(RuntimeError::JavaScript(ref m)) if m.contains("TypeError")),
        "{err:?}"
    );
}
