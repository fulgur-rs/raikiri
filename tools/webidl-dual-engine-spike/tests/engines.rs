use std::process::Command;

fn batch(backend: &str, mode: &str, scripts: &[&str]) -> serde_json::Value {
    static NEXT_FILE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let root = std::env::var_os("TMPDIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("HOME").expect("HOME")).join("tmp")
        });
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join(format!(
        "webidl-worker-test-{}-{}-{backend}-{mode}.json",
        std::process::id(),
        NEXT_FILE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::write(&path, serde_json::to_vec(scripts).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_webidl-dual-engine-spike"))
        .args(["--backend", backend, "--batch"])
        .arg(&path)
        .args(["--worker", mode])
        .output();
    std::fs::remove_file(path).unwrap();
    let output = output.unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("batch JSON report")
}

#[test]
fn page_realms_isolate_mutations_jobs_and_exceptions() {
    let scripts = [
        r#"const lexicalLeak = 123; (() => {
            globalThis.leak = element;
            Array.prototype.leak = 123;
            Element.prototype.getAttribute = () => 'poisoned';
            element.setAttribute('data-final', 'dirty');
            Promise.resolve().then(() => markJob());
            return JSON.stringify({dirty:true});
        })()"#,
        r#"(() => {
            checkpoint();
            if (typeof leak !== 'undefined' || typeof lexicalLeak !== 'undefined' ||
                Array.prototype.leak !== undefined || element.getAttribute('data-final') !== null ||
                element.parentNode !== documentNode) throw new Error('page contamination');
            Promise.resolve().then(() => { markJob(); element.setAttribute('data-final', 'job'); });
            checkpoint();
            return JSON.stringify({value:element.getAttribute('data-final')});
        })()"#,
        r#"globalThis.leak = 42;
            Promise.resolve().then(() => markJob());
            throw new Error('expected page failure');"#,
        r#"(() => { checkpoint();
            if (typeof leak !== 'undefined' || element.getAttribute('data-final') !== null)
                throw new Error('exception leaked state');
            return JSON.stringify({clean:true});
        })()"#,
        include_str!("contract.js"),
        include_str!("contract.js"),
    ];
    for backend in [
        #[cfg(feature = "boa")]
        "boa",
        #[cfg(feature = "v8")]
        "v8",
    ] {
        for mode in ["reuse", "fresh"] {
            let report = batch(backend, mode, &scripts);
            assert_eq!(
                report["worker_initializations"],
                if mode == "reuse" { 1 } else { 6 }
            );
            let pages = report["pages"].as_array().unwrap();
            assert_eq!(pages.len(), 6);
            assert_eq!(pages[0]["mutation_count"], 1);
            assert_eq!(pages[0]["job_callbacks"], 0);
            assert_eq!(pages[1]["results"]["value"], "job");
            assert_eq!(pages[1]["mutation_count"], 1);
            assert_eq!(pages[1]["job_callbacks"], 1);
            assert!(
                pages[2]["error"]
                    .as_str()
                    .unwrap()
                    .contains("expected page failure")
            );
            assert_eq!(pages[3]["results"]["clean"], true);
            assert_eq!(pages[3]["job_callbacks"], 0);
            assert_eq!(pages[3]["mutation_count"], 0);
            for page in &pages[4..] {
                assert_eq!(page["results"].as_array().unwrap().len(), 42);
                for row in page["results"].as_array().unwrap() {
                    assert_eq!(row["status"], "PASS", "{backend}/{mode}: {row}");
                }
                assert_eq!(page["dom_attribute"], "from-js");
            }
        }
    }
}

#[test]
fn hundreds_of_pages_without_forced_gc_allow_worker_teardown() {
    let scripts = vec!["JSON.stringify({checksum:0})"; 500];
    for backend in [
        #[cfg(feature = "boa")]
        "boa",
        #[cfg(feature = "v8")]
        "v8",
    ] {
        let report = batch(backend, "reuse", &scripts);
        assert_eq!(report["worker_initializations"], 1);
        let pages = report["pages"].as_array().unwrap();
        assert_eq!(pages.len(), 500);
        for page in pages {
            assert_eq!(page["results"]["checksum"], 0);
            assert_eq!(page["mutation_count"], 0);
            assert_eq!(page["job_callbacks"], 0);
        }
    }
}

