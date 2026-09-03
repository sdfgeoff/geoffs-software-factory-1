use std::{path::Path, process::Command};

fn main() {
    for path in [
        "frontend/index.html",
        "frontend/package.json",
        "frontend/package-lock.json",
        "frontend/src",
        "frontend/tsconfig.json",
        "frontend/vite.config.ts",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }

    let frontend = Path::new("frontend");
    if !frontend.join("node_modules").exists() {
        run(frontend, "npm", &["ci"]);
    }
    run(frontend, "npm", &["run", "build"]);

    // rust-embed resolves paths relative to the crate when compiling.
    println!(
        "cargo:rustc-env=FRONTEND_DIST={}",
        frontend.join("dist").display()
    );
}

fn run(dir: &Path, program: &str, args: &[&str]) {
    let status = Command::new(program)
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap_or_else(|error| panic!("failed to run {program}: {error}"));

    if !status.success() {
        panic!("{program} {} failed with {status}", args.join(" "));
    }
}
