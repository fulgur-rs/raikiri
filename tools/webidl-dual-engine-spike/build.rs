mod codegen;

fn main() {
    println!("cargo:rerun-if-changed=idl/dom.webidl");
    println!("cargo:rerun-if-changed=codegen.rs");
    let source = std::fs::read_to_string("idl/dom.webidl").expect("IDL source");
    let interfaces = codegen::parse(&source).expect("supported IDL subset");
    let (common, boa, v8) = codegen::generate(&interfaces);
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    for (name, contents) in [("common.rs", common), ("boa.rs", boa), ("v8.rs", v8)] {
        std::fs::write(out.join(name), contents).expect("write generated bindings");
    }
}
