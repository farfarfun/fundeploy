//! Casdoor 的原生发行包和持久化配置。

use anyhow::{bail, Context, Result};
use std::fs::File;
use std::io::Read;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::PathBuf;

use super::config;
use super::daemon::{Daemon, DaemonSpec, Github, Proc};
use crate::common::env::{home, str_or};
use crate::common::ini::Ini;
use crate::common::platform::{Arch, Os};
use crate::ui;

const PORT: u16 = 8805;

pub struct Casdoor {
    spec: DaemonSpec,
}

pub fn from_env() -> Casdoor {
    let home = PathBuf::from(str_or(
        "CASDOOR_SERVICE_HOME",
        &home().join(".farfarfun/casdoor").to_string_lossy(),
    ));
    Casdoor::at(home)
}

impl Casdoor {
    fn at(home: PathBuf) -> Self {
        let config = home.join("data/app.conf");
        let proc = Proc::new(
            "main",
            "Casdoor",
            "casdoor",
            "0.0.0.0",
            PORT,
            home.join("app"),
        )
        .args(vec![
            "--config".into(),
            config.to_string_lossy().into_owned(),
        ]);
        let source = Github {
            repo: "casdoor/casdoor".into(),
            asset: Box::new(|_, p| {
                let os = match p.os {
                    Os::Linux => "Linux",
                    Os::MacOs => "Darwin",
                };
                let arch = match p.arch {
                    Arch::X86_64 => "x86_64",
                    Arch::Aarch64 => "arm64",
                };
                format!("casdoor_{os}_{arch}.tar.gz")
            }),
            strip: 0,
            binary_name: "casdoor".into(),
            scan: 20,
        };
        let spec = DaemonSpec::new("casdoor", "Casdoor SSO", home, source, vec![proc])
            .entry(format!("http://127.0.0.1:{PORT}"))
            .note("Casdoor 绑定所有网卡；启动前须在防火墙中阻止公网访问 TCP 8805。")
            .note("公网入口由 Caddy 提供 HTTPS；配置位于 data/app.conf。");
        Self { spec }
    }

    fn config_path(&self) -> PathBuf {
        self.spec.data_dir().join("app.conf")
    }

    fn database_uri(&self) -> String {
        format!(
            "file:{}?cache=shared",
            self.spec.data_dir().join("casdoor.db").display()
        )
    }

    fn prepare(&self) -> Result<()> {
        if !self.spec.home.is_absolute() {
            bail!("CASDOOR_SERVICE_HOME 必须是绝对路径");
        }
        let data = self.spec.data_dir();
        config::ensure_private_dir(&data)?;
        let files = data.join("files");
        config::ensure_private_dir(&files)?;

        let link = self.spec.app_dir().join("files");
        match std::fs::symlink_metadata(&link) {
            Ok(meta) if meta.file_type().is_symlink() && std::fs::read_link(&link)? == files => {}
            Ok(_) => bail!("{} 已存在且不是 Casdoor 的持久化文件目录", link.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                symlink(&files, &link).with_context(|| format!("创建 {} 失败", link.display()))?;
            }
            Err(e) => return Err(e).with_context(|| format!("检查 {} 失败", link.display())),
        }

        let path = self.config_path();
        if !path.exists() {
            let mut random = [0u8; 32];
            File::open("/dev/urandom")
                .context("读取系统随机数失败")?
                .read_exact(&mut random)
                .context("生成管理员初始密码失败")?;
            let body = format!(
                "appname = casdoor\n\
                 httpport = {PORT}\n\
                 runmode = prod\n\
                 driverName = sqlite\n\
                 dataSourceName = {}\n\
                 dbName = casdoor\n\
                 origin = https://id.example.com\n\
                 initAdminPassword = {}\n\
                 verificationCodeTimeout = 5\n\
                 sessionCookieLifeTime = 28800\n\
                 inactiveTimeoutMinutes = 30\n\
                 defaultLanguage = zh\n\
                 ldapServerPort = 0\n\
                 ldapsServerPort = 0\n\
                 radiusServerPort = -1\n\
                 disableUpgrade = true\n\
                 logConfig = {{\"adapter\":\"console\"}}\n",
                self.database_uri(),
                hex::encode(random),
            );
            if config::write_if_absent(&path, &body)? {
                ui::step(format!("已生成 {}（先填真实 HTTPS 域名）", path.display()));
            }
        }
        Ok(())
    }

