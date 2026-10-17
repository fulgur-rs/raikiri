#[path = "../../codegen.rs"]
mod codegen;

fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    assert_eq!(
        args.len(),
        3,
        "usage: idl-generate <source.webidl> <output-directory>"
    );
    let source = std::fs::read_to_string(&args[1]).expect("IDL source");
    let interfaces = codegen::parse(&source).expect("supported IDL subset");
    let (common, boa, v8) = codegen::generate(&interfaces);
    let out = std::path::Path::new(&args[2]);
    std::fs::create_dir_all(out).expect("output directory");
    for (name, contents) in [("common.rs", common), ("boa.rs", boa), ("v8.rs", v8)] {
        std::fs::write(out.join(name), contents).expect("generated bindings");
    }
}
