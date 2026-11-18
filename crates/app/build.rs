//! No Windows, embute o ícone no EcoReset.exe (também ao gerar no Mac com o MinGW).
fn main() {
    println!("cargo:rerun-if-changed=../../packaging/icon/ecoreset.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu") && !cfg!(windows) {
        res.set_windres_path("x86_64-w64-mingw32-windres");
        res.set_ar_path("x86_64-w64-mingw32-ar");
    }
    res.set_icon("../../packaging/icon/ecoreset.ico");
    res.set("FileDescription", "EcoReset");
    res.set("ProductName", "EcoReset");
    if let Err(e) = res.compile() {
        println!("cargo:warning=ícone não embutido: {e}");
    }
}
