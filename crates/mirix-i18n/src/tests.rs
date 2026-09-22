use super::*;

#[test]
fn engine_catalogs_keep_their_own_fallback_and_external_overrides() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("zh-CN.ftl"), "title = 自定义输入法\n").unwrap();
    let locale = Localizer::with_catalogs(
        "zh-CN",
        &[root.path().to_owned()],
        &[
            (
                "en-US",
                "title = Example\ndescription = Engine description\n",
            ),
            ("zh-CN", "title = 示例\n"),
        ],
    )
    .unwrap();
    assert_eq!(locale.text("title"), "自定义输入法");
    assert_eq!(locale.text("description"), "Engine description");
    assert!(Localizer::with_catalogs("en-US", &[], &[("zh-CN", "title = 示例")]).is_err());
}

#[test]
fn language_negotiation_handles_regions_order_and_unavailable_languages() {
    for (requested, expected) in [
        (vec!["zh-CN"], "zh-CN"),
        (vec!["zh-Hans"], "zh-CN"),
        (vec!["zh"], "zh-CN"),
        (vec!["en-GB"], "en-US"),
        (vec!["de-DE", "zh-CN"], "zh-CN"),
        (vec!["de-DE"], "en-US"),
        (vec![], "en-US"),
    ] {
        let requested = requested
            .into_iter()
            .map(|language| language.parse().unwrap())
            .collect::<Vec<_>>();
        let localizer = Localizer::load_requested(&requested, &[]).unwrap();
        assert_eq!(localizer.language().to_string(), expected);
    }
}

#[test]
fn external_language_is_discovered_and_partial_messages_use_english() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("fr-FR.ftl"),
        "language-name = Français\nmenu-language = Langue\ntray-quit = Quitter Mirix\n",
    )
    .unwrap();
    let localizer = Localizer::load(Some("fr-FR"), &[root.path().to_owned()]).unwrap();
    assert_eq!(localizer.text("tray-quit"), "Quitter Mirix");
    assert_eq!(localizer.text("tray-restart"), "Restart Mirix");
    assert!(
        localizer
            .available_languages()
            .iter()
            .any(|language| language.id == "fr-FR" && language.name == "Français")
    );
    assert_eq!(localizer.text("menu-language"), "Langue");
}

#[test]
fn directory_precedence_and_invalid_resource_or_reference_do_not_break_ui() {
    let system = tempfile::tempdir().unwrap();
    let user = tempfile::tempdir().unwrap();
    std::fs::write(system.path().join("zh-CN.ftl"), "tray-quit = 系统退出\n").unwrap();
    std::fs::write(
        user.path().join("zh-CN.ftl"),
        "tray-quit = 用户退出\ntray-restart = { missing-message }\n",
    )
    .unwrap();
    std::fs::write(user.path().join("ja-JP.ftl"), "tray-quit = {\n").unwrap();
    std::fs::write(
        user.path().join("en-US.ftl"),
        "tray-restart = { also-missing }\n",
    )
    .unwrap();
    let localizer =
        Localizer::load(Some("zh-CN"), &[system.path().into(), user.path().into()]).unwrap();
    assert_eq!(localizer.text("tray-quit"), "用户退出");
    assert_eq!(localizer.text("menu-skins"), "皮肤");
    assert_eq!(localizer.text("tray-restart"), "Restart Mirix");
    assert!(
        !localizer
            .available_languages()
            .iter()
            .any(|language| language.id == "ja-JP")
    );
}

#[test]
fn all_builtin_messages_resolve_with_fluent_arguments_in_both_languages() {
    let chinese = Localizer::load(Some("zh-CN"), &[]).unwrap();
    let english = Localizer::load(Some("en-US"), &[]).unwrap();
    // Check each built-in translation directly so English completion cannot
    // hide a missing or malformed Simplified Chinese message.
    let mut args = FluentArgs::new();
    args.set("method", "Pinyin");
    args.set("shortcut", "Ctrl+Space");
    args.set("current", 2);
    args.set("total", 9);
    for id in ENGLISH
        .lines()
        .filter_map(|line| line.split_once(" = ").map(|(id, _)| id))
    {
        assert!(
            message(&english.bundles[0], id, Some(&args)).is_some(),
            "{id}"
        );
        assert!(
            message(&chinese.bundles[0], id, Some(&args)).is_some(),
            "{id}"
        );
    }
    assert!(chinese.format("tray-title", Some(&args)).contains("Pinyin"));
    assert_eq!(chinese.text("language-name"), "简体中文");
}
