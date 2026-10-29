<a id="示例-日报"></a>

# 日报

把带时间戳的对话记录、Agent 会话、IM 消息、协作文档、会议纪要、任务记录等材料，编译成简洁、有出处的**日报**：每天一页。

Skill 源码：[openviking/system_assets/compile/example-skills/daily-report](https://github.com/volcengine/OpenViking/tree/main/openviking/system_assets/compile/example-skills/daily-report)

先确认[前置条件](01-overview.md#前置条件)，并在 OpenViking 仓库根目录运行以下命令。来源目录需替换为自己的资料目录。

<a id="第一步-准备来源"></a>

## 准备来源

日报的来源通常是已经在 OpenViking 里的会话、消息或文档。如果要从本地导入一批记录：

```bash
ov add-resource ./work-logs --to viking://resources/work-logs --wait
ov ls -r viking://resources/work-logs
```

<a id="第二步-添加-skill"></a>

## 添加 Skill

```bash
ov add-skill openviking/system_assets/compile/example-skills/daily-report -p viking://agent/skills --wait
ov skills list
# → viking://agent/skills/daily-report
```

<a id="第三步-执行编译"></a>

## 执行编译

在 `--instruction` 里说清楚**日期、时区、报告对象和侧重点**，Skill 会据此定位和取舍：

```bash
ov compile \
  --from viking://resources/work-logs \
  --to viking://resources/daily-report \
  --skill viking://agent/skills/daily-report \
  --instruction "按 Asia/Shanghai 时区生成 2026-08-20 的日报，聚焦我的工作产出与决策"
```

一次生成多天，把日期范围写进 `--instruction` 即可（每天仍是独立一页）：

```bash
ov compile \
  --from viking://resources/work-logs \
  --to viking://resources/daily-report \
  --skill viking://agent/skills/daily-report \
  --instruction "按 Asia/Shanghai 时区生成 2026-08-18 至 2026-08-20 每天一份日报"
```

命令会立刻返回 `task_id`：

```bash
ov task status cmp_01abc      # 查看进度与最终结果
ov task cancel cmp_01abc      # 协作式取消
```

<a id="第四步-查看产物"></a>

## 查看产物

日报是纯 Markdown，直接读即可：

```bash
ov tree viking://resources/daily-report
ov read viking://resources/daily-report/2026-08-20.md
```


## 产物核对

结构示意（非实测输出）：

```markdown
# 2026-08-20 日报
## 完成事项
- 完成反馈导出演练。[来源](viking://resources/work-logs/demo.md)
## 待办与阻塞
- 离线安装包的最终交付时间尚未确认。
```

上面的虚构材料只示意正文结构，实际产物以来源记录为准。逐项检查日期与时区、完成状态、负责人和来源；来源中未完成的工作不能写成已完成，没有记录的事项也不应补写。

## 相关文档

- [上下文编译](./01-overview.md)
- [知识蒸馏示例](./05-knowledge-distillation.md)
- [Agent Runtime API](../api/23-agent-runtime.md)
