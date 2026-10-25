use std::os::unix::fs::PermissionsExt;
use std::process::Command;

use fundeploy::common::pip::{Mode, PipPkg};

#[test]
fn install_uses_uv_pip() {
    // 子进程独占 PATH，避免影响并行测试或调用真实包管理器。
    if let Ok(case) = std::env::var("FUNDEPLOY_TEST_PIP_CASE") {
        let pkg = PipPkg::new("demo-api", "demo-api");
        let version = (case == "pinned").then_some("v1.2.3");
        let result = pkg.install(version);
        if case == "failure" {
            assert!(result.is_err());
        } else {
            result.unwrap();
        }
        if case == "new" {
            assert_eq!(pkg.mode(), Mode::Pip);
        } else if case == "existing" {
            assert_eq!(pkg.managed_version().as_deref(), Some("3.0.0"));
            let warning = pkg.shadow_warning().unwrap();
            assert!(warning.contains("2.0.0") && warning.contains("3.0.0"));
        }
        return;
    }

    for case in ["new", "existing", "pinned", "failure"] {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("calls");
        let uv = dir.path().join("uv");
        std::fs::write(
            &uv,
            r#"#!/bin/sh
if [ "$1 $2" = 'tool list' ]; then
    [ "$FUNDEPLOY_TEST_PIP_CASE" != new ] || exit 0
    printf 'demo-api v2.0.0\n- demo-api\n'
    exit 0
fi
printf '%s\n' "$@" >> "$FUNDEPLOY_TEST_PIP_LOG"
[ "$1 $2" = 'pip install' ] || exit 90
[ "$FUNDEPLOY_TEST_PIP_CASE" != failure ] || exit 1
"#,
        )
        .unwrap();
        std::fs::set_permissions(&uv, std::fs::Permissions::from_mode(0o755)).unwrap();
        let python = dir.path().join("old-tool/bin/python");
        if case != "new" {
            std::fs::create_dir_all(python.parent().unwrap()).unwrap();
            std::fs::write(&python, "#!/bin/sh\nprintf '3.0.0\\n'\n").unwrap();
            std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o755)).unwrap();
            let cli = dir.path().join("demo-api");
            std::fs::write(&cli, format!("#!{}\n", python.display())).unwrap();
            std::fs::set_permissions(cli, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "install_uses_uv_pip", "--nocapture"])
            .env("PATH", dir.path())
            .env("FUNDEPLOY_TEST_PIP_CASE", case)
            .env("FUNDEPLOY_TEST_PIP_LOG", &log)
            .output()
            .unwrap();
        assert!(output.status.success(), "{case}: {output:?}");

        let mut expected = if case == "pinned" {
            "pip\ninstall\ndemo-api==1.2.3\n".to_string()
        } else {
            "pip\ninstall\ndemo-api\n".to_string()
        };
        if case != "new" {
            expected.push_str(&format!("--python\n{}\n", python.display()));
        }
        if case != "pinned" {
            expected.push_str("-U\n");
        }
        assert_eq!(std::fs::read_to_string(log).unwrap(), expected, "{case}");
    }
}
