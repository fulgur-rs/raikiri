use super::*;

#[test]
fn parse_args_uses_defaults_and_accepts_overrides() {
    let defaults = parse_args(&[]).unwrap();
    assert_eq!(defaults, Args::default());
    let args = parse_args(&[
        "--wpt-root".to_owned(),
        "wpt".to_owned(),
        "--baseline=base.txt".to_owned(),
        "--output".to_owned(),
        "dashboard.html".to_owned(),
        "--basic-reftest-count=12".to_owned(),
        "--tracked-areas=tracked.txt".to_owned(),
    ])
    .unwrap();
    assert_eq!(args.wpt_root, PathBuf::from("wpt"));
    assert_eq!(args.baseline, PathBuf::from("base.txt"));
    assert_eq!(args.output, PathBuf::from("dashboard.html"));
    assert_eq!(args.basic_reftest_count, Some(12));
    assert_eq!(args.tracked_areas, PathBuf::from("tracked.txt"));
}

#[test]
fn normalize_baseline_id_resolves_a_unique_bare_filename() {
    let files = vec![
        "css/css-grid/parsing/grid-row-valid.html".to_owned(),
        "css/css-grid/grid-row-invalid.html".to_owned(),
    ];
    assert_eq!(
        normalize_baseline_id("grid-row-valid.html", &files),
        "css/css-grid/parsing/grid-row-valid.html"
    );
    assert_eq!(
        normalize_baseline_id("unknown.html", &files),
        "unknown.html"
    );
    let ambiguous = vec![
        "css/one/duplicate.html".to_owned(),
        "css/two/duplicate.html".to_owned(),
    ];
    assert_eq!(
        normalize_baseline_id("duplicate.html", &ambiguous),
        "duplicate.html"
    );
}

#[test]
fn parse_args_rejects_unknown_and_missing_values() {
    assert!(parse_args(&["--unknown".to_owned()]).is_err());
    assert!(parse_args(&["--output".to_owned()]).is_err());
    assert!(
        parse_args(&[
            "--basic-reftest-count".to_owned(),
            "not-a-number".to_owned(),
        ])
        .is_err()
    );
}

#[test]
fn html_escape_and_timestamp_are_safe() {
    assert_eq!(html_escape("<&>\"'"), "&lt;&amp;&gt;&quot;&#39;");
    let timestamp = generated_at_jst();
    assert!(timestamp.ends_with(" JST"));
    assert_eq!(timestamp.len(), 20);
}

fn reftest_fixture() -> (Vec<String>, Reftests) {
    let files: Vec<String> = [
        "css/README.md",
        "css/LICENSE",
        "other/x.html",
        "css/css-page/a.html",
        "css/css-page/b.html",
        "css/css-page/sub/c.html",
        "css/css-page/sub/c-ref.html",
        "css/css-page/sub/d.xht",
        "css/css-page/reference/e.html",
        "css/css-page/plain.html",
        "css/css-page/waived.html",
        "css/css-scroll-snap/f.html",
        "css/css-new/g.html",
        "css/support/h.html",
    ]
    .map(str::to_owned)
    .to_vec();
    let link = r#"<link rel="match" href="ref.html">"#.to_owned();
    let reftests = find_reftests(&files, |file| match file {
        "css/css-page/plain.html" => Some("<p>no reference</p>".to_owned()),
        _ => Some(link.clone()),
    });
    (files, reftests)
}

#[test]
fn find_reftests_skips_references_support_files_and_non_reftests() {
    let (_, reftests) = reftest_fixture();
    assert_eq!(
        reftests.runnable,
        [
            "other/x.html",
            "css/css-page/a.html",
            "css/css-page/b.html",
            "css/css-page/sub/c.html",
            "css/css-page/waived.html",
            "css/css-scroll-snap/f.html",
            "css/css-new/g.html",
        ]
    );
    assert_eq!(reftests.xml, ["css/css-page/sub/d.xht"]);
    let unreadable = find_reftests(&["css/a/b.html".to_owned()], |_| None);
    assert_eq!(unreadable.unreadable, 1);
    assert!(unreadable.runnable.is_empty());
}

#[test]
fn wpt_test_path_follows_manifest_naming_rules() {
    assert!(is_wpt_test_path("css/a/test.html"));
    assert!(is_wpt_test_path("test.html"));
    assert!(is_wpt_test_path("css/a/reference-box.html"));
    assert!(!is_wpt_test_path("css/a/test-ref.html"));
    assert!(!is_wpt_test_path("css/a/test-notref.xht"));
    assert!(!is_wpt_test_path("css/a/ref-test.html"));
    assert!(!is_wpt_test_path("css/a/resources/x.html"));
    assert!(!is_wpt_test_path("css/a/tools/x.html"));
}

