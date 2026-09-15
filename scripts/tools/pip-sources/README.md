# pip 镜像配置

脚本并行探测常用 pip 索引的响应延迟，将最快的可用源设为
`index-url`，其余源设为 `extra-index-url`。配置读写全部委托给
`python -m pip config --user`，不直接解析或生成 `pip.conf`。

## 使用

```bash
fundeploy dev pip install
fundeploy dev pip update
fundeploy dev pip status
fundeploy dev pip uninstall
```

在仓库中可直接运行：

```bash
./scripts/tools/pip-sources/setup.sh
./scripts/tools/pip-sources/setup.sh -v
NONINTERACTIVE=1 ./scripts/tools/pip-sources/setup.sh
```

也支持单文件执行：

```bash
curl -LsSf https://raw.githubusercontent.com/farfarfun/fundeploy/HEAD/scripts/tools/pip-sources/setup.sh | bash
curl -LsSf https://gitee.com/farfarfun/fundeploy/raw/master/scripts/tools/pip-sources/setup.sh | bash
```

无命令时等同于 `install`。交互终端在写入前会确认，
`NONINTERACTIVE=1` 会直接配置。

## 行为

1. 通过 `pip config --user get` 读取已有的 `index-url` 和
   `extra-index-url`。
2. 并行请求每个源的 `setuptools` 索引页。
3. 按延迟排序可用源；已有自定义源临时不可用时仍保留在末尾。
4. 通过 `pip config --user set` 写入配置。
5. `uninstall` 清除用户级 `index-url`、`extra-index-url` 和
   `trusted-host`。

HTTP 明文源默认排除。确实需要内网明文镜像时设置
`PIP_SOURCES_ALLOW_INSECURE=1`，脚本只会为这些 HTTP 源生成
`trusted-host`，不会关闭 HTTPS 源的证书校验。

## 环境变量

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `PIP_SOURCES_PYTHON` | `python3` | 执行 pip 配置的 Python |
| `PIP_SOURCES_TEST_TIMEOUT` | `5` | 单次请求总超时，单位秒 |
| `PIP_SOURCES_CONNECT_TIMEOUT` | `2` | 连接超时，单位秒 |
| `PIP_SOURCES_PARALLEL_JOBS` | `8` | 每批并行探测数量 |
| `PIP_SOURCES_TEST_PACKAGE` | `setuptools` | 延迟探测包 |
| `PIP_SOURCES_ALLOW_INSECURE` | `0` | 设为 `1` 允许 HTTP 源 |
| `NONINTERACTIVE` | `0` | 设为 `1` 跳过写入确认 |

## 内置源

`tsinghua`、`aliyun`、`douban`、`tencent`、`huawei`、
`ustc`、`bfsu`、`sjtu`、`official`，以及需要对应网络环境的
`artlab-visable`、`artlab-pai`、`artlab-aop`、`antfin`。

`hust`、`tbsite` 和 `tbsite_aliyun` 使用 HTTP，仅在显式允许时参与探测。

配置文件的实际位置和优先级由 pip 决定，可用以下命令检查：

```bash
python3 -m pip config debug
python3 -m pip config --user list
```
