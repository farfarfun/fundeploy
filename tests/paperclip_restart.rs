use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[test]
fn restart_stops_before_starting_and_checks_health() {
    for initial_pid in ["null", "12345"] {
        for stop_code in [0, 3] {
            let dir = tempfile::tempdir().unwrap();
            let bin = dir.path().join(".local/bin/paperclipai");
            std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
            std::fs::write(
                &bin,
                r#"#!/bin/sh
printf '%s\n' "$*" >> "$HOME/calls"
case "$*" in
    'service status --json')
        if [ -f "$HOME/stopped" ]; then
            printf '{"supported":true,"pid":null}\n'
        else
            printf '{"supported":true,"pid":%s,"detail":"loaded"}\n' "$TEST_PID"
        fi
        ;;
    'service stop') : > "$HOME/stopped"; exit "$TEST_STOP_CODE" ;;
    'service start') exit 0 ;;
    '--version') printf '2026.1.0\n' ;;
    *) exit 99 ;;
esac
"#,
            )
            .unwrap();
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            drop(listener);
            let home = dir.path().join("paperclip");
            let instance = home.join("instances/default");
            std::fs::create_dir_all(&instance).unwrap();
            std::fs::write(
                instance.join("config.json"),
                format!(r#"{{"server":{{"host":"127.0.0.1","port":{port}}}}}"#),
            )
            .unwrap();
            let output = Command::new(env!("CARGO_BIN_EXE_fundeploy"))
                .args(["service", "paperclip", "restart"])
                .env("HOME", dir.path())
                .env("PAPERCLIP_HOME", home)
                .env("PAPERCLIP_INSTANCE_ID", "default")
                .env("PAPERCLIP_START_HEALTH_TIMEOUT_SEC", "0")
                .env("TEST_PID", initial_pid)
                .env("TEST_STOP_CODE", stop_code.to_string())
                .output()
                .unwrap();
            let calls = std::fs::read_to_string(dir.path().join("calls")).unwrap();
            let stop = calls.find("service stop\n").unwrap();
            let start = calls.find("service start\n").unwrap();
            assert!(stop < start, "{calls}");
            assert!(!calls.contains("service restart"), "{calls}");
            // 上游 start 成功但端口没起来，不能把这次重启报成成功。
            assert!(!output.status.success(), "{output:?}");
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("健康检查"),
                "{output:?}"
            );
        }
    }
}
