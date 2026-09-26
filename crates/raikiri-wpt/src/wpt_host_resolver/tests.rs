use super::*;

#[test]
fn parses_comments_aliases_ipv6_and_identical_duplicates() {
    let directory = tempfile::tempdir().unwrap();
    let hosts = directory.path().join("hosts");
    fs::write(&hosts, "# generated hosts\n\n127.0.0.1 Web-Platform.Test. www.web-platform.test # aliases\n127.0.0.1 web-platform.test\n::1 ipv6.web-platform.test\n").unwrap();
    let resolver = WptHostResolver::from_hosts_file(&hosts).unwrap();
    assert!(format!("{resolver:?}").contains("WptHostResolver"));
    let overrides = resolver.into_overrides();
    assert_eq!(
        overrides.get("web-platform.test"),
        Some("127.0.0.1".parse().unwrap())
    );
    assert_eq!(
        overrides.get("www.web-platform.test"),
        Some("127.0.0.1".parse().unwrap())
    );
    assert_eq!(
        overrides.get("ipv6.web-platform.test"),
        Some("::1".parse().unwrap())
    );
}

#[test]
fn malformed_hosts_report_path_line_and_reason() {
    let directory = tempfile::tempdir().unwrap();
    let hosts = directory.path().join("hosts");
    for (source, line, reason) in [
        (
            "# comment\nnot-an-ip web-platform.test\n",
            2,
            "invalid IP address",
        ),
        (
            "127.0.0.1 # missing hostname\n",
            1,
            "expected at least one hostname",
        ),
        (
            "127.0.0.1 WEB-PLATFORM.TEST.\n::1 web-platform.test\n",
            2,
            "maps to both",
        ),
    ] {
        fs::write(&hosts, source).unwrap();
        let error = WptHostResolver::from_hosts_file(&hosts).unwrap_err();
        assert_eq!(error.line, Some(line));
        assert!(
            error
                .to_string()
                .contains(&format!("{}:{line}:", hosts.display()))
        );
        assert!(error.to_string().contains(reason));
    }
}

#[test]
fn missing_hosts_file_reports_path_without_line() {
    let directory = tempfile::tempdir().unwrap();
    let hosts = directory.path().join("missing");
    let error = WptHostResolver::from_hosts_file(&hosts).unwrap_err();
    assert_eq!(error.line, None);
    assert!(
        error
            .to_string()
            .starts_with(&format!("{}:", hosts.display()))
    );
}
