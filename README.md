# Cat Tetris

Rust + macroquad. Runs on Windows and macOS (and Linux).

## Run
    cargo run --release

## macOS app bundle (universal: Apple Silicon + Intel)
    ./build_macos.sh
Produces `dist/Cat Tetris.app` and `dist/CatTetris-macos.zip`.

The app is only ad-hoc signed. If macOS blocks a copy you downloaded, right-click the app,
choose Open, or run: `xattr -dr com.apple.quarantine "Cat Tetris.app"`

## Controls
Arrows move / soft drop, Up or X rotate, Z rotate back, Space hard drop, P pause, M music, Esc quit.
