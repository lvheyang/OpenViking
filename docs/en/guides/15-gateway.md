---
description: Give any API-key model client OpenViking memory by pointing it at OpenViking Gateway.
---

# OpenViking Gateway

OpenViking Gateway is a model API proxy that gives OpenViking memory to any client that lets you change its base URL. You change two settings: the base URL becomes the gateway address, and the provider's API key becomes a gateway key. There is no plugin to install and no code or prompt to change.

While it forwards a request, the gateway does three things:

| Capability | What the gateway does |
| --- | --- |
| Automatic recall | For each new user message, the gateway searches OpenViking, appends the relevant memory to that message, and then passes it to the model. |
| Model-driven memory | The model can call OpenViking tools to search, read, write and import material. The gateway runs the tools, so the client does not declare or implement any of them. |
| Saving and compaction | The gateway saves conversations to OpenViking, which extracts new memories from them. When a conversation nears the model's context window, the gateway has the same model write a summary that replaces the earlier part. |

> **Note**: OpenViking Gateway is in beta. It is a different component from VikingBot Gateway (`vikingbot gateway`), which handles remote access and chat platforms.

## Who it is for {#who-it-is-for}

- **For**: clients that cannot install a plugin or MCP server, such as chat apps, SDK scripts and low-code platforms, and teams that want to manage providers, keys and memory settings in one place.
- **Not for**: clients signed in directly with a subscription (use a [subscription proxy upstream](../gateway/07-upstreams.md#custom-upstreams) instead; Codex can use a ChatGPT subscription this way, see [Use a ChatGPT subscription](../gateway/03-connect-clients.md#codex-chatgpt-subscription)), and clients such as Cursor and Trae whose model calls leave from the vendor's servers.

The gateway supports three model APIs: Anthropic Messages, OpenAI Chat Completions and OpenAI Responses (when every request carries the full history; see [Client requirements](../gateway/04-openviking-tools.md#client-requirements)). It forwards each request to a model endpoint you configure, called an **upstream**. A request goes out in the same API it came in; the gateway never converts between APIs.

## Connection flow

1. Enable the gateway and open Studio; prepare a reachable model upstream.
2. Test the upstream, select a context profile, and issue a gateway key (`ovgw_…`).
3. Set the client address and key, send a request, then check saving, extraction, and recall in a new conversation.

For a first experience with screenshots, follow [Gateway integration](../getting-started/12-gateway.md). For an existing service, choose the local or remote path in [Quickstart](../gateway/02-quickstart.md).

<a id="key-terms"></a>

For terminology and settings, see the [configuration reference](../gateway/13-configuration.md#key-terms).

## Architecture at a glance {#architecture-at-a-glance}

![OpenViking Gateway in its two forms. Embedded: clients call the OpenViking Server port, which passes everything under /gateway (model APIs, uploads and gateway management) to the gateway in the same process; the gateway searches and saves memory over loopback HTTP with the user's own OpenViking key. Standalone: openviking-gateway on its own port serves the same /gateway paths and a gateway-only Studio, and calls an existing OpenViking service over HTTP(S). Either way the gateway forwards each request to the upstream model provider in the same API, sessions and memories live in OpenViking, and the gateway keeps its own data in two local SQLite files by default or in shared Redis, encrypted when OpenViking encryption is on](../../images/gateway/architecture.en.svg)

- **Where it runs**: once enabled, inside OpenViking Server on the server's port. You can also run `openviking-gateway` on its own against an existing OpenViking service, open source or OpenViking Cloud. Either way, every gateway URL is under `/gateway`, such as `/gateway/v1`, so clients use the same paths. See [Choose a deployment form](22-gateway-operations.md#deployment-options).
- **How it relates to OpenViking**: the gateway only calls OpenViking's public APIs. Embedding and memory extraction happen in OpenViking.
- **Where the data lives**: sessions and memories live in OpenViking. The gateway defaults to two SQLite files and can use shared Redis for multiple replicas. OpenViking encryption protects stored values in either backend. See [Security and data](../gateway/11-security-and-data.md).
- **When something fails**: if OpenViking is down, conversations go on without memory and saving retries on its own. If the gateway is down, every model call that goes through it fails.

## What you get and what it costs {#what-you-get-and-what-it-costs}

What you get:

- relevant memory on every new message;
- your user profile at the start of a new conversation;
- conversations saved to OpenViking and turned into new memories;
- automatic compaction of long conversations;
- with OpenViking tools on, a model that searches, reads and writes memory itself (see [Agentic memory for any client](../gateway/04-openviking-tools.md#agentic-memory-for-any-client)).

What it costs:

- the first model call of each new message can wait up to 2 extra seconds;
- added memory is billed as input tokens, mostly at the cached rate on later requests;
- the turn that triggers compaction makes one extra model call.

For how each of these works, turn by turn, see [How it works](../gateway/01-how-it-works.md).

## Gateway or plugin? {#gateway-or-plugin}

OpenViking also connects to agents through plugins, such as those for Claude Code, Codex, Cursor, TRAE, OpenCode, pi, DSH, OpenClaw and Hermes (see [Choose an integration](../agent-integrations/01-overview.md)). Both read and write the same OpenViking memory. What separates them is where they run: the gateway sits on the model call path and changes the request the model receives, while a plugin runs inside one agent and works through that agent's events. Neither is better in general; most differences below follow from that one fact, and some vary from plugin to plugin.

<details>
<summary>Compare gateway and plugin behavior</summary>

### Side by side {#gateway-and-plugin-compared}

| | OpenViking Gateway | Agent plugin |
| --- | --- | --- |
| Where it runs | One shared service between clients and the model provider. It sees only model requests. | Inside each agent, through its hooks or extension API. It sees the agent's sessions, events and workspace. |
| Clients | Any client whose base URL and API key you can set, in Anthropic Messages, Chat Completions or Responses: chat apps, SDK scripts, low-code platforms and coding agents. Not Cursor or TRAE, whose model calls leave from the vendor's servers. | Only agents that have an OpenViking plugin. Other MCP clients can add the OpenViking tools, without automatic recall or saving. |
| Model sign-in | Needs a provider API key, held by the gateway or [sent by each client](../gateway/03-connect-clients.md#bring-your-own-provider-key). Clients cannot sign in with a subscription; use a [subscription proxy upstream](../gateway/07-upstreams.md#custom-upstreams) instead. Codex can use a ChatGPT subscription through [CLIProxyAPI](../gateway/03-connect-clients.md#codex-chatgpt-subscription); for other subscription services, check their terms yourself. | Does not touch model calls, so the agent keeps its own sign-in, subscriptions included. |
| Recall | For each new user message, appended to that message; the search waits at most 2 seconds by default. Searches memories, resources and skills by default. | On the agent's prompt event. Most plugins send the session ID so the server can expand the query from the conversation, and leave resources out of automatic recall. |
| Saving | Finished turns, with the client's own tool calls and results, one turn behind. Sub-agent and housekeeping requests are not saved. | On the agent's events, usually at the end of each turn. Sub-agents differ: Claude Code, OpenCode and DSH save them as separate sessions, Codex folds them into the main session, Hermes skips them. |
| OpenViking tools | The gateway runs them during the reply, so clients without tool or MCP support get them too. Read-only tools by default. Calls [skip the client's permission prompts](../gateway/04-openviking-tools.md#risks-and-controls) and show a one-line notice. | The agent runs them as its own tools, so calls and results appear in the transcript and follow the agent's tool permissions. |
| Prompt caching | Puts each memory block back byte for byte on later requests, so the cached prefix holds. The **Overview** tab shows the cache hit rate. | Depends on the plugin. Hook-based plugins hand the block to the agent, which keeps it in its history; pi and OpenCode v2 also keep earlier recall stable; OpenClaw rebuilds recall in the system prompt every turn. |
| Long conversations | Compacts near the model's context window for any client, including apps that never compact. The [experimental agent-managed context windows](../gateway/05-long-conversations.md#experimental-agent-managed-context-windows) let the model start fresh windows itself. | The agent compacts; most plugins commit what they captured first. pi and OpenClaw can replace the agent's summary with OpenViking's. |
| Per-project memory | Cannot see the working directory. To keep projects apart, give each one a gateway key bound to a different OpenViking user. | Plugins built on the shared plugin code tag memories with the current repository automatically. |
| Visibility | Added memory is invisible unless the profile turns on the [recall summary](../gateway/08-context-profiles.md#recall-summary). Studio's request log shows counts and saving status, not content. | Claude Code's status line and `/openviking-memory:ov` command show recall counts and where added context came from; pi's status line shows the connection. Other plugins have no indicator of their own. |
| Setup and operation | One service, embedded in OpenViking Server or standalone. Users change only a base URL and a key; upstreams, profiles and keys are managed in Studio. | Installed and upgraded on each machine, each with its own OpenViking credentials. |
| Teams | An admin issues one gateway key per person and can revoke each one. Users never see the provider key when the gateway holds it. | Each person installs the plugin with their own OpenViking API key. Behavior settings can be shared per repository in `.openviking/config.json`. |
| When something fails | Without OpenViking, requests go on without memory and saving retries. If the gateway is down (for the embedded form, the OpenViking Server), every model call through it fails. It holds provider keys and unsaved text centrally, and adds one network hop to every model call. | Without OpenViking, the agent goes on without memory after the plugin's recall timeout (15 seconds for most, 4 for Hermes); most plugins queue failed writes on disk. Model calls never depend on OpenViking. |

The [capability reference](../agent-integrations/16-capability-reference.md) has the per-plugin details; [How it works](../gateway/01-how-it-works.md) has the gateway's.

</details>

### Which to choose {#which-to-choose}

Choose the gateway when:

- the client has no OpenViking plugin: chat apps, SDK scripts, low-code platforms, or your own app that calls a model API;
- several people or clients should share one setup, with providers, memory settings and per-person keys managed in Studio and nothing installed on users' machines;
- the client lacks memory tools or compaction of its own, since the gateway adds both;
- your clients already use provider API keys, or you route model traffic through a central proxy anyway.

Choose a plugin when:

- you sign in to Claude Code with a Claude subscription;
- you use Cursor or TRAE, which only a plugin can reach;
- you want each repository's memory tagged without setup, or every OpenViking tool call shown and approved in the agent;
- you want saving to follow the agent's events, such as sub-agent sessions, a save before the agent compacts and a commit at exit;
- you do not want another service on the model call path.

Codex with a ChatGPT subscription works either way: with its plugin, or through the gateway with a [CLIProxyAPI upstream](../gateway/03-connect-clients.md#codex-chatgpt-subscription).

You can mix them: give your coding agents their plugins and send everything else through the gateway. For one client with both, see the next section.

> **Try new features early**: the gateway controls the whole model request, which allows features that a plugin cannot add through an agent's extension API. [Agent-managed context windows](../gateway/05-long-conversations.md#experimental-agent-managed-context-windows), where the model decides when to start a fresh context window, is the first of these, and more are planned for the gateway. If you want to try them early, connect a client through the gateway.

### When both are in use {#when-both-are-in-use}

When a request shows signs of an OpenViking plugin, the gateway steps aside: for that [conversation](#key-terms) it stops adding memory, saving turns and offering OpenViking tools, and only forwards requests. That keeps the same content from being added or saved twice. It looks for two signs:

- memory blocks that plugins insert: `<openviking-context>` (the plugins built on the shared plugin code, such as Claude Code, Codex, OpenCode, pi and DSH), `<relevant-memories>` (OpenClaw), `<memory-context>` (Hermes) or `<relevant-memory>`;
- an `X-OpenViking-Plugin` request header, which plugin and app authors can send to opt out explicitly.

The decision holds for the rest of that conversation and does not affect others. To use gateway memory again, start a new conversation without the plugin. Studio's request log flags these requests **OpenViking plugin in use**. In effect the plugin handles memory and the gateway only forwards, while the client still needs a gateway key and an upstream with a provider API key.

OpenViking MCP tools alone, without a plugin, do not count. The gateway still adds memory and saves turns; it only stops offering its own OpenViking tools, so the model does not see two sets with the same names. In the request details on the **Requests** tab, **OpenViking tools** then reads **Not offered: The client already has OpenViking tools**; see [When tools are not offered](../gateway/04-openviking-tools.md#when-tools-are-not-offered).

## Where to go next {#next-steps}

### By task {#by-task}

| I want to… | Go to |
| --- | --- |
| Understand what happens in a turn, what the model sees and why it works this way | [How it works](../gateway/01-how-it-works.md) |
| Try it end to end, from a gateway key to proof that memory works | [Get started](../gateway/02-quickstart.md) |
| Connect Claude Code, Codex, a chat app or an SDK | [Connect clients](../gateway/03-connect-clients.md) |
| Let the model use memory tools, or handle long conversations | [OpenViking tools](../gateway/04-openviking-tools.md) and [Long conversations in the gateway](../gateway/05-long-conversations.md) |
| Deploy, expose, upgrade and monitor the gateway | [Deployment and operations](22-gateway-operations.md) |
| Review what is stored, who can see it and how to delete it | [Security and data](../gateway/11-security-and-data.md) |
| Fix an error, a warning or missing memory | [Troubleshooting](../gateway/12-troubleshooting.md) |
| Look up a setting's name, default or range | [Configuration reference](../gateway/13-configuration.md) |
| Manage the gateway over HTTP | [Management API](../api/25-gateway.md) |

### Studio operations {#by-studio-tab}

For upstreams, context profiles, keys, and request logs, see [Manage in Studio](../gateway/06-manage-in-studio.md).

### Related docs {#related-docs}

- [Authentication](04-authentication.md): create accounts, users and their keys.
- [Public Access & Reverse Proxy](12-public-access.md): put OpenViking behind HTTPS.
- [Agent Integrations](../agent-integrations/01-overview.md): plugins for agents that support them.
- [MCP Integration](06-mcp-integration.md): give MCP clients OpenViking tools directly.
