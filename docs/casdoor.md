# Casdoor SSO

Casdoor 提供账号、微信 / 手机 / 邮箱登录和标准 OIDC；Caddy 提供 HTTPS；业务应用
只接 OIDC，并自行决定业务权限。`fundeploy service casdoor` 管理官方原生二进制，
无需 Docker、Java、PostgreSQL；默认单实例使用 SQLite。

## 部署

需要 Linux x86_64 或 arm64、`fundeploy`、原生安装的 Caddy，以及指向本机的
公网域名。以下文件路径以默认服务目录 `~/.farfarfun/casdoor` 为例；若设置了
`CASDOOR_SERVICE_HOME`，请使用对应的绝对路径。

开放 TCP 80/443；**启动 Casdoor 前**用主机防火墙和云安全组阻止公网访问 TCP
8805、UDP 1812。Casdoor 的 HTTP 监听会绑定所有网卡，Caddy 反代到本机端口并不会
自动隐藏它。配置将 LDAP 关闭；Casdoor 没有 RADIUS 关闭开关，所以使用无效端口
阻止其监听，启动日志中的 RADIUS bind 错误是预期的。

不指定版本时安装最新的当前平台发行包；也可以固定到已验证的 v4.25.0。Casdoor
发行包必须有可用的上游 SHA-256 校验和，否则安装会失败。安装时自动生成
`data/app.conf` 和随机的首次管理员密码，数据目录权限为 `0700`，
配置文件权限为 `0600`。安装和升级会替换 `app/`，保留 `data/`；上传文件位于
`data/files/`。

```bash
fundeploy service casdoor install
```

需要固定版本时使用：

```bash
fundeploy service casdoor install --version 4.25.0
```

编辑 `~/.farfarfun/casdoor/data/app.conf`，把 `origin` 改成真实的 `https://` 域名。
`initAdminPassword` 已生成；在新数据库首次启动时用于设置 `built-in/admin` 的
bcrypt 密码。启动时会校验配置权限、真实 HTTPS 域名、管理员密码、数据库路径
和 `8805` 端口。

将下面的站点块加入 `/etc/caddy/Caddyfile`，把域名改成真实值：

```caddyfile
id.example.com {
    reverse_proxy 127.0.0.1:8805
}
```

确认防火墙规则已生效后启动：

```bash
fundeploy service casdoor start
fundeploy service casdoor status
sudo caddy validate --config /etc/caddy/Caddyfile
sudo systemctl enable --now caddy
sudo systemctl reload caddy
curl -fsS https://id.example.com/.well-known/openid-configuration
```

最后一行换成真实域名，确认返回的 `issuer` 为该 HTTPS 域名。Caddy 自动申请和续期
TLS 证书。初次登录使用组织 `built-in`、用户名 `admin` 和配置文件里的密码；建议立即
给管理员启用 MFA。`fundeploy service casdoor status` 显示服务状态和日志路径。
`start` 只启动后台进程，不会配置开机自启；生产主机需由现有服务管理器安排启动。

## 开通登录方式

在 Casdoor 管理界面建一个 `farfarfun` Organization，并为业务入口建 Application；
用户放在同一 Organization 下。每个业务应用各建独立 Application / OIDC client，
不要共用 client secret。

1. **邮箱**：新建 Email Provider，填写 SMTP 凭据，先执行测试发送，再关联到
   Application。Signin methods 加入 `Verification code`；注册表单的邮箱字段保持
   正常验证码验证。
2. **手机号**：新建 SMS Provider，选择阿里云、腾讯云等已开通的短信服务，填写
   模板和密钥并测试发送；关联到 Application，`Verification code` 允许手机号。
   注册表单的手机字段保持验证码验证。
3. **微信**：PC 扫码需要微信开放平台已审核的网页应用，建 `WeChat / Web` Provider；
   微信内网页需要公众号，另建 `WeChat / Mobile` Provider。将所需 Provider 关联到
   Application，Signin methods 加入 `WeChat`。普通手机浏览器不能直接使用公众号
   网页授权，小程序也需单独配置。

Application 的 **Binding rule** 可用 `Email`、`Phone`，它们仅匹配已验证的
标识；不要添加 `Name`。微信若没有提供邮箱或手机号，用户登录后应在账号页完成
绑定。验证码有效期在生成的 `data/app.conf` 中设为 5 分钟；发送频率、
注册开关和各 Provider 的 `Can sign up` 按实际开放策略设置。微信、短信、邮箱凭据
只在 Casdoor 管理界面保存，不提交到本仓库。

## 应用接入

给每个 Application 填准确的 HTTPS **Redirect URLs**，避免通配符。业务应用使用
授权码 + PKCE S256，并校验 `state`、`nonce`、ID Token 签名、`iss`、`aud`、`exp`。
以 `(issuer, sub)` 作为稳定外部身份绑定本地用户，业务权限仍由应用决定。

| 配置 | 值 |
| --- | --- |
| Issuer | `https://<SSO_HOSTNAME>` |
| Discovery | `<issuer>/.well-known/openid-configuration` |
| Client ID / secret | 对应 Casdoor Application 的值 |
| Scope | `openid profile email phone`，按需取用 |
| Redirect URI | 该 Application 中登记的准确回调地址 |

Web 后端宜签发自己的 `HttpOnly; Secure; SameSite=Lax` 会话 cookie。独立 API
应校验访问令牌的目标 audience，不能把 ID Token 当 API Bearer 令牌。退出使用
discovery 中的 `end_session_endpoint`；若要求所有应用立即退出，业务后端还需支持
back-channel logout 和可撤销的本地会话。

## 备份与升级

SQLite 适合单实例；需要多实例或更高并发时再迁移到 PostgreSQL。`data/` 包含
数据库、上传文件和管理员初始密码。最简单的一致性备份方式是短暂停机：

```bash
umask 077
fundeploy service casdoor stop
tar -C "$HOME/.farfarfun/casdoor" -czf "$HOME/casdoor-$(date +%F).tar.gz" data
fundeploy service casdoor start
```

备份加密保存并演练恢复。升级前先备份，再用 `fundeploy service casdoor upgrade`；
回退用 `fundeploy service casdoor rollback <版本>`。管理界面的一键自升级已关闭。

上游文档：[原生安装](https://casdoor.ai/docs/basic/server-installation/)、
[OIDC 接入](https://casdoor.ai/docs/how-to-connect/oidc-client/)、
[微信](https://casdoor.ai/docs/provider/oauth/Wechat/)、
[短信](https://casdoor.ai/docs/provider/sms/overview/)、
[邮箱](https://casdoor.ai/docs/provider/email/overview/)。
