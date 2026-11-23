#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[test]
fn farfarfun_website_uses_the_scoped_package_and_registry() {
    for tool in ["npm", "pnpm"] {
        for action in ["install", "upgrade", "rollback", "missing-registry"] {
            let dir = tempfile::tempdir().unwrap();
            let log = dir.path().join("calls");
            let bin = dir.path().join(tool);
            std::fs::write(
                &bin,
                r#"#!/bin/sh
printf '%s\n' "$*" >> "$FUNDEPLOY_TEST_NPM_LOG"
case "$1" in
    config)
        [ "$2 $3" = 'get @farfarfun:registry' ] || exit 90
        printf '%s\n' "$FUNDEPLOY_TEST_NPM_REGISTRY"
        ;;
    ls) printf '{}\n' ;;
    view)
        [ "$4 $5" = '--registry https://private.example/npm/' ] || exit 91
        printf '0.1.9\n'
        ;;
    install|add)
        [ "$4 $5" = '--registry https://private.example/npm/' ] || exit 92
        ;;
    *) exit 93 ;;
esac
"#,
            )
            .unwrap();
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            drop(listener);

            let mut command = Command::new(env!("CARGO_BIN_EXE_fundeploy"));
            command.args(["service", "farfarfun-web"]);
            match action {
                "rollback" => command.args(["rollback", "0.1.8"]),
                "missing-registry" => command.arg("install"),
                _ => command.arg(action),
            };
            let output = command
                .env("PATH", dir.path())
                .env_remove("npm_config_registry")
                .env_remove("NPM_CONFIG_REGISTRY")
                .env_remove("FARFARFUN_WEB_FRONTEND_VERSION")
                .env_remove("FARFARFUN_WEB_FRONTEND_PACKAGE")
                .env("FARFARFUN_WEB_FRONTEND_HOST", "127.0.0.1")
                .env("FARFARFUN_WEB_FRONTEND_PORT", port.to_string())
                .env("FUNDEPLOY_TEST_NPM_LOG", &log)
                .env(
                    "FUNDEPLOY_TEST_NPM_REGISTRY",
                    if action == "missing-registry" {
                        "undefined"
                    } else {
                        "https://private.example/npm/"
                    },
                )
                .output()
                .unwrap();
            let calls = std::fs::read_to_string(log).unwrap();
            let install = if tool == "pnpm" { "add" } else { "install" };
            if action == "missing-registry" {
                assert!(!output.status.success());
                assert!(String::from_utf8_lossy(&output.stderr).contains("@farfarfun:registry"));
                assert!(!calls.contains(&format!("{install} -g")), "{calls}");
                continue;
            }
            assert!(output.status.success(), "{tool} {action}: {output:?}");
            let version = if action == "rollback" {
                "0.1.8"
            } else {
                "0.1.9"
            };
            let query = if action == "rollback" {
                "@farfarfun/farfarfun-web@0.1.8"
            } else {
                "@farfarfun/farfarfun-web"
            };
            assert!(
                calls.contains(&format!(
                    "view {query} version --registry https://private.example/npm/"
                )),
                "{calls}"
            );
            assert!(
                calls.contains(&format!(
                    "{install} -g @farfarfun/farfarfun-web@{version} --registry https://private.example/npm/"
                )),
                "{calls}"
            );
            assert!(!calls.contains("config set"), "{calls}");
        }
    }
}
