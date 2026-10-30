use super::*;

fn evaluate(runtime: &mut DomRuntime, script: &str) {
    assert!(runtime.evaluate(script).unwrap().to_boolean(), "{script}");
}

#[test]
fn seeking_and_pausing_visit_only_the_selected_targets_effects() {
    for unrelated in [0, 8, 32] {
        let (mut runtime, target) = runtime_with_target();
        expose_target(&mut runtime, target);
        runtime
            .evaluate(&format!(
                "var effects = []; for (var i = 0; i < {unrelated}; i++) {{ \
                var e = document.body.appendChild(document.createElement('div')); \
                var a = e.animate({{fontSize:['0px','40px']}},40); \
                a.pause(); a.currentTime=20; effects.push(a); \
             }} var chosen = target.animate({{fontSize:['0px','40px']}},40); \
             chosen.pause(); chosen.currentTime=20;"
            ))
            .unwrap();
        COMPOSITION_WORK.set([0; 3]);
        evaluate(
            &mut runtime,
            "for (var i=0;i<100;i++) chosen.currentTime=(i%2===0?10:20); chosen.currentTime===20",
        );
        assert_eq!(COMPOSITION_WORK.get(), [100, 100, 100]);
        assert_eq!(
            animation_style(&mut runtime, target).as_deref(),
            Some("font-size: 20px;")
        );
        COMPOSITION_WORK.set([0; 3]);
        evaluate(&mut runtime, "chosen.pause(); chosen.currentTime===20");
        assert_eq!(COMPOSITION_WORK.get(), [1, 1, 1]);
    }
}

#[test]
fn interleaved_targets_keep_creation_order_and_restore_earlier_properties() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    runtime.evaluate(
        "var other=document.body.appendChild(document.createElement('div')); \
         var first=target.animate({fontSize:['0px','40px']},40); first.pause(); first.currentTime=20; \
         var foreign=other.animate({fontSize:['40px','80px']},40); foreign.pause(); foreign.currentTime=20; \
         var spacing=target.animate({wordSpacing:['0px','8px']},40); spacing.pause(); spacing.currentTime=20; \
         var last=target.animate({fontSize:['20px','60px']},40); last.pause(); last.currentTime=20;"
    ).unwrap();
    assert_eq!(
        animation_style(&mut runtime, target).as_deref(),
        Some("font-size: 20px; word-spacing: 4px; font-size: 40px;")
    );
    runtime.evaluate("last.currentTime=40;").unwrap();
    assert_eq!(
        animation_style(&mut runtime, target).as_deref(),
        Some("font-size: 20px; word-spacing: 4px;")
    );
    runtime.evaluate("first.currentTime=-1;").unwrap();
    assert_eq!(
        animation_style(&mut runtime, target).as_deref(),
        Some("word-spacing: 4px;")
    );
    runtime.evaluate("spacing.currentTime=40;").unwrap();
    assert_eq!(animation_style(&mut runtime, target), None);
    evaluate(&mut runtime, "foreign.currentTime===20");
    let authored = with_state(runtime.context_mut(), |state| {
        state
            .host
            .document()
            .node(StyleNodeId::new(target as u64))
            .unwrap()
            .as_element()
            .unwrap()
            .inline_style_source()
            .map(str::to_owned)
    })
    .unwrap();
    assert_eq!(authored.as_deref(), Some("font-size: 12px"));
}

#[test]
fn creating_an_unsampled_effect_recomposes_only_its_target() {
    let (mut runtime, target) = runtime_with_target();
    expose_target(&mut runtime, target);
    runtime.evaluate(
        "var sampled=target.animate({fontSize:['0px','40px']},40); sampled.pause(); sampled.currentTime=20; \
         var other=document.body.appendChild(document.createElement('div')); \
         var foreign=other.animate({fontSize:['40px','80px']},40); foreign.pause(); foreign.currentTime=20;"
    ).unwrap();
    COMPOSITION_WORK.set([0; 3]);
    runtime
        .evaluate("var unsampled=target.animate([],40);")
        .unwrap();
    assert_eq!(COMPOSITION_WORK.get(), [2, 1, 1]);
    assert_eq!(
        animation_style(&mut runtime, target).as_deref(),
        Some("font-size: 20px;")
    );
}
