//! The fork must not claim the original client's installation or data.
use spotifast::{identity, paths::AppDirs, updates};

#[test]
fn native_identity_and_data_are_separate_from_the_original() {
    assert_eq!(identity::SLUG, "spotifast-x");
    assert_eq!(identity::APP_ID, "io.github.Astonikum.SpotifastX");
    let original = directories::ProjectDirs::from("me", "paolino", "spotifast").unwrap();
    let fork = AppDirs::installed();
    assert_ne!(fork.config, original.config_dir());
    assert_ne!(fork.cache, original.cache_dir());
    assert_ne!(
        fork.state,
        original.state_dir().unwrap_or(original.data_local_dir())
    );
    assert_eq!(updates::CONFIG.repository, "Astonikum/Spotifast-X");
    assert_eq!(updates::CONFIG.macos.bundle_ids, &[identity::APP_ID]);
}

#[test]
fn windows_packages_keep_the_forks_own_stable_upgrade_identity() {
    let msi = include_str!("../packaging/windows/spotifast.wxs");
    let inno = include_str!("../packaging/windows/spotifast.iss");
    for forbidden in [
        "3DFFED43-88FE-4A31-9461-50BE03094316",
        "8EC6378E-B3EB-41F9-873B-01AE4466E5EE",
        "FCED1EA0-EBF5-4C32-BA3B-A3AD724BACC3",
    ] {
        assert!(
            !msi.contains(forbidden),
            "original MSI identity: {forbidden}"
        );
        assert!(
            !inno.contains(forbidden),
            "original Inno identity: {forbidden}"
        );
    }
    assert!(msi.contains("Identity = \"SpotifastX\""));
    assert!(msi.contains("UpgradeCode = \"{18C9FCBE-EA54-5950-A90D-2C892B456B93}\""));
    assert!(msi.contains("ProgramComponent = \"{AE28F2C7-D9ED-57C6-8588-892F1C3C99CA}\""));
    assert!(msi.contains("InnoAppId = \"{27C41F04-736F-5218-9F62-736A70106ED4}\""));
    assert!(inno.contains("AppId={{27C41F04-736F-5218-9F62-736A70106ED4}"));
    assert!(inno.contains("#define AppIdentity \"SpotifastX\""));
    assert!(inno.contains("Software\\Classes\\SpotifastX.spotify"));
    // HKCR includes both per-user and system-wide handlers.
    assert!(msi.contains("Id=\"LinkCommand\" Root=\"HKCR\""));
    assert!(inno.contains("RegQueryStringValue(HKCR, 'spotify\\shell\\open\\command'"));
}
