fn main() {
    #[cfg(windows)]
    winresource::WindowsResource::new().set_icon("packaging/icon.ico").compile().expect("embed icon");
}
