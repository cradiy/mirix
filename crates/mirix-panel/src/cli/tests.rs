use super::Cli;
use clap::Parser;

#[test]
fn settings_file_preview_is_separate_from_connected_and_candidate_modes() {
    let cli = Cli::try_parse_from([
        "mirix-panel",
        "--settings-demo",
        "page.json",
        "--language",
        "zh-CN",
    ])
    .unwrap();
    assert_eq!(cli.settings_demo.unwrap().to_str(), Some("page.json"));
    assert!(cli.socket.is_none());
    for extra in [
        vec!["--settings"],
        vec!["--socket", "demo.sock"],
        vec!["--demo"],
        vec!["--skin", "skin"],
    ] {
        let mut args = vec!["mirix-panel", "--settings-demo", "page.json"];
        args.extend(extra);
        assert!(Cli::try_parse_from(args).is_err());
    }
}
