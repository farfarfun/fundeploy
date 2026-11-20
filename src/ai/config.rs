//! 交互配置与模型发现。

use std::io::{ErrorKind, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::{fs::PermissionsExt, process::CommandExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{anyhow, bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use toml_edit::{value, DocumentMut, Item, Table};
use url::Url;

use super::Ai;
use crate::common::{cmd, env};
use crate::ui;

const TIMEOUT: Duration = Duration::from_secs(10);
const MANUAL_MODEL: &str = "手动输入模型 ID";

#[derive(Serialize, Deserialize)]
struct Config {
    base_url: String,
    api_key: String,
    model: String,
}

pub fn configure(tool: &Ai) -> Result<()> {
    ensure!(ui::interactive(), "请在交互终端中配置 AI 工具");
    let default_url = match tool.name {
        "codex" => "https://api.openai.com/v1",
        "claude" => "https://api.anthropic.com",
        "cursor" => "https://api2.cursor.sh",
        "antigravity" => "https://generativelanguage.googleapis.com",
        _ => bail!("{} 没有配置入口", tool.name),
    };
    let previous = if matches!(tool.name, "cursor" | "antigravity") {
        read_config(&profile_path(tool), |raw| {
            raw.map(parse_profile).transpose()
        })?
    } else {
        None
    };
    ui::heading(format!("配置 {}", tool.summary));
    let default_url = previous.as_ref().map_or(default_url, |v| &v.base_url);
    let base_url = normalize_url(tool.name, &ui::input("API 地址（base_url）", default_url)?)?;
    let api_key = ui::password("API 密钥（api_key）")?.trim().to_owned();
    ensure!(
        !api_key.is_empty() && api_key.bytes().all(|b| b.is_ascii_graphic()),
        "API 密钥须为非空且不含空白的 ASCII 字符串"
    );

    ui::step("拉取模型列表");
    let mut claude_key = if base_url == "https://api.anthropic.com" {
        "x-api-key"
    } else {
        "Authorization"
    };
    let models = if tool.name == "cursor" {
        cursor_models(tool, &base_url, &api_key)
    } else {
        let http = ureq::AgentBuilder::new()
            .timeout(TIMEOUT)
            .redirects(0)
            .build();
        let models = http_models(&http, tool.name, &base_url, &api_key, claude_key);
        if tool.name == "claude" && models.is_err() {
            let other = if claude_key == "Authorization" {
                "x-api-key"
            } else {
                "Authorization"
            };
            match http_models(&http, tool.name, &base_url, &api_key, other) {
                Ok(models) => {
                    claude_key = other;
                    Ok(models)
                }
                Err(_) => models,
            }
        } else {
            models
        }
    };
    let mut options = match models {
        Ok(models) => models,
        Err(error) => {
            ui::warn(format!("拉取模型列表失败（{error}）"));
            Vec::new()
        }
    };
    let selected = if options.is_empty() {
        MANUAL_MODEL.to_owned()
    } else {
        options.push(MANUAL_MODEL.to_owned());
        ui::select("选择默认模型", &options)?
            .ok_or(inquire::InquireError::OperationCanceled)?
            .clone()
    };
    let model = if selected == MANUAL_MODEL {
        let default = previous.as_ref().map_or("", |v| &v.model);
        ui::input("默认模型 ID", default)?
    } else {
        selected
    };
    ensure!(valid_model(&model), "模型 ID 无效");
    let config = Config {
        base_url,
        api_key,
        model,
    };

    let path = match tool.name {
        "codex" => {
            let path = config_dir("CODEX_HOME", ".codex").join("config.toml");
            edit_config(&path, |raw| codex_config(raw.unwrap_or_default(), &config))?;
            path
        }
        "claude" => {
            let path = config_dir("CLAUDE_CONFIG_DIR", ".claude").join("settings.json");
            edit_config(&path, |raw| claude_config(raw, &config, claude_key))?;
            path
        }
        _ => {
            let path = profile_path(tool);
            edit_config(&path, |_| Ok(serde_json::to_string_pretty(&config)?))?;
            ui::hint(format!("启动命令为 fundeploy ai {} run", tool.name));
            path
        }
    };
    ui::ok(format!("已保存模型 {} 至 {}", config.model, path.display()));
    Ok(())
}

fn normalize_url(tool: &str, raw: &str) -> Result<String> {
    let mut url = Url::parse(raw).map_err(|_| anyhow!("API 地址无效"))?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.host().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && !raw.chars().any(char::is_whitespace),
        "请输入不带用户名、密码、查询参数或片段的 HTTP(S) API 地址"
    );
    let path = url.path().trim_end_matches('/');
    let path = match tool {
        "codex" if path.is_empty() => "/v1",
        "claude" => path.strip_suffix("/v1").unwrap_or(path),
        "antigravity" => path.strip_suffix("/v1beta").unwrap_or(path),
        _ => path,
    }
    .to_owned();
    url.set_path(&path);
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

fn valid_model(id: &str) -> bool {
    !id.is_empty() && !id.starts_with('-') && !id.chars().any(char::is_control)
}

fn http_models(
    http: &ureq::Agent,
    tool: &str,
    base: &str,
    key: &str,
    claude_key: &str,
) -> Result<Vec<String>> {
    let (path, header) = match tool {
        "claude" => ("/v1/models", claude_key),
        "antigravity" => ("/v1beta/models", "x-goog-api-key"),
        _ => ("/models", "Authorization"),
    };
    let auth = if header == "Authorization" {
        format!("Bearer {key}")
    } else {
        key.to_owned()
    };
    let url = format!("{base}{path}");
    let mut cursor: Option<(&str, String)> = None;
    let mut ids = Vec::new();
    // shortcut: 最多读取十页，超长模型目录需要扩大上限。
    for _ in 0..10 {
        let mut req = http.get(&url).set(header, &auth);
        if tool == "claude" {
            req = req.set("anthropic-version", "2023-06-01");
        }
        if let Some((parameter, token)) = &cursor {
            req = req.query(parameter, token);
        }
        let response = req.call().map_err(|error| match error {
            ureq::Error::Status(code, _) => anyhow!("HTTP {code}"),
            ureq::Error::Transport(_) => anyhow!("无法连接到模型接口"),
        })?;
        ensure!(response.status() == 200, "HTTP {}", response.status());
        let page: Value = serde_json::from_reader(response.into_reader().take(4 * 1024 * 1024))
            .map_err(|_| anyhow!("模型接口返回了无效 JSON"))?;
        ids.extend(model_ids(tool, &page));
        let next = if tool == "antigravity" {
            page["nextPageToken"].as_str().map(|s| ("pageToken", s))
        } else if tool == "claude" && page["has_more"].as_bool() == Some(true) {
            page["last_id"].as_str().map(|s| ("after_id", s))
        } else {
            None
        };
        let Some((parameter, token)) = next.filter(|(_, token)| !token.is_empty()) else {
            break;
        };
        ensure!(
            cursor.as_ref().is_none_or(|(_, last)| last != token),
            "模型接口返回了重复的分页标记"
        );
        cursor = Some((parameter, token.to_owned()));
    }
    ids.sort_unstable();
    ids.dedup();
    ensure!(!ids.is_empty(), "接口没有返回可选模型");
    Ok(ids)
}

fn model_ids(tool: &str, page: &Value) -> Vec<String> {
    let gemini = tool == "antigravity";
    let entries = if gemini {
        &page["models"]
    } else {
        &page["data"]
    };
    entries
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            if !gemini {
                return row["id"].as_str();
            }
            if row["supportedGenerationMethods"]
                .as_array()
                .is_some_and(|methods| !methods.iter().any(|method| method == "generateContent"))
            {
                return None;
            }
            row["name"].as_str()?.strip_prefix("models/")
        })
        .filter(|id| valid_model(id))
        .map(str::to_owned)
        .collect()
}

fn cursor_models(tool: &Ai, base: &str, key: &str) -> Result<Vec<String>> {
    let bin = bin_path(tool)?;
    let edits = [
        ("CURSOR_API_ENDPOINT".into(), Some(base.to_owned())),
        ("CURSOR_API_KEY".into(), Some(key.to_owned())),
        ("CURSOR_AUTH_TOKEN".into(), None),
        ("NO_COLOR".into(), Some("1".into())),
    ];
    let output = cmd::try_run_timeout_with([bin.as_str(), "models"], &edits, TIMEOUT)?
        .context("agent models 超时")?;
    ensure!(
        output.success(),
        "agent models 失败，请检查 Cursor 端点和密钥"
    );
    let mut models: Vec<_> = output.stdout.lines().filter_map(cursor_model_id).collect();
    models.sort_unstable();
    models.dedup();
    ensure!(!models.is_empty(), "agent models 没有返回可选模型");
    Ok(models)
}

fn cursor_model_id(line: &str) -> Option<String> {
    let line = line.trim().trim_start_matches("- ").trim_start();
    let id = line
        .split_once(" - ")
        .or_else(|| line.split_once('\t'))
        .map_or(line, |(id, _)| id)
        .trim();
    (!id.is_empty()
        && !id.starts_with('-')
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_./:[],=".contains(&b)))
    .then(|| id.to_owned())
}

pub fn run(tool: &Ai, args: &[String]) -> Result<()> {
    ensure!(
        matches!(tool.name, "cursor" | "antigravity"),
        "请直接运行 {}",
        tool.bins[0]
    );
    let config = read_config(&profile_path(tool), |raw| {
        parse_profile(raw.with_context(|| format!("请先运行 fundeploy ai {} config", tool.name))?)
    })?;
    let mut command = Command::new(bin_path(tool)?);
    if tool.name == "cursor" {
        command
            .env("CURSOR_API_ENDPOINT", &config.base_url)
            .env("CURSOR_API_KEY", &config.api_key)
            .env_remove("CURSOR_AUTH_TOKEN")
            .args(["--model", &config.model]);
    } else {
        // agy 只从进程环境取密钥，模型 ID 则通过会话配置传入。
        let settings = json!({
            "modelProvider": "gemini",
            "customModelsConfig": { "customModels": {
                "fundeploy": {
                    "label": config.model,
                    "modelName": format!("gemini-api://models/{}", config.model)
                }
            }}
        });
        command
            .env("GEMINI_API_KEY", &config.api_key)
            .env("GOOGLE_GEMINI_BASE_URL", &config.base_url)
            .args(["--settings", &settings.to_string(), "--model", "fundeploy"]);
    }
    Err(command.args(args).exec()).with_context(|| format!("启动 {} 失败", tool.name))
}

fn bin_path(tool: &Ai) -> Result<String> {
    tool.located()
        .map(|(_, path)| path)
        .with_context(|| format!("找不到 {}，请先安装并加入 PATH", tool.bins[0]))
}

fn config_dir(var: &str, default: &str) -> PathBuf {
    env::opt(var).map_or_else(|| env::home().join(default), |p| env::expand_tilde(&p))
}

fn profile_path(tool: &Ai) -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| env::home().join(".config"))
        .join("fundeploy/ai")
        .join(format!("{}.json", tool.name))
}

