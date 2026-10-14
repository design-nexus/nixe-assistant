//! First-party manifests are refreshed when their tools change; user packages are not.
use assistant_core::Config;
use assistant_tools::plugins;
use std::path::Path;

#[tokio::test]
async fn migration_refreshes_first_party_manifests() {
    let temp = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_DATA_HOME", temp.path().join("data"));
    std::env::set_var("XDG_CONFIG_HOME", temp.path().join("config"));
    let mut cfg = Config::default();
    cfg.browser.enabled = false;
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let worker = Path::new("/usr/bin/true");
    plugins::migrate(&cfg, worker, &checkout).await.unwrap();

    let read = |id: &str| std::fs::read_to_string(plugins::package_dir().join(id).join("plugin.toml")).unwrap();
    for (id, tool) in [
        ("windows", "windows.focus"),
        ("services", "services.control"),
        ("network", "network.wifi"),
        ("admin", "admin.install"),
        ("admin", "packages.search"),
        ("services", "services.system_status"),
        ("files", "files.write"),
        ("desktop-controls", "audio.devices"),
    ] {
        assert!(read(id).contains(tool), "{id} lacks {tool}");
    }
    assert!(plugins::saved().unwrap()["windows"].enabled, "new groups start enabled");

    // An install from before this release: the files manifest only knew files.read.
    let files = plugins::package_dir().join("files/plugin.toml");
    let old = read("files").replace("\"files.write\",", "").replace("\"files.list\",", "");
    assert!(!old.contains("files.write"), "the simulated old manifest lacks the new tool");
    std::fs::write(&files, &old).unwrap();
    let mut state = plugins::saved().unwrap();
    state.get_mut("files").unwrap().enabled = false;
    plugins::save(&state).unwrap();
    plugins::migrate(&cfg, worker, &checkout).await.unwrap();
    assert!(read("files").contains("files.write"), "refreshed");
    assert!(!plugins::saved().unwrap()["files"].enabled, "the user's enabled choice is kept");

    // A package the user installed under a first-party ID stays as it is.
    let mut state = plugins::saved().unwrap();
    state.get_mut("network").unwrap().source = "https://example.invalid/network".into();
    plugins::save(&state).unwrap();
    std::fs::write(plugins::package_dir().join("network/plugin.toml"), "user = true\n").unwrap();
    plugins::migrate(&cfg, worker, &checkout).await.unwrap();
    assert_eq!(read("network"), "user = true\n");
}
