use std::path::PathBuf;

fn main() {
    tauri_build::build();

    // The AVR toolchain + simavr live in a git-ignored `.toolchain/` folder at the
    // workspace root (see README "Development notes"). Resolve it relative to
    // this crate so both linking and bindgen agree.
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.parent().expect("src-tauri has a parent").to_path_buf();
    let simavr = root.join(".toolchain/simavr/simavr");

    // The object directory is named after the *host triplet*, and that is not the same
    // everywhere: obj-x86_64-pc-linux-gnu here, obj-x86_64-linux-gnu on a GitHub runner.
    // Assuming one of them panicked five minutes into a CI build, on a machine where
    // simavr had compiled perfectly — so find whichever directory holds the archive.
    let libdir = std::fs::read_dir(&simavr)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("obj-"))
        })
        .find(|path| path.join("libsimavr.a").exists())
        .unwrap_or_else(|| {
            panic!(
                "no libsimavr.a under {}/obj-*. Build it first:\n  \
                 git clone --depth 1 https://github.com/buserror/simavr .toolchain/simavr\n  \
                 make -C .toolchain/simavr -j build-simavr",
                simavr.display()
            )
        });

    // Linked *statically*, deliberately.
    //
    // This used to link libsimavr.so with an rpath baked into the binary pointing at
    // `.toolchain/` — a git-ignored development path. That works on the machine that
    // built it and nowhere else, which is fine until the first release. The archive is
    // built by the same `make` that produces the shared object, so there is nothing to
    // gain from the dynamic version.
    println!("cargo:rustc-link-search=native={}", libdir.display());
    println!("cargo:rustc-link-lib=static=simavr");
    // simavr's own C dependencies: libelf for ELF loading, libm for the floating point.
    println!("cargo:rustc-link-lib=dylib=elf");
    println!("cargo:rustc-link-lib=dylib=m");

    let bindings = bindgen::Builder::default()
        .header_contents(
            "wrapper.h",
            "#include \"sim_avr.h\"\n\
             #include \"sim_elf.h\"\n\
             #include \"avr_ioport.h\"\n\
             #include \"avr_uart.h\"\n\
             #include \"avr_adc.h\"\n",
        )
        .clang_arg(format!("-I{}", simavr.display()))
        .clang_arg(format!("-I{}", simavr.join("sim").display()))
        .allowlist_function("avr_.*")
        .allowlist_function("elf_read_firmware")
        .allowlist_type("avr_.*")
        .allowlist_type("elf_firmware_t")
        .allowlist_var("AVR_.*")
        .allowlist_var("IOPORT_.*")
        .allowlist_var("UART_.*")
        .allowlist_var("ADC_.*")
        .allowlist_var("cpu_.*")
        .layout_tests(false)
        .generate()
        .expect("failed to generate simavr bindings");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out.join("simavr_bindings.rs"))
        .expect("failed to write simavr bindings");

    println!("cargo:rerun-if-changed=build.rs");

    // A small C shim to read avr_t fields that bindgen left opaque.
    cc::Build::new()
        .file("simavr_shim.c")
        .include(simavr.join("sim"))
        .include(&simavr)
        .flag("-Wno-unused-parameter")
        .compile("simavr_shim");
    println!("cargo:rerun-if-changed=simavr_shim.c");
}
