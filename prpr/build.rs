// Build the embedded tempo processor with the target C++ compiler.
fn main() {
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++11")
        .include("vendor/soundtouch/include")
        .include("vendor/soundtouch/source/SoundTouch")
        .define("SOUNDTOUCH_FLOAT_SAMPLES", "1")
        .file("src/practice_soundtouch.cpp");
    for entry in std::fs::read_dir("vendor/soundtouch/source/SoundTouch").unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|x| x == "cpp") {
            build.file(path);
        }
    }
    build.compile("phira_practice_soundtouch");
    println!("cargo:rerun-if-changed=vendor/soundtouch");
    println!("cargo:rerun-if-changed=src/practice_soundtouch.cpp");
}