#[test]
fn area_list_accepts_area_prefixes_only() {
    assert_eq!(
        parse_area_list("# comment\n\ncss/css-page/\n css/CSS2/ \n").unwrap(),
        ["css/css-page/", "css/CSS2/"]
    );
    assert!(parse_area_list("css/css-page").is_err());
    assert!(parse_area_list("css/css-page/sub/").is_err());
    assert!(parse_area_list("html/x/").is_err());
    assert!(parse_area_list("css//").is_err());
    assert!(parse_area_list("css/a/\ncss/a/\n").is_err());
}

#[test]
fn sweep_collapses_variants_to_the_worst_outcome() {
    let root = std::env::temp_dir().join(format!("raikiri-dashboard-sweep-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let path = root.join("sweep.tsv");
    fs::write(
        &path,
        "a.html?1\tPASS\t\na.html?2\tFAIL\tmismatch\nb.html\tXPASS\t\nc.html\tXFAIL\tknown\nd.html?x\tERROR\tboom\nd.html?y\tFAIL\t\ne.html\tSKIP\tskipped\nf.html#frag\tERROR\tunsupported\ng.html\tPASS\tpasses only without local resources\n",
    )
    .unwrap();
    let sweep = read_sweep(&path).unwrap();
    assert_eq!(sweep.get("a.html"), Some(&Outcome::Fail));
    assert_eq!(sweep.get("b.html"), Some(&Outcome::Pass));
    assert_eq!(sweep.get("c.html"), Some(&Outcome::Fail));
    assert_eq!(sweep.get("d.html"), Some(&Outcome::Error));
    assert_eq!(sweep.get("e.html"), None);
    assert_eq!(sweep.get("f.html"), Some(&Outcome::Error));
    assert_eq!(sweep.get("g.html"), Some(&Outcome::Fail));
    fs::write(&path, "a.html\tNOPE\t\n").unwrap();
    assert!(read_sweep(&path).is_err());
    assert!(read_sweep(&root.join("missing.tsv")).is_err());
    fs::remove_dir_all(root).unwrap();
}

fn summary_fixture(sweep: Option<&BTreeMap<String, Outcome>>) -> Summary {
    let (files, reftests) = reftest_fixture();
    let baseline = ["css/css-page/a.html".to_owned(), "css/README.md".to_owned()]
        .into_iter()
        .collect();
    let known_issues = vec![
        (
            "css/css-scroll-snap/".to_owned(),
            "Out of print scope (scroll snapping)".to_owned(),
        ),
        ("css/css-page/waived.html".to_owned(), "waived".to_owned()),
    ];
    summarize(
        &files,
        &reftests,
        &baseline,
        &["css/css-page/".to_owned()],
        &[],
        &known_issues,
        sweep,
    )
    .unwrap()
}

#[test]
fn summarize_splits_areas_and_counts_sweep_outcomes() {
    let sweep: BTreeMap<String, Outcome> = [
        ("css/css-page/a.html".to_owned(), Outcome::Pass),
        ("css/css-page/sub/c.html".to_owned(), Outcome::Error),
        ("css/css-page/waived.html".to_owned(), Outcome::Pass),
        ("css/css-scroll-snap/f.html".to_owned(), Outcome::Pass),
    ]
    .into_iter()
    .collect();
    let summary = summary_fixture(Some(&sweep));
    assert_eq!(summary.in_scope.len(), 1);
    let page = &summary.in_scope[0];
    assert_eq!(page.path, "css/css-page");
    assert_eq!(
        page.tally,
        Tally {
            reftests: 3,
            pass: 1,
            fail: 0,
            error: 1,
            baseline: 1,
        }
    );
    assert_eq!(page.tally.not_run(), 1);
    assert_eq!(page.children.len(), 1);
    assert_eq!(page.children[0].0, "css/css-page/sub");
    assert_eq!(page.children[0].1.error, 1);
    assert_eq!(summary.waived, 1);
    assert_eq!(summary.xml_reftests, 1);
    assert_eq!(
        summary
            .unclassified
            .iter()
            .map(|row| row.path.as_str())
            .collect::<Vec<_>>(),
        ["css/css-new"]
    );
    assert_eq!(
        summary.out_of_scope,
        [OutOfScopeRow {
            path: "css/css-scroll-snap".to_owned(),
            reftests: 1,
            reason: "Out of print scope (scroll snapping)".to_owned(),
        }]
    );
    assert_eq!(summary.in_scope_total().reftests, 3);
}

#[test]
fn summarize_rejects_an_area_in_both_scope_lists() {
    let (files, reftests) = reftest_fixture();
    let error = summarize(
        &files,
        &reftests,
        &HashSet::new(),
        &["css/css-page/".to_owned()],
        &[],
        &[("css/css-page/".to_owned(), "reason".to_owned())],
        None,
    )
    .unwrap_err();
    assert!(error.contains("both print-scope.txt and known-issues.txt"));
}

#[test]
fn summarize_rejects_a_tracked_area_in_another_scope_list() {
    let (files, reftests) = reftest_fixture();
    let tracked = ["css/css-new/".to_owned()];
    let error = summarize(
        &files,
        &reftests,
        &HashSet::new(),
        &tracked,
        &tracked,
        &[],
        None,
    )
    .unwrap_err();
    assert!(error.contains("both print-scope.txt and tracked-areas.txt"));
    let error = summarize(
        &files,
        &reftests,
        &HashSet::new(),
        &[],
        &tracked,
        &[("css/css-new/".to_owned(), "reason".to_owned())],
        None,
    )
    .unwrap_err();
    assert!(error.contains("both tracked-areas.txt and known-issues.txt"));
}

#[test]
fn tracked_areas_get_their_own_table_outside_the_print_scope() {
    let (files, reftests) = reftest_fixture();
    let sweep: BTreeMap<String, Outcome> = [("css/css-new/g.html".to_owned(), Outcome::Pass)]
        .into_iter()
        .collect();
    let summary = summarize(
        &files,
        &reftests,
        &HashSet::new(),
        &["css/css-page/".to_owned()],
        &["css/css-new/".to_owned()],
        &[("css/css-scroll-snap/".to_owned(), "reason".to_owned())],
        Some(&sweep),
    )
    .unwrap();
    assert!(summary.unclassified.is_empty());
    assert_eq!(summary.tracked.len(), 1);
    assert_eq!(summary.tracked[0].path, "css/css-new");
    assert_eq!(summary.tracked[0].tally.pass, 1);
    assert_eq!(summary.in_scope_total().reftests, 4);
    assert_eq!(summary.in_scope_total().pass, 0);
    let html = render_dashboard(&summary, &page_info(Some("raikiri abc")));
    assert!(html.contains("<h2>Tracked outside the print scope</h2>"));
    assert!(html.contains("<tr class=\"total\"><td>Total (tracked)</td><td class=\"num\">1</td><td class=\"num\">1</td>"));
    assert!(html.contains("In-scope pass rate: 0.00%</strong> (0 of 4 reftests)"));
    assert!(!html.contains("<h2>Unclassified</h2>"));
}

#[test]
fn sweep_label_reads_metadata_and_rejects_another_wpt_pin() {
    let root = std::env::temp_dir().join(format!("raikiri-dashboard-meta-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let sweep = root.join("sweep.tsv");
    assert!(
        sweep_label(&sweep, "pin")
            .unwrap_err()
            .contains("could not read sweep metadata")
    );
    fs::write(root.join("sweep.tsv.meta"), "raikiri_sha=abc\n").unwrap();
    assert!(
        sweep_label(&sweep, "pin")
            .unwrap_err()
            .contains("missing wpt_sha")
    );
    fs::write(
        root.join("sweep.tsv.meta"),
        "wpt_sha=97ea26e26a2aac3eec7e\nraikiri_sha=9ed001b0123456789\n",
    )
    .unwrap();
    assert_eq!(
        sweep_label(&sweep, "97ea26e26a2aac3eec7e").unwrap(),
        "raikiri 9ed001b012, WPT 97ea26e26a"
    );
    assert_eq!(
        sweep_label(&sweep, "unknown").unwrap(),
        "raikiri 9ed001b012, WPT 97ea26e26a"
    );
    assert!(
        sweep_label(&sweep, "other")
            .unwrap_err()
            .contains("was measured at WPT")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rate_cell_uses_expected_class_and_percentage() {
    assert!(rate_cell(0, 0).contains("class=\"empty\""));
    assert!(rate_cell(0, 10).contains("class=\"fail\""));
    assert!(rate_cell(5, 10).contains("50.00%"));
    assert!(rate_cell(10, 10).contains("class=\"pass\""));
}

fn page_info(sweep_label: Option<&str>) -> PageInfo<'_> {
    PageInfo {
        pin: "pin",
        generated_at: "now",
        css_files: 12,
        baseline_entries: 2,
        sweep_label,
        basic_reftest_count: Some(7),
    }
}

#[test]
fn render_dashboard_shows_sweep_counts_and_scope_sections() {
    let sweep: BTreeMap<String, Outcome> = [("css/css-page/a.html".to_owned(), Outcome::Pass)]
        .into_iter()
        .collect();
    let html = render_dashboard(
        &summary_fixture(Some(&sweep)),
        &page_info(Some("raikiri abc")),
    );
    assert!(html.contains("WPT pin: <code>pin</code>"));
    assert!(html.contains("Basic reftest smoke: <code>7</code> passed"));
    assert!(html.contains("In-scope pass rate: 33.33%</strong> (1 of 3 reftests)"));
    assert!(html.contains("sweep: <code>raikiri abc</code>"));
    assert!(html.contains("<h2>Unclassified</h2>"));
    assert!(html.contains("<td>css/css-scroll-snap</td><td class=\"num\">1</td><td>Out of print scope (scroll snapping)</td>"));
    assert!(html.contains("↳ css/css-page/sub"));
    assert!(html.contains("1 in-scope reftests matching a narrower known-issues entry"));
}

#[test]
fn render_dashboard_without_sweep_falls_back_to_baseline_rate() {
    let html = render_dashboard(&summary_fixture(None), &page_info(None));
    assert!(html.contains("In-scope baseline rate: 33.33%"));
    assert!(html.contains("<td class=\"num muted\">–</td>"));
    assert!(!html.contains("sweep: <code>"));
    assert!(!html.contains("could not be read"));
    let unreadable = Summary {
        unreadable: 2,
        ..Summary::default()
    };
    let html = render_dashboard(&unreadable, &page_info(None));
    assert!(html.contains("2 candidate files could not be read"));
    assert!(!html.contains("<h2>Unclassified</h2>"));
    assert!(!html.contains("<h2>Tracked outside the print scope</h2>"));
}

#[test]
fn file_inventory_falls_back_without_git() {
    let root =
        std::env::temp_dir().join(format!("raikiri-dashboard-no-git-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("css/foo/nested")).unwrap();
    fs::write(root.join("css/foo/a.html"), "a").unwrap();
    fs::write(root.join("css/foo/nested/b.html"), "b").unwrap();
    let files = list_wpt_files(&root).unwrap();
    assert_eq!(files, ["css/foo/a.html", "css/foo/nested/b.html"]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn file_inventory_uses_git_tree_and_reads_head() {
    let root = std::env::temp_dir().join(format!("raikiri-dashboard-git-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("css/foo")).unwrap();
    fs::write(root.join("css/foo/a.html"), "a").unwrap();
    let status = Command::new("git")
        .arg("init")
        .arg("-q")
        .current_dir(&root)
        .status()
        .unwrap();
    assert!(status.success());
    let status = Command::new("git")
        .args([
            "-c",
            "user.name=dashboard-test",
            "-c",
            "user.email=dashboard@test.invalid",
        ])
        .args(["add", "."])
        .current_dir(&root)
        .status()
        .unwrap();
    assert!(status.success());
    let status = Command::new("git")
        .args([
            "-c",
            "user.name=dashboard-test",
            "-c",
            "user.email=dashboard@test.invalid",
        ])
        .args(["commit", "-qm", "fixture"])
        .current_dir(&root)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(list_wpt_files(&root).unwrap(), ["css/foo/a.html"]);
    assert_eq!(git_head(&root).unwrap().len(), 40);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn file_inventory_falls_back_after_empty_git_inventory() {
    let root = std::env::temp_dir().join(format!(
        "raikiri-dashboard-empty-git-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("README"), "fixture").unwrap();
    let status = Command::new("git")
        .arg("init")
        .arg("-q")
        .current_dir(&root)
        .status()
        .unwrap();
    assert!(status.success());
    let status = Command::new("git")
        .args([
            "-c",
            "user.name=dashboard-test",
            "-c",
            "user.email=dashboard@test.invalid",
        ])
        .args(["add", "README"])
        .current_dir(&root)
        .status()
        .unwrap();
    assert!(status.success());
    let status = Command::new("git")
        .args([
            "-c",
            "user.name=dashboard-test",
            "-c",
            "user.email=dashboard@test.invalid",
        ])
        .args(["commit", "-qm", "fixture"])
        .current_dir(&root)
        .status()
        .unwrap();
    assert!(status.success());
    fs::create_dir_all(root.join("css/foo")).unwrap();
    fs::write(root.join("css/foo/a.html"), "a").unwrap();
    assert_eq!(list_wpt_files(&root).unwrap(), ["css/foo/a.html"]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn file_inventory_reports_empty_css_tree() {
    let root = std::env::temp_dir().join(format!("raikiri-dashboard-empty-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("css")).unwrap();
    let error = list_wpt_files(&root).unwrap_err();
    assert!(error.contains("no WPT CSS files found"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn read_baseline_normalizes_entries_and_reports_io_errors() {
    let root =
        std::env::temp_dir().join(format!("raikiri-dashboard-baseline-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let path = root.join("baseline.txt");
    fs::write(&path, "# comment\ncss/foo/a.html\na.html\n").unwrap();
    let files = vec!["css/foo/a.html".to_owned()];
    let entries = read_baseline(&path, &files).unwrap();
    assert_eq!(entries.len(), 1);
    assert!(read_baseline(&root.join("missing.txt"), &files).is_err());
    fs::remove_dir_all(root).unwrap();
}
