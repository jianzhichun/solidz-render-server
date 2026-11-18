//! An embedding host's folder for SolidCraft's per-user files (`recovery::set_user_dir`). In a
//! test binary of its own: the folder is process-wide.

use solidcraft_engine::{crash, recovery};

/// Data, cache, recovery and crash reports all go in the host's folder; the first folder set stays.
#[test]
fn the_hosts_folder_holds_every_per_user_file() {
    let dir = std::env::temp_dir().join(format!("solidcraft-user-dir-{}", std::process::id()));
    assert!(recovery::set_user_dir(dir.clone()));
    assert!(!recovery::set_user_dir(dir.join("other")), "the first folder stays");
    assert_eq!((recovery::data_dir(), recovery::cache_dir()), (Some(dir.clone()), Some(dir.clone())));
    // The variables that name the folders themselves still win.
    if std::env::var_os("SOLIDCRAFT_RECOVERY_DIR").is_none() && std::env::var_os("SOLIDCRAFT_CRASH_DIR").is_none() {
        let named = |d: Option<std::path::PathBuf>| d.and_then(|d| Some((d.parent()?.file_name()?.to_owned(), d.file_name()?.to_owned())));
        let top = dir.file_name().unwrap().to_owned();
        assert_eq!(named(recovery::default_dir()), Some((top.clone(), "recovery".into())));
        assert_eq!(named(crash::default_dir()), Some((top, "crashes".into())));
    }
}