#[test]
fn result_conversion_and_json_errors_do_not_contaminate_the_next_page() {
    let dirty = r#"globalThis.leak = element;
        element.setAttribute('data-final', 'dirty');
        Promise.resolve().then(() => markJob());"#;
    let conversion =
        format!("{dirty} ({{toString() {{ throw new Error('conversion failure'); }} }})");
    let invalid_json = format!("{dirty} 'invalid JSON'");
    let clean = r#"(() => { checkpoint();
        if (typeof leak !== 'undefined' || element.getAttribute('data-final') !== null)
            throw new Error('result failure leaked state');
        return JSON.stringify({clean:true});
    })()"#;
    for backend in [
        #[cfg(feature = "boa")]
        "boa",
        #[cfg(feature = "v8")]
        "v8",
    ] {
        let report = batch(
            backend,
            "reuse",
            &[&conversion, clean, &invalid_json, clean],
        );
        assert_eq!(report["worker_initializations"], 1);
        let pages = report["pages"].as_array().unwrap();
        assert!(
            pages[0]["error"]
                .as_str()
                .unwrap()
                .contains("conversion failure")
        );
        assert!(pages[2]["error"].is_string());
        for page in [&pages[1], &pages[3]] {
            assert_eq!(page["results"]["clean"], true);
            assert_eq!(page["mutation_count"], 0);
            assert_eq!(page["job_callbacks"], 0);
        }
    }
}

#[test]
fn repeated_contract_uses_one_worker_and_fresh_documents() {
    for backend in [
        #[cfg(feature = "boa")]
        "boa",
        #[cfg(feature = "v8")]
        "v8",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_webidl-dual-engine-spike"))
            .args(["--backend", backend, "--repeat", "20", "--worker", "reuse"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["worker_initializations"], 1);
        let pages = report["pages"].as_array().unwrap();
        assert_eq!(pages.len(), 20);
        let mutations = &pages[0]["mutation_count"];
        for page in pages {
            assert_eq!(page["mutation_count"], *mutations);
            assert_eq!(page["dom_attribute"], "from-js");
            for row in page["results"].as_array().unwrap() {
                assert_eq!(row["status"], "PASS", "{backend}: {row}");
            }
        }
    }
}

fn contract(backend: &str) {
    let output = Command::new(env!("CARGO_BIN_EXE_webidl-dual-engine-spike"))
        .args(["--backend", backend])
        .output()
        .expect("run prototype");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.stdout.is_empty(),
        "backend must report executed JS contract results"
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON report");
    let results = report["results"].as_array().expect("contract results");
    assert_eq!(results.len(), 42);
    for result in results {
        assert_eq!(result["status"], "PASS", "backend={backend}: {result}");
    }
    assert_eq!(
        report["dom_attribute"], "from-js",
        "JS must mutate the real Rust Document"
    );
}

#[test]
#[cfg(feature = "boa")]
fn boa_generated_bindings_execute_the_dom_contract() {
    contract("boa");
}

#[test]
#[cfg(feature = "v8")]
fn v8_generated_bindings_execute_the_dom_contract() {
    contract("v8");
}

#[test]
fn reports_the_utf16_storage_limit_on_each_enabled_backend() {
    for backend in [
        #[cfg(feature = "boa")]
        "boa",
        #[cfg(feature = "v8")]
        "v8",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_webidl-dual-engine-spike"))
            .args([
                "--backend",
                backend,
                "--script",
                concat!(env!("CARGO_MANIFEST_DIR"), "/tests/surrogate.js"),
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["results"]["roundtrip"], false);
        assert_eq!(report["results"]["error"], "TypeError");
    }
}
