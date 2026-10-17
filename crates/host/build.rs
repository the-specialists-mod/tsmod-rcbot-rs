//! The version the plugin reports to the port: `TSMOD_RCBOT_VERSION` when set (the release
//! workflow sets it from the `v*` tag it builds), else the crate's.
fn main() {
    println!("cargo:rerun-if-env-changed=TSMOD_RCBOT_VERSION");
    let v =
        std::env::var("TSMOD_RCBOT_VERSION").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| std::env::var("CARGO_PKG_VERSION").unwrap());
    let v = v.strip_prefix('v').unwrap_or(&v);
    assert!(!v.contains('\0'), "TSMOD_RCBOT_VERSION has a NUL");
    println!("cargo:rustc-env=TSMOD_RCBOT_VERSION={v}");
}
