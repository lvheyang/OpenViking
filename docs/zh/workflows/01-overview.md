<a id="应用开发路径"></a>

# 开发应用

OpenViking 为应用提供可检索的资料、跨会话记忆、技能与规则。先选择应用需要完成的任务，再沿对应指南运行一个完整样例，核对结果后接入业务代码。

## 开始之前

准备可访问的 OpenViking 服务、当前用户凭据和一种客户端。已有服务可直接使用；首次启动服务和配置 CLI 时，先完成[快速开始](../getting-started/02-quickstart.md)。应用可使用 Python、TypeScript、Go SDK 或 HTTP API。

## 选择任务

| 应用目标 | 完整操作指南 | 接入业务前核对 |
| --- | --- | --- |
| 检索文档或代码 | [资料检索](../getting-started/08-resources.md) | 导入完成，命中 URI 可读取，答案与原文一致 |
| 跨会话使用已知背景 | [记忆使用](../getting-started/09-memory.md) | 提交和提取完成，新会话能找到记忆并使用它 |
| 复用任务方法 | [技能使用](../getting-started/10-skills.md) | 技能可见，Agent 读取并按它完成任务 |
| 核验强约束 | [规则校验](02-rule-validation.md) | 编译与索引完成，Agent 对照实际证据核验 |
| 整理 Wiki、图谱或日报 | [上下文编译](../context-compilation/01-overview.md) | 产物可读取，结论或关系能追溯到来源 |
| 迁移已有资料 | [导入导出](../getting-started/15-transfer.md) | 目标环境可读取和检索导入内容 |

## 接入应用

用固定样例完成一条任务链路后，将输入替换为业务资料，并保存任务 ID、命中 URI 与错误信息。异步任务完成后再读取结果；记忆的 User 与 Peer 范围要与实际调用者一致。接口返回成功不代表模型回答已符合事实。

<a id="先验证检索-再接入应用"></a>
<a id="再验证跨会话记忆"></a>
<a id="上线前补齐访问与运维"></a>

## 下一步

- [SDK、HTTP 与 CLI 约定](../api/01-overview.md)：选择调用方式，查询认证、返回和错误约定。
- [部署运维](../guides/00-overview.md)：上线前确认持久化、访问控制、观测与恢复。
- [项目资料同步](../guides/18-openviking-assets.md)：持续更新来源；[快照与恢复](../guides/15-snapshot.md)：保留和恢复版本。
