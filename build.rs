fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winres::WindowsResource::new()
            .set_icon("assets/tetris.ico")
            .compile()
            .expect("failed to embed Windows application icon");
    }
}
