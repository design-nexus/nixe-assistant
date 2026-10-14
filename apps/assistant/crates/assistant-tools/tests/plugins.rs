use assistant_core::Config;
use assistant_tools::{
    plugins::{self, Manifest, PluginManager, Route, Saved},
    registry::Registry,
};
use assistant_policy::{decide, AutonomyMode, Context, Decision, Grant, Rules};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::Path};
use tokio_util::sync::CancellationToken;
/// A grant the host has already checked, as the plugin manager receives it.
fn checked(tool: &str, args: &Value) -> Grant {
    let rules = Rules::default();
    let verdict = decide(
        tool,
        &Context {
            available: true,
            trusted: true,
            tool_rule: Some(Decision::Allow),
            rules: &rules,
            mode: AutonomyMode::Autonomous,
        },
    );
    let grant = Grant::allowed(&verdict, args).unwrap();
    grant.check(tool, args).unwrap();
    grant
}
fn manifest(id: &str, tools: Vec<String>, args: Vec<String>) -> Manifest {
    Manifest {
        manifest_version: 1,
        id: id.into(),
        version: "1.0.0".into(),
        description: "fixture".into(),
        executable: "python3".into(),
        args,
        tools,
        dependencies: vec![],
        optional_dependencies: vec![],
        routes: vec![],
        providers: vec![],
        settings_schema: json!({"type":"object","additionalProperties":false}),
    }
}
fn package(m: &Manifest) {
    let dir = plugins::package_dir().join(&m.id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("plugin.toml"), toml::to_string(m).unwrap()).unwrap();
}
#[tokio::test]
async fn manifest_dispatch_reload_disable_and_extraction() {
    let temp = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_DATA_HOME", temp.path().join("data"));
    std::env::set_var("XDG_CONFIG_HOME", temp.path().join("config"));
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/mcp_fixture.py");
    let m = manifest(
        "fixture",
        vec!["fixture.echo".into()],
        vec!["-u".into(), fixture.display().to_string()],
    );
    let mut malformed = m.clone();
    malformed.manifest_version = 8;
    assert!(malformed.validate().is_err());
    malformed = m.clone();
    malformed.id = "../escape".into();
    assert!(malformed.validate().is_err());
    malformed = m.clone();
    malformed.tools.push("fixture.echo".into());
    assert!(malformed.validate().is_err());
    malformed = m.clone();
    malformed.settings_schema = json!({"type":"not-a-json-schema-type"});
    assert!(malformed.validate().is_err());
    package(&m);
    let mut cfg = Config::default();
    cfg.mcp.servers.insert(
        "fixture".into(),
        assistant_core::config::McpServerCfg {
            command: "python3".into(),
            args: m.args.clone(),
            env: std::collections::HashMap::from([(
                "FIXTURE_LOG".into(),
                temp.path().join("log").display().to_string(),
            )]),
        },
    );
    let manager = PluginManager::new();
    manager.change("enable", "fixture", &cfg).await.unwrap();

    let mut route_a = m.clone();
    route_a.routes.push(Route {
        pattern: "hello".into(),
        tool: "fixture.echo".into(),
        template: false,
    });
    package(&route_a);
    manager.change("reload", "fixture", &cfg).await.unwrap();
    let mut dependent = manifest("dependent", vec!["echo".into()], m.args.clone());
    dependent.dependencies = vec!["fixture".into()];
    dependent.routes.push(Route {
        pattern: "^h(?P<rest>.+)$".into(),
        tool: "echo".into(),
        template: true,
    });
    package(&dependent);
    manager.change("enable", "dependent", &cfg).await.unwrap();
    assert!(manager
        .route("hello", &CancellationToken::new())
        .await
        .is_err());
    let generation = manager.generation("fixture.echo").unwrap();
    assert!(manager.change("disable", "fixture", &cfg).await.is_err());
    manager.change("disable", "dependent", &cfg).await.unwrap();
    assert!(manager
        .validate("fixture.echo", &json!({"text":42}))
        .is_err());
    let second = manifest("collision", m.tools.clone(), m.args.clone());
    package(&second);
    assert!(manager.change("enable", "collision", &cfg).await.is_err());
    let mut broken = m.clone();
    broken.executable = "/missing/executable".into();
    package(&broken);
    assert!(manager.change("reload", "fixture", &cfg).await.is_err());
    assert_eq!(manager.generation("fixture.echo"), Some(generation));
    assert_eq!(
        manager
            .call(
                "fixture.echo",
                json!({"text":"working"}),
                &cfg,
                &CancellationToken::new(),
                &checked("fixture.echo", &json!({"text":"working"})),
            )
            .await
            .unwrap()["structuredContent"]["text"],
        "working"
    );
    package(&m);
    let owned = manager.token("fixture.echo");
    let cancel = CancellationToken::new();
    let grant = checked("fixture.echo", &json!({"text":"wait"}));
    let waiting = manager.call("fixture.echo", json!({"text":"wait"}), &cfg, &cancel, &grant);
    let disable = async {
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        manager.change("disable", "fixture", &cfg).await.unwrap();
    };
    let (result, _) = tokio::join!(waiting, disable);
    assert!(result.is_err());
    assert!(!manager.available("fixture.echo"));
    assert!(owned.is_cancelled());
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(
        !Path::new(&format!("/proc/{generation}")).exists(),
        "plugin process was not reaped"
    );
    // Newly installed third-party packages remain disabled and default to confirmation.
    let registry = Registry::load(&cfg).await;
    let mut prompt_cfg = cfg.clone();
    prompt_cfg.clock.inject_prompt = true;
    prompt_cfg.clock.prompt = "PRIVATE_CLOCK_SENTINEL {time}".into();
    assert!(!registry
        .system_prompt(&prompt_cfg)
        .contains("PRIVATE_CLOCK_SENTINEL"));
    assert!(registry
        .authorize("fixture.echo", &json!({"text":"denied"}), &cfg)
        .is_err());
    // Safe extraction rejects links rather than following an escaping target.
    let archive = temp.path().join("unsafe.tar");
    {
        let mut tar = tar::Builder::new(std::fs::File::create(&archive).unwrap());
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_link_name("/tmp/escape").unwrap();
        header.set_cksum();
        tar.append_data(&mut header, "link", std::io::empty())
            .unwrap();
        tar.finish().unwrap();
    }
    assert!(plugins::stage(archive.to_str().unwrap(), None).is_err());
    let mut state = BTreeMap::new();
    state.insert(
        "fixture".into(),
        Saved {
            enabled: false,
            ..Default::default()
        },
    );
    plugins::save(&state).unwrap();
    assert!(plugins::data_dir("fixture").exists());
    // The bundled desktop packages must discover exactly their declared schemas,
    // expose voice routes, and preserve permissions through normal host dispatch.
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ids = [
        "routines",
        "projects",
        "capture",
        "calendar",
        "file-search",
        "media",
        "power",
        "history",
        "notes",
    ];
    for id in ids {
        let source = checkout.join("abilities").join(id);
        let target = plugins::package_dir().join(id);
        std::fs::create_dir_all(&target).unwrap();
        for file in std::fs::read_dir(&source).unwrap() {
            let file = file.unwrap();
            if file.file_type().unwrap().is_file() {
                std::fs::copy(file.path(), target.join(file.file_name())).unwrap();
            }
        }
        Manifest::read(&target).unwrap();
        state.insert(
            id.into(),
            Saved {
                enabled: true,
                trusted: true,
                source: "bundled".into(),
                ..Default::default()
            },
        );
    }
    plugins::save(&state).unwrap();
    let registry = Registry::load(&cfg).await;
    assert!(registry.errors.is_empty(), "{:?}", registry.errors);
    for id in ids {
        assert!(
            registry
                .plugins
                .as_ref()
                .unwrap()
                .inspect()
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["id"] == id && p["health"] == "running"),
            "{id}"
        );
    }
    assert_eq!(registry.policy("capture.screen", &cfg), "confirm");
    assert_eq!(registry.policy("calendar.create", &cfg), "confirm");
    assert_eq!(registry.policy("routines.run", &cfg), "confirm");
    assert_eq!(registry.policy("history.undo", &cfg), "allow");
    assert_eq!(registry.policy("notes.undo", &cfg), "confirm");
    assert!(registry
        .call_allowed(
            "capture.clipboard",
            json!({}),
            &cfg,
            &CancellationToken::new()
        )
        .await
        .is_err());
    assert_eq!(
        registry
            .plugins
            .as_ref()
            .unwrap()
            .route("pause music", &CancellationToken::new())
            .await
            .unwrap()
            .unwrap()["arguments"]["action"],
        "pause"
    );
    assert_eq!(
        registry
            .plugins
            .as_ref()
            .unwrap()
            .route("use project demo", &CancellationToken::new())
            .await
            .unwrap()
            .unwrap()["arguments"]["name"],
        "demo"
    );
    assert_eq!(
        registry
            .plugins
            .as_ref()
            .unwrap()
            .route("undo that", &CancellationToken::new())
            .await
            .unwrap()
            .unwrap()["tool"],
        "history.undo"
    );
    let note = registry
        .call_allowed(
            "notes.create",
            json!({"text":"undo fixture"}),
            &cfg,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(note["structuredContent"]["undo"]["tool"], "notes.undo");
    assert!(registry
        .call_allowed(
            "notes.undo",
            note["structuredContent"]["undo"]["arguments"].clone(),
            &cfg,
            &CancellationToken::new()
        )
        .await
        .is_err());
    let undo = note["structuredContent"]["undo"]["arguments"].clone();
    let grant = Grant::approved(&registry.verdict("notes.undo", &cfg), &undo, "test").unwrap();
    registry
        .call("notes.undo", undo, &cfg, &CancellationToken::new(), &grant)
        .await
        .unwrap();
    for id in ids {
        registry
            .plugins
            .as_ref()
            .unwrap()
            .change("disable", id, &cfg)
            .await
            .unwrap();
    }
}
