use std::process::Command;

#[test]
fn redirected_monitor_prints_one_snapshot() {
    let output = Command::new(env!("CARGO_BIN_EXE_fundeploy"))
        .arg("monitor")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let panel = String::from_utf8(output.stdout).unwrap();
    for field in ["物理核", "逻辑核", "当前利用率", "内存", "磁盘", "GPU"] {
        assert!(panel.contains(field), "{panel}");
    }
    assert_eq!(panel.matches("本机状态").count(), 1);
    assert!(!panel.contains('\x1b'));
}
