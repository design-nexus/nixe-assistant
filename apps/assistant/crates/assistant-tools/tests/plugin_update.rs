//! An update keeps what was reviewed: the same tools and the same sandbox.
use assistant_core::config::SandboxProfile;
use assistant_tools::plugins::{self, Manifest};
use serde_json::json;

fn manifest(sandbox: Option<SandboxProfile>) -> Manifest {
    Manifest {
        manifest_version: 1,
        id: "fixture".into(),
        version: "1.0.0".into(),
        description: "fixture".into(),
        executable: "python3".into(),
        args: vec![],
        tools: vec!["fixture.echo".into()],
        dependencies: vec![],
        optional_dependencies: vec![],
        routes: vec![],
        providers: vec![],
        settings_schema: json!({"type":"object","additionalProperties":false}),
        sandbox,
    }
}

fn staged(m: &Manifest) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("plugin.toml"), toml::to_string(m).unwrap()).unwrap();
    dir
}

#[test]
fn an_update_cant_widen_the_sandbox() {
    let temp = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_DATA_HOME", temp.path().join("data"));
    std::env::set_var("XDG_CONFIG_HOME", temp.path().join("config"));
    let first = staged(&manifest(None));
    plugins::install(first.path(), "local", false).unwrap();
    let same = staged(&manifest(None));
    plugins::install(same.path(), "local", true).expect("same surface updates");
    let wider = staged(&manifest(Some(SandboxProfile { network: true, ..Default::default() })));
    let err = plugins::install(wider.path(), "local", true).unwrap_err();
    assert!(err.to_string().contains("permission surface changed"), "{err}");
}
