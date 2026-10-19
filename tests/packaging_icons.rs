//! Packaging icons must load in Qt's SVG renderer (system tray / desktop).
use std::fs;
use std::path::PathBuf;

#[test]
fn linux_tray_svg_avoids_filters_qt_rejects() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("packaging/icons/spotifast.svg");
    let svg = fs::read_to_string(&path).expect("spotifast.svg");
    assert!(
        !svg.contains("feFuncA") && !svg.contains("<filter"),
        "Qt 6 rejects feFuncA inside feComponentTransfer; the tray then shows a checkerboard (#660)"
    );
    assert!(svg.contains("viewBox"), "icon must remain a valid SVG mark");
}

#[test]
fn packaging_uses_the_supplied_spotifast_x_svg() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let supplied = fs::read(root.join("assets/SpotifastX.svg")).unwrap();
    assert_eq!(
        fs::read(root.join("packaging/icons/spotifast.svg")).unwrap(),
        supplied
    );
    assert_eq!(
        fs::read(root.join("docs/assets/images/logo.svg")).unwrap(),
        supplied
    );
}
