use artur::load_config;
use std::io::Write;
use tempfile::NamedTempFile;

#[tokio::test]
async fn loads_config_from_file() {
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(
        r#"
version = 1

[[endpoints]]
name = "hello"
method = "GET"
path = "/hello"
action = "respond.static"

[endpoints.response]
body = { ok = true }
"#
        .as_bytes(),
    )
    .unwrap();
    let cfg = load_config(file.path().to_str().unwrap()).await.unwrap();
    assert_eq!(cfg.version, 1);
    assert_eq!(cfg.server.port, 46796);
    assert_eq!(cfg.endpoints[0].name, "hello");
}
