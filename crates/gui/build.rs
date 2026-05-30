fn main() {
    #[cfg(target_os = "windows")]
    {
        let icon = "assets/voidcore-icon.ico";
        println!("cargo:rerun-if-changed={icon}");
        println!("cargo:rerun-if-changed=assets/voidcore-icon.png");

        let mut res = winres::WindowsResource::new();
        res.set_icon(icon);
        res.compile().expect("failed to embed Windows application icon");
    }
}
