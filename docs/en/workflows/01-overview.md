<a id="application-workflows"></a>

# Develop applications

OpenViking provides searchable resources, cross-session memories, skills, and rules for applications. Choose a task, run a complete example in its guide, and check the result before integrating it into business code.

## Before you start

Prepare a reachable OpenViking service, credentials for the current user, and a client. You can use an existing service; for initial server and CLI setup, follow the [Quickstart](../getting-started/02-quickstart.md). Applications can use Python, TypeScript, Go SDKs, or HTTP APIs.

## Choose a task

| Application goal | Complete guide | Check before integration |
| --- | --- | --- |
| Retrieve documents or code | [Resource retrieval](../getting-started/08-resources.md) | Import completes, matched URIs can be read, answers agree with sources |
| Reuse context in another session | [Use memory](../getting-started/09-memory.md) | Commit and extraction complete, a new session finds and uses the memory |
| Reuse a task method | [Use skills](../getting-started/10-skills.md) | The skill is visible and the Agent reads and applies it |
| Check explicit constraints | [Rule validation](02-rule-validation.md) | Compilation and indexing complete; the Agent checks actual evidence |
| Produce a Wiki, graph, or report | [Context compilation](../context-compilation/01-overview.md) | Output is readable and conclusions or relations trace back to sources |
| Migrate existing resources | [Import and export](../getting-started/15-transfer.md) | The target service can read and retrieve imported content |

## Integrate into an application

After completing a workflow with fixed samples, substitute business data and retain task IDs, matched URIs, and errors. Wait for asynchronous processing before reading its output. Keep User and Peer scope aligned with the caller. A successful API response does not establish factual answer quality.

<a id="verify-retrieval-before-wiring-it-into-an-application"></a>
<a id="verify-memory-across-sessions"></a>
<a id="add-access-control-and-operations-before-rollout"></a>

## Next steps

- [SDK, HTTP and CLI conventions](../api/01-overview.md): choose a client and check authentication, responses, and errors.
- [Deploy and operate](../guides/00-overview.md): verify persistence, access control, observation, and recovery before rollout.
- [Project asset sync](../guides/18-openviking-assets.md): keep sources current; [Snapshots and recovery](../guides/15-snapshot.md): retain and restore versions.
