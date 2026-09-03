# Changelog

## [0.1.15]

### 新增

- README 补充「关于 farfarfun」组织介绍区块，末尾附精确的 MIT 协议说明句。

### 修复

- `.gitignore` 补齐 `__pycache__/`、`*.pyc`、`*.db`、`*.rar`、`.venv/`、`.run/`、
  `node_modules/` 等规范要求的规则。

### 变更

- 无

### 废弃

- 无

> 说明：`scripts/setup.sh` 统一入口、`dev`/`prod` 环境参数强制校验、运行时文件
> 迁移到 `.run/` 涉及对 8 个独立服务脚本（airflow/celery/paperclip/code-server/
> new-api/sub2api/open-pencil/funflix-web）的行为改动，且这些脚本管理的是安装
> 在用户本机 `~/opt/<service>` 下的第三方长期运行服务，与 `bash-service-guide`
> 面向的「仓库自身持有的后端/worker/前端 dev server」场景不完全一致（没有天然
> 的 dev/prod 环境划分，强行拆分环境参数意义不明确；迁移运行时文件路径会影响
> 已安装用户的现有 PID/日志文件位置，需要兼容迁移方案）。这部分改动留待仓库
> 所有者确认适配方式后再单独处理，详见 issue #383 的说明。
