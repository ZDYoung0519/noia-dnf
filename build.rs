fn main() {
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
