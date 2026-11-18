//! A host's product name (`set_product_name`): its own test binary, as the name is process-wide.

use solidcraft_ui_text::{Catalog, Locale, branded, install, product_name, set_product_name};

static ENGLISH: Catalog = Catalog::new(
    "en",
    "English",
    r#"{"About SolidCraft":"About SolidCraft","update.available":"SolidCraft {version} is available","ui.cancel":"Cancel"}"#,
);
static CHINESE: Catalog = Catalog::new("zh", "中文", r#"{"About SolidCraft":"关于 SolidCraft","update.available":"SolidCraft {version} 可用"}"#);
static LOCALES: &[&Catalog] = &[&ENGLISH, &CHINESE];

/// Every message names the host's product, in every language; source labels written with
/// SolidCraft's name still find their translation; other text is branded on request.
#[test]
fn messages_name_the_hosts_product() {
    assert_eq!(product_name(), "SolidCraft");
    assert_eq!(branded("SolidCraft cut: Body1"), "SolidCraft cut: Body1");
    assert!(set_product_name("SolidZ"));
    assert!(!set_product_name("Other"), "named once");
    install(LOCALES);
    let (en, zh) = (Locale::for_language("en"), Locale::for_language("zh"));
    assert_eq!(en.format("update.available", &[("version", "1")]), "SolidZ 1 is available");
    assert_eq!(zh.format("update.available", &[("version", "1")]), "SolidZ 1 可用");
    assert_eq!(en.label("About SolidCraft"), "About SolidZ");
    assert_eq!(zh.label("About SolidCraft"), "关于 SolidZ");
    assert_eq!(zh.label("Cancel"), "Cancel");
    assert_eq!(branded("SolidCraft cut: Body1"), "SolidZ cut: Body1");
    assert_eq!(product_name(), "SolidZ");
}
