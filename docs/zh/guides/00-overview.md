# 选择部署路径

部署 OpenViking 需要准备模型访问、持久化存储和用户身份。下面先给出已安装服务端的初始化流程，再按本地、自建共享或企业交付选择完整步骤。

## 初始化自建服务

安装服务端并准备 Embedding、VLM 凭据后，在服务主机运行配置向导。用于团队访问时，配置持久化 workspace、监听地址和 `server.root_api_key`，再为业务调用创建用户密钥。

```bash
openviking-server init
openviking-server doctor
openviking-server
```

`doctor` 通过后启动服务，并从业务客户端完成[数据链路验收](03-deployment.md#data-path-check)。首次安装和用户创建的完整步骤分别见[服务端部署](03-deployment.md)和[认证](04-authentication.md)。已有服务地址与用户密钥时可直接[连接 CLI](../getting-started/05-cli-setup.md)。

## 选择部署路径

| 部署目标 | 阅读顺序 | 完成标准 |
| --- | --- | --- |
| 本地体验 | [快速开始](../getting-started/02-quickstart.md) | 导入一份文档，检索并读取其中的内容 |
| 自建共享服务 | [自建服务](03-deployment.md) → [身份认证](04-authentication.md) → [公网访问](12-public-access.md) | 验证持久化、授权访问、未授权拒绝和首次数据处理 |
| 企业私有化交付 | [部署前检查](19-deployment-checklist.md) → [安装与验收](20-private-deployment.md) → [升级与排障](21-private-operations.md) | 按交付物清单与安装指南逐项验收 |

企业私有化路径依赖交付物、镜像和环境条件，先核对清单。

## 配置模型与访问控制

首次设置模型，使用[配置模型与服务](01-configuration.md)；查某个字段，使用[服务端配置字段](../configuration/01-server.md)。服务端 `ov.conf` 与客户端 `ovcli.conf` 的职责不同，客户端字段见[客户端配置](../configuration/02-client.md)。

开放远程访问前完成身份认证和反向代理配置。需要 OAuth 的 MCP 客户端继续阅读 [OAuth 2.1](11-oauth.md)。多用户共享时，理解[多租户身份](../concepts/11-multi-tenant.md)和 [ACL 继承](../concepts/15-acl.md)；需要静态数据加密时，按[加密指南](08-encryption.md)配置。

## 运行后如何检查

先从[观测与排障入口](05-observability.md)选择工具。单次请求看[操作遥测](07-operation-telemetry.md)，时间趋势看 [Prometheus / Grafana](11-grafana-prometheus.md)，指标含义查[指标口径](../concepts/12-metrics.md)。常见问题见[FAQ](../faq/faq.md)。

有存储或性能需求时，再选择[主备存储](13-multi-write-storage.md)、[RAGFS 缓存](14-ragfs-cache.md)或 [cuVS](16-cuvs.md)。这些是按需配置项，不是首次部署的必经步骤。
