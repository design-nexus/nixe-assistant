//! The worker refuses calls the host didn't grant, before running anything.
use assistant_core::Config;
use assistant_policy::{decide, AutonomyMode, Context, Grant, Rules};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

fn call(worker: &mut std::process::Child, id: i64, params: Value) -> Value {
    let request = json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":params});
    let stdin = worker.stdin.as_mut().unwrap();
    writeln!(stdin, "{request}").unwrap();
    let mut line = String::new();
    BufReader::new(worker.stdout.as_mut().unwrap()).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

#[test]
fn calls_need_the_hosts_grant() {
    let cfg = serde_json::to_string(&Config::default()).unwrap();
    let mut worker = Command::new(env!("CARGO_BIN_EXE_nixe-assistant-plugin-worker"))
        .arg("system-status")
        .env("ASSISTANT_CONFIG", cfg)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let args = json!({});
    let none = call(&mut worker, 1, json!({"name":"system.status","arguments":args}));
    assert!(none["error"]["message"].as_str().unwrap().contains("no grant"), "{none}");

    let rules = Rules::default();
    let verdict = decide(
        "system.status",
        &Context { available: true, trusted: true, tool_rule: None, rules: &rules, mode: AutonomyMode::Autonomous },
    );
    let grant = Grant::allowed(&verdict, &args).unwrap();
    grant.check("system.status", &args).unwrap();
    let meta = grant.attest("system.status", &json!({"other":1})).unwrap();
    let wrong = call(&mut worker, 2, json!({"name":"system.status","arguments":args,"_meta":meta}));
    assert!(wrong["error"]["message"].as_str().unwrap().contains("arguments changed"), "{wrong}");

    let meta = grant.attest("system.status", &args).unwrap();
    let ok = call(&mut worker, 3, json!({"name":"system.status","arguments":args,"_meta":meta}));
    assert_eq!(ok["result"]["isError"], false, "{ok}");
    worker.kill().unwrap();
}
