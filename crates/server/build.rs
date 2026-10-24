fn main() {
    // Rebuild when the web UI is rebuilt so the embedded copy never goes stale.
    println!("cargo:rerun-if-changed=../../web/dist");
}
