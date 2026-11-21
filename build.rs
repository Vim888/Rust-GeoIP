fn main() {
    cc::Build::new()
        .file("src/bridge.c")
        .compile("geoip_bridge");
    println!("cargo:rerun-if-changed=src/bridge.c");
    println!("cargo:rustc-link-lib=GeoIP");
}
