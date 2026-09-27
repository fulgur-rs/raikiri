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
