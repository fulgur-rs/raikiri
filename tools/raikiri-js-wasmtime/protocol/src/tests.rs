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
            max_enqueued_jobs: 19,
            max_selector_work: Some(23),
            max_loop_iterations: 11,
            max_recursion: 13,
            max_nodes: 17,
        },
    };
    let dto = LimitsDto::from(options.clone());
    let back = dto.clone().into_options();
    assert_eq!(back.limits.max_tasks, 7);
    assert_eq!(back.limits.max_enqueued_jobs, 19);
    assert_eq!(back.limits.max_selector_work, Some(23));
    assert_eq!(back.limits.max_nodes, 17);
    let bytes = encode(&dto).unwrap();
    let decoded: LimitsDto = decode(&bytes).unwrap();
    assert_eq!(decoded.tasks, 7);

    for abort in [
        AbortDto::VirtualTime,
        AbortDto::Tasks,
        AbortDto::EnqueuedJobs,
        AbortDto::SelectorWork,
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

#[test]
fn image_dto_roundtrips_and_rejects_mismatched_bytes() {
    let pixels = ImagePixels {
        size: (1.0, 0.5),
        image: std::sync::Arc::new(DecodedImage::new(1, 1, vec![1, 2, 3, 255])),
    };
    let bytes = encode(&ImageDto::from(&pixels)).unwrap();
    assert!(
        String::from_utf8_lossy(&bytes).contains(r#""rgba":"AQID/w==""#),
        "the bytes travel as base64"
    );
    let decoded = decode::<ImageDto>(&bytes).unwrap().into_native().unwrap();
    assert_eq!(decoded.size, (1.0, 0.5));
    assert_eq!(*decoded.image, *pixels.image);
    let short = ImageDto {
        size: (2.0, 1.0),
        width: 2,
        height: 1,
        rgba: vec![0; 4],
    };
    assert!(short.into_native().is_err());
    assert!(decode::<ImageDto>(br#"{"size":[1,1],"width":1,"height":1,"rgba":"AQ"}"#).is_err());
}

#[test]
fn image_dto_fits_a_large_image_in_one_message() {
    let image = DecodedImage::new(1024, 1024, vec![255; 1024 * 1024 * 4]);
    let pixels = ImagePixels {
        size: (1024.0, 1024.0),
        image: std::sync::Arc::new(image),
    };
    let response = Response {
        version: VERSION,
        result: Ok(HostValue::Image(Some(ImageDto::from(&pixels)))),
    };
    let bytes = encode(&response).unwrap();
    let Ok(HostValue::Image(Some(dto))) = decode::<Response>(&bytes).unwrap().result else {
        panic!("an image response");
    };
    assert_eq!(dto.rgba.len(), 1024 * 1024 * 4);
}

#[test]
fn base64_roundtrips_every_tail_length_and_rejects_malformed_text() {
    for len in 0..=7 {
        let bytes: Vec<u8> = (0..len).map(|i| (i * 37 + 250) as u8).collect();
        assert_eq!(base64::decode(&base64::encode(&bytes)).unwrap(), bytes);
    }
    assert_eq!(base64::encode(b"foobar"), "Zm9vYmFy");
    assert_eq!(base64::encode(b"fo"), "Zm8=");
    for bad in ["Zm8", "Zm=v", "Z===", "Zm8=Zm8=", "Zm9*"] {
        assert!(base64::decode(bad).is_none(), "{bad}");
    }
}

#[test]
fn selector_work_none_and_exact_limits_keep_version_eight_wire_roundtrip() {
    assert_eq!(VERSION, 8);
    for limit in [None, Some(0), Some(1), Some(u64::MAX)] {
        let mut options = raikiri_js::runtime::RunOptions::default();
        options.limits.max_selector_work = limit;
        let encoded = encode(&LimitsDto::from(options.clone())).unwrap();
        let decoded: LimitsDto = decode(&encoded).unwrap();
        assert_eq!(decoded.into_options(), options);
    }
}
