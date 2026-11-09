use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[test]
fn upgrade_does_not_replace_a_cargo_install_with_an_apt_copy() {
    let dir = tempfile::tempdir().unwrap();
    let cli = dir.path().join("funget");
    std::fs::write(&cli, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_fundeploy"))
        .args(["tool", "funget", "upgrade"])
        .env("PATH", dir.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("请用原安装渠道升级"));
}

#[test]
fn upgrade_keeps_the_installed_homebrew_channel() {
    let dir = tempfile::tempdir().unwrap();
    for (name, script) in [
        (
            "brew",
            "#!/bin/sh\nif [ \"$1\" = list ]; then echo 'funget 1.2.0'; else echo \"brew $*\" >> \"$TOOL_CALLS\"; fi\n",
        ),
        ("apt-get", "#!/bin/sh\necho \"apt $*\" >> \"$TOOL_CALLS\"\nexit 1\n"),
    ] {
        let path = dir.path().join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let calls = dir.path().join("calls");
    let output = Command::new(env!("CARGO_BIN_EXE_fundeploy"))
        .args(["tool", "funget", "upgrade"])
        .env("PATH", dir.path())
        .env("TOOL_CALLS", &calls)
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        std::fs::read_to_string(calls).unwrap(),
        "brew install funget\n"
    );
}