fn parse_profile(raw: &str) -> Result<Config> {
    serde_json::from_str(raw).map_err(|_| anyhow!("fundeploy AI 配置格式有误"))
}

fn codex_config(raw: &str, config: &Config) -> Result<String> {
    let mut doc = raw
        .parse::<DocumentMut>()
        .map_err(|_| anyhow!("Codex TOML 配置格式有误"))?;
    let providers = doc
        .entry("model_providers")
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_like_mut()
        .context("Codex 的 model_providers 必须是表")?;
    let mut provider = Table::new();
    provider.insert("name", value("fundeploy"));
    provider.insert("base_url", value(&config.base_url));
    provider.insert("wire_api", value("responses"));
    provider.insert("experimental_bearer_token", value(&config.api_key));
    providers.insert("fundeploy", Item::Table(provider));
    doc["model_provider"] = value("fundeploy");
    doc["model"] = value(&config.model);
    Ok(doc.to_string())
}

fn claude_config(raw: Option<&str>, config: &Config, auth_header: &str) -> Result<String> {
    let mut settings: Map<String, Value> = match raw {
        Some(raw) => {
            serde_json::from_str(raw).map_err(|_| anyhow!("Claude Code JSON 配置格式有误"))?
        }
        None => Map::new(),
    };
    let vars = settings
        .entry("env")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .context("Claude Code 的 env 必须是 JSON 对象")?;
    let (auth, other) = if auth_header == "x-api-key" {
        ("ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN")
    } else {
        ("ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY")
    };
    vars.remove(other);
    vars.remove("ANTHROPIC_MODEL");
    vars.insert(auth.to_owned(), json!(config.api_key));
    vars.insert("ANTHROPIC_BASE_URL".to_owned(), json!(config.base_url));
    settings.insert("model".to_owned(), json!(config.model));
    Ok(serde_json::to_string_pretty(&settings)?)
}

