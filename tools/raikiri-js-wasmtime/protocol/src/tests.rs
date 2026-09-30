use super::*;
#[test]
fn transport_cap_and_version() {
    assert!(encode(&vec![0u8; MAX_MESSAGE]).is_err());
    let bytes = encode(&Request {
        version: VERSION + 1,
        operation: HostOperation::Fetch("x".into()),
    })
    .unwrap();
    let r: Request = decode(&bytes).unwrap();
    assert!(r.check_version().is_err());
}

#[test]
fn geometry_roundtrip_preserves_float_bits() {
    let value = 92.19999694824219f64;
    let dto = GeometryDto {
        border: [value; 6],
        padding: [value; 6],
        scroll: [value; 2],
        position: 0,
        is_inline: false,
    };
    let bytes = encode(&dto).unwrap();
    let decoded: GeometryDto = decode(&bytes).unwrap();
    assert_eq!(decoded.border[0].to_bits(), value.to_bits());
}

#[test]
fn nonfinite_harness_status_roundtrips_as_error_status() {
    for status in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let dto = DeliveryDto {
            tests: vec![],
            harness_status: status,
            harness_message: "unexpected status".into(),
        };
        let decoded: DeliveryDto = decode(&encode(&dto).unwrap()).unwrap();
        assert_eq!(decoded.harness_status, 1.0);
        assert_eq!(decoded.harness_message, dto.harness_message);
    }
}

#[test]
fn decode_rejects_oversize_and_corrupt_payload() {
    assert!(decode::<Request>(&vec![0u8; MAX_MESSAGE + 1]).is_err());
    assert!(decode::<Request>(b"not json").is_err());
    let good = encode(&Request {
        version: VERSION,
        operation: HostOperation::Fetch("x".into()),
    })
    .unwrap();
    let ok: Request = decode(&good).unwrap();
    assert!(ok.check_version().is_ok());
}

#[test]
fn hostile_geometry_is_rejected() {
    fn base() -> GeometryDto {
        GeometryDto {
            border: [0.0; 6],
            padding: [0.0; 6],
            scroll: [0.0; 2],
            position: 0,
            is_inline: false,
        }
    }
    assert!(base().into_native().is_ok());
    for position in [5u8, 6, 255] {
        let mut bad = base();
        bad.position = position;
        assert!(bad.into_native().is_err());
    }
    let mut bad = base();
    bad.border[3] = f64::NAN;
    assert!(bad.into_native().is_err());
    let mut bad = base();
    bad.padding[0] = f64::INFINITY;
    assert!(bad.into_native().is_err());
    let mut bad = base();
    bad.scroll[1] = f64::NEG_INFINITY;
    assert!(bad.into_native().is_err());
}

#[test]
fn limits_and_report_abort_variants_roundtrip() {
    use raikiri_js::runtime::{Limits, RunOptions};
    let options = RunOptions {
        limits: Limits {
            max_virtual_time_ms: 123.5,
            max_tasks: 7,
            max_loop_iterations: 11,
            max_recursion: 13,
            max_nodes: 17,
        },
    };
    let dto = LimitsDto::from(options.clone());
    let back = dto.clone().into_options();
    assert_eq!(back.limits.max_tasks, 7);
    assert_eq!(back.limits.max_nodes, 17);
    let bytes = encode(&dto).unwrap();
    let decoded: LimitsDto = decode(&bytes).unwrap();
    assert_eq!(decoded.tasks, 7);

    for abort in [
        AbortDto::VirtualTime,
        AbortDto::Tasks,
        AbortDto::LoopIterations,
        AbortDto::Recursion,
        AbortDto::Nodes,
    ] {
        let report = ReportDto {
            scripts_run: 2,
            uncaught_errors: vec!["e".into()],
            fetch_errors: Vec::new(),
            aborted: Some(abort),
            host_failures: Vec::new(),
            console: vec![("log".into(), "msg".into())],
        };
        let native = report.clone().into_native();
        assert!(native.aborted.is_some());
        let back = ReportDto::from(native);
        assert!(back.aborted.is_some());
        let bytes = encode(&report).unwrap();
        let decoded: ReportDto = decode(&bytes).unwrap();
        assert!(decoded.aborted.is_some());
    }
    // No-abort report roundtrips as native with no abort.
    let clean = ReportDto {
        scripts_run: 1,
        uncaught_errors: Vec::new(),
        fetch_errors: Vec::new(),
        aborted: None,
        host_failures: Vec::new(),
        console: Vec::new(),
    };
    assert!(clean.clone().into_native().aborted.is_none());
}

#[test]
fn guest_envelope_version_mismatch_is_detectable() {
    let request = GuestRequest {
        version: VERSION + 1,
        operation: GuestOperation::TakeResults,
    };
    let bytes = encode(&request).unwrap();
    let decoded: GuestRequest = decode(&bytes).unwrap();
    assert_ne!(decoded.version, VERSION);

    let response = GuestResponse {
        version: VERSION + 1,
        result: Ok(GuestValue::Unit),
    };
    let bytes = encode(&response).unwrap();
    let decoded: GuestResponse = decode(&bytes).unwrap();
    assert_ne!(decoded.version, VERSION);
}
