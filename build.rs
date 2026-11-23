fn main() {
    println!("cargo:rerun-if-changed=.env");
    println!("cargo:rerun-if-env-changed=NOIA_RELEASE_URL");
    let releases = std::env::var("NOIA_RELEASE_URL").unwrap_or_else(|_| {
        include_str!(".env")
            .lines()
            .find_map(|line| line.strip_prefix("NOIA_RELEASE_URL="))
            .expect("Set NOIA_RELEASE_URL in .env or the build environment")
            .trim()
            .to_owned()
    });
    let releases = releases.trim_end_matches('/');
    let parts: Vec<_> = releases
        .strip_prefix("https://github.com/")
        .unwrap_or("")
        .split('/')
        .collect();
    assert!(
        parts.len() == 3
            && parts[2] == "releases"
            && parts[..2].iter().all(|s| !s.is_empty()
                && *s != "."
                && *s != ".."
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))),
        "NOIA_RELEASE_URL must be https://github.com/OWNER/REPO/releases"
    );
    println!("cargo:rustc-env=NOIA_RELEASE_URL={releases}");
    println!("cargo:rerun-if-changed=locales");
    println!("cargo:rerun-if-changed=assets/app.ico");
    #[cfg(windows)]
    winresource::WindowsResource::new()
        .set_icon("assets/app.ico")
        .set("ProductName", "Noia DNF")
        .set("FileDescription", "Noia DNF")
        .compile()
        .expect("Failed to compile Windows icon resources");
}
