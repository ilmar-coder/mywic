//! No Windows, embute o ícone no EcoReset.exe.
fn main() {
    println!("cargo:rerun-if-changed=../../packaging/icon/ecoreset.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../packaging/icon/ecoreset.ico");
        res.set("FileDescription", "EcoReset");
        res.set("ProductName", "EcoReset");
        if let Err(e) = res.compile() {
            println!("cargo:warning=ícone não embutido: {e}");
        }
    }
}