    fn validate_config(&self) -> Result<()> {
        let path = self.config_path();
        let mode = std::fs::metadata(&path)
            .with_context(|| format!("检查 {} 失败", path.display()))?
            .permissions()
            .mode()
            & 0o777;
        if mode != 0o600 {
            bail!("{} 的权限必须是 0600", path.display());
        }

        let cfg = Ini::load(&path)?;
        for (key, expected) in [
            ("httpport", PORT.to_string()),
            ("runmode", "prod".into()),
            ("ldapServerPort", "0".into()),
            ("ldapsServerPort", "0".into()),
            ("radiusServerPort", "-1".into()),
            ("disableUpgrade", "true".into()),
        ] {
            if cfg.get_unique(None, key) != Some(expected.as_str()) {
                bail!("{} 中的 {key} 必须唯一且等于 {expected}", path.display());
            }
        }

        match cfg.get_unique(None, "driverName") {
            Some("sqlite")
                if cfg.get_unique(None, "dataSourceName") == Some(&self.database_uri()) => {}
            Some("postgres")
                if cfg
                    .get_unique(None, "dataSourceName")
                    .is_some_and(|v| !v.is_empty())
                    && cfg
                        .get_unique(None, "dbName")
                        .is_some_and(|v| !v.is_empty()) => {}
            _ => bail!(
                "{} 中的数据库配置无效；仅支持指定路径的 SQLite 或 PostgreSQL",
                path.display()
            ),
        }

        let origin = cfg.get_unique(None, "origin").unwrap_or("");
        let host = origin.strip_prefix("https://").unwrap_or("");
        if host == "id.example.com"
            || !host.contains('.')
            || !host.split('.').all(|label| {
                !label.is_empty()
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
        {
            bail!("{} 中的 origin 必须是真实 HTTPS 域名", path.display());
        }
        let password = cfg.get_unique(None, "initAdminPassword").unwrap_or("");
        if password.len() != 64
            || !password
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            bail!(
                "{} 中的 initAdminPassword 必须是 64 位小写十六进制数",
                path.display()
            );
        }
        Ok(())
    }
}

impl Daemon for Casdoor {
    fn spec(&self) -> &DaemonSpec {
        &self.spec
    }

    fn require_checksum(&self) -> bool {
        true
    }

    fn post_install(&self) -> Result<()> {
        if !self.spec.app_dir().join("web/build").is_dir() {
            bail!("Casdoor 发行包缺少 web/build，无法提供登录页面");
        }
        self.prepare()
    }

    fn ensure_config(&self) -> Result<()> {
        self.prepare()?;
        self.validate_config()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::platform::Platform;

    #[test]
    fn 官方附件与进程声明() {
        let s = Casdoor::at(PathBuf::from("/tmp/casdoor-test"));
        assert_eq!(s.spec.procs[0].port, 8805);
        assert_eq!(s.spec.procs[0].host, "0.0.0.0");
        assert_eq!(s.spec.procs[0].args[0], "--config");
        assert!(s.require_checksum());
        assert_eq!(
            (s.spec.source.asset)(
                "v4.25.0",
                Platform {
                    os: Os::Linux,
                    arch: Arch::X86_64
                }
            ),
            "casdoor_Linux_x86_64.tar.gz"
        );
        assert_eq!(
            (s.spec.source.asset)(
                "v4.25.0",
                Platform {
                    os: Os::MacOs,
                    arch: Arch::Aarch64
                }
            ),
            "casdoor_Darwin_arm64.tar.gz"
        );
    }

    #[test]
    fn 配置与文件跨升级保留且启动前校验() {
        let tmp = tempfile::tempdir().unwrap();
        let s = Casdoor::at(tmp.path().join("casdoor"));
        std::fs::create_dir_all(s.spec.app_dir().join("web/build")).unwrap();
        s.post_install().unwrap();

        let path = s.config_path();
        let config = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(s.spec.data_dir())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::read_link(s.spec.app_dir().join("files")).unwrap(),
            s.spec.data_dir().join("files")
        );
        assert!(s.ensure_config().is_err(), "示例域名不能启动");

        let mut ini = Ini::load(&path).unwrap();
        ini.set(None, "origin", "https://id.farfarfun.example");
        ini.save(&path).unwrap();
        s.ensure_config().unwrap();
        let edited = std::fs::read_to_string(&path).unwrap();
        s.post_install().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
        assert!(config.contains("initAdminPassword = "));

        ini.set(None, "httpport", "8000");
        ini.save(&path).unwrap();
        assert!(s.ensure_config().is_err(), "旧端口不能启动");

        ini.set(None, "httpport", "8805");
        ini.set(None, "disableUpgrade", "false");
        ini.save(&path).unwrap();
        assert!(s.ensure_config().is_err(), "一键自升级不能绕过 fundeploy");
    }

    #[test]
    fn 接受_postgresql_并拒绝无效数据库配置() {
        let tmp = tempfile::tempdir().unwrap();
        let s = Casdoor::at(tmp.path().join("casdoor"));
        std::fs::create_dir_all(s.spec.app_dir()).unwrap();
        s.prepare().unwrap();
        let path = s.config_path();
        let mut ini = Ini::load(&path).unwrap();
        ini.set(None, "origin", "https://id.farfarfun.example");
        ini.set(None, "driverName", "postgres");
        ini.set(
            None,
            "dataSourceName",
            "user=casdoor password=secret host=127.0.0.1 port=5432 sslmode=disable dbname=casdoor",
        );
        ini.save(&path).unwrap();
        s.validate_config().unwrap();

        ini.set(None, "dataSourceName", "");
        ini.save(&path).unwrap();
        assert!(s.validate_config().is_err());
        ini.set(None, "dataSourceName", "user=casdoor dbname=casdoor");
        ini.set(None, "dbName", "");
        ini.save(&path).unwrap();
        assert!(s.validate_config().is_err());
        ini.set(None, "dataSourceName", "file:other.db");
        ini.set(None, "driverName", "sqlite");
        ini.save(&path).unwrap();
        assert!(s.validate_config().is_err());
    }

    #[test]
    fn 相对服务目录会在创建数据前被拒绝() {
        let s = Casdoor::at(PathBuf::from("relative-casdoor"));
        assert!(s.prepare().is_err());
        assert!(!s.spec.home.exists());
    }
}
