#[cfg(not(any(feature = "boa", feature = "v8")))]
compile_error!("select a JavaScript backend");

#[cfg(feature = "boa")]
mod boa_backend;
mod shared;
#[cfg(feature = "v8")]
mod v8_backend;

mod generated {
    use crate::shared::{BindingCall, Interface, NativeValue};
    include!(concat!(env!("OUT_DIR"), "/common.rs"));
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

enum Worker {
    #[cfg(feature = "boa")]
    Boa(Box<boa_backend::Worker>),
    #[cfg(feature = "v8")]
    V8(v8_backend::Worker),
}

impl Worker {
    fn new(backend: &str) -> Result<Self, String> {
        match backend {
            #[cfg(feature = "boa")]
            "boa" => boa_backend::Worker::new().map(|worker| Self::Boa(Box::new(worker))),
            #[cfg(feature = "v8")]
            "v8" => v8_backend::Worker::new().map(Self::V8),
            _ => Err("unknown backend".into()),
        }
    }

    fn run_page(&mut self, script: &str) -> Result<shared::PageOutput, String> {
        match self {
            #[cfg(feature = "boa")]
            Self::Boa(worker) => worker.run_page(script),
            #[cfg(feature = "v8")]
            Self::V8(worker) => worker.run_page(script),
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut backend = None;
    let mut input = None;
    let mut repeat = None;
    let mut mode = "reuse".to_owned();
    while let Some(flag) = args.next() {
        let value = args.next().ok_or("each option requires a value")?;
        match flag.as_str() {
            "--backend" => backend = Some(value),
            "--script" | "--batch" if input.is_none() => input = Some((flag, value)),
            "--repeat" => {
                let count: usize = value.parse().map_err(|_| "invalid repeat count")?;
                if !(1..=10000).contains(&count) {
                    return Err("repeat must be 1..10000".into());
                }
                repeat = Some(count);
            }
            "--worker" if matches!(value.as_str(), "reuse" | "fresh") => mode = value,
            _ => return Err("unknown or conflicting option".into()),
        }
    }
    let backend = backend.ok_or("--backend boa|v8 is required")?;
    let batch_input = input.as_ref().is_some_and(|(flag, _)| flag == "--batch");
    if batch_input && repeat.is_some() {
        return Err("--batch and --repeat cannot be combined".into());
    }
    let scripts: Vec<String> = match input {
        Some((flag, path)) if flag == "--batch" => {
            serde_json::from_str(&std::fs::read_to_string(path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
        }
        Some((_, path)) => {
            vec![std::fs::read_to_string(path).map_err(|e| e.to_string())?; repeat.unwrap_or(1)]
        }
        None => vec![include_str!("../tests/contract.js").to_owned(); repeat.unwrap_or(1)],
    };
    if scripts.is_empty() {
        return Err("batch must contain at least one script".into());
    }
    let is_batch = batch_input || repeat.is_some();
    let started = std::time::Instant::now();
    let mut worker = None;
    let mut initializations = 0;
    let mut init_ms = 0.0;
    let mut pages = Vec::with_capacity(scripts.len());
    for script in scripts {
        if worker.is_none() {
            let init = std::time::Instant::now();
            worker = Some(Worker::new(&backend)?);
            init_ms += init.elapsed().as_secs_f64() * 1000.0;
            initializations += 1;
        }
        let page_start = std::time::Instant::now();
        let result = worker
            .as_mut()
            .expect("worker initialized")
            .run_page(&script);
        let page_ms = page_start.elapsed().as_secs_f64() * 1000.0;
        let mut page = match result {
            Ok(output) => serde_json::to_value(output).map_err(|e| e.to_string())?,
            Err(error) if is_batch => serde_json::json!({"error": error}),
            Err(error) => return Err(error),
        };
        page["elapsed_ms"] = page_ms.into();
        pages.push(page);
        if mode == "fresh" {
            drop(worker.take());
        }
    }
    drop(worker);
    let total_ms = started.elapsed().as_secs_f64() * 1000.0;
    let report = if is_batch {
        serde_json::json!({"engine":backend,"worker":mode,"worker_initializations":initializations,
            "worker_init_ms":init_ms,"elapsed_ms":total_ms,"pages":pages})
    } else {
        let mut page = pages.pop().expect("nonempty batch");
        page["engine"] = backend.into();
        page["elapsed_ms"] = total_ms.into();
        page
    };
    println!("{report}");
    Ok(())
}