fn read_config<T>(path: &Path, read: impl FnOnce(Option<&str>) -> Result<T>) -> Result<T> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => Some(raw),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => return Err(e).with_context(|| format!("读不了 {}", path.display())),
    };
    read(raw.as_deref()).with_context(|| format!("读取 {} 失败", path.display()))
}

fn edit_config(path: &Path, edit: impl FnOnce(Option<&str>) -> Result<String>) -> Result<()> {
    let display = path.display();
    let path = if path.is_symlink() {
        path.canonicalize()
            .with_context(|| format!("无法解析 {display}"))?
    } else {
        path.to_path_buf()
    };
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let lock = std::fs::File::open(dir)?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(std::io::Error::last_os_error()).context("无法锁定配置目录");
    }
    let updated = read_config(&path, edit)?;
    let mut staged = tempfile::NamedTempFile::new_in(dir)?;
    staged
        .as_file()
        .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    writeln!(staged, "{}", updated.trim_end())?;
    staged
        .persist(&path)
        .with_context(|| format!("写不了 {display}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config {
            base_url: "https://gateway.example/v1".into(),
            api_key: "test-key".into(),
            model: "test-model".into(),
        }
    }

    #[test]
    fn 配置透过符号链接更新并保留另一家提供方() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target.toml");
        let link = temp.path().join("config.toml");
        std::fs::write(
            &target,
            "# keep comment\nmodel_providers = { old = { name = \"Old\" } }\n",
        )
        .unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        edit_config(&link, |raw| codex_config(raw.unwrap(), &config())).unwrap();
        let saved = std::fs::read_to_string(&target).unwrap();
        let doc = saved.parse::<DocumentMut>().unwrap();
        assert!(link.is_symlink());
        assert!(saved.contains("# keep comment"));
        assert_eq!(doc["model_providers"]["old"]["name"].as_str(), Some("Old"));
        assert_eq!(doc["model"].as_str(), Some("test-model"));
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );

        std::fs::write(&target, "broken = [").unwrap();
        assert!(edit_config(&link, |raw| codex_config(raw.unwrap(), &config())).is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "broken = [");
    }

    #[test]
    fn claude_只更新与网关和模型冲突的设置() {
        let saved = claude_config(
            Some(r#"{"permissions":{"deny":["Bash(test)"]},"env":{"USE_CACHE":"1","ANTHROPIC_AUTH_TOKEN":"old"}}"#),
            &config(),
            "x-api-key",
        )
        .unwrap();
        let settings: Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(settings["permissions"]["deny"], json!(["Bash(test)"]));
        assert_eq!(
            settings["env"],
            json!({
                "USE_CACHE": "1",
                "ANTHROPIC_API_KEY": "test-key",
                "ANTHROPIC_BASE_URL": "https://gateway.example/v1"
            })
        );
        assert_eq!(settings["model"], "test-model");
    }

    #[test]
    fn 模型列表遵循各工具的接口格式() {
        assert_eq!(
            model_ids(
                "antigravity",
                &json!({"models":[
                    {"name":"models/embed","supportedGenerationMethods":["embedContent"]},
                    {"name":"models/gemini","supportedGenerationMethods":["generateContent"]}
                ]})
            ),
            ["gemini"]
        );
        assert_eq!(
            model_ids("codex", &json!({"data":[{"id":"provider/gpt"}]})),
            ["provider/gpt"]
        );
        assert_eq!(
            cursor_model_id("  gpt-5[effort=high] - GPT-5"),
            Some("gpt-5[effort=high]".to_owned())
        );
        assert!(cursor_model_id("No models available for this account.").is_none());
    }
}
