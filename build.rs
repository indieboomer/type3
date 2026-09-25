use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=src/renderer/sphere.wgsl");
    let source = fs::read_to_string("src/renderer/sphere.wgsl").expect("read shader");
    let module = naga::front::wgsl::parse_str(&source)
        .unwrap_or_else(|error| panic!("{}", error.emit_to_string(&source)));
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .expect("validate shader");
    for (entry, stage) in [
        ("vs_main", naga::ShaderStage::Vertex),
        ("fs_main", naga::ShaderStage::Fragment),
    ] {
        let pipeline = naga::back::spv::PipelineOptions {
            shader_stage: stage,
            entry_point: entry.into(),
        };
        let options = naga::back::spv::Options::default();
        let words = naga::back::spv::write_vec(&module, &info, &options, Some(&pipeline))
            .expect("compile SPIR-V");
        let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        fs::write(
            PathBuf::from(env::var_os("OUT_DIR").unwrap()).join(format!("{entry}.spv")),
            bytes,
        )
        .unwrap();
    }
}
