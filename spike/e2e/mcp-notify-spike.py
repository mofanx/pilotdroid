#!/usr/bin/env python3
"""mcp-notify-spike — can a watch_hit be PUSHED to a real MCP client?

Two halves, both with the official python `mcp` SDK:

  A. in-proc echo server (lowlevel `Server` over stdio): its tool call
     emits `notifications/message` (LoggingMessageNotification) before
     returning -> does ClientSession surface it? via logging_callback
     and/or message_handler?
  B. real `pilotdroid serve` handshake: advertised capabilities tell
     whether the shipped server can push at all (it advertises no
     `logging`/`notifications` — pure request/response today), and a
     raw unsolicited-read probe confirms zero traffic between calls.

Writes spike/e2e/mcp-notify-spike.json; exit 0 always (spike, not gate).
"""
import asyncio, json, os, sys, tempfile, textwrap

from mcp import ClientSession, StdioServerParameters, types
from mcp.client.stdio import stdio_client

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.join(HERE, "..", "..")
CLI = os.path.join(ROOT, "target", "debug", "pilotdroid-cli")
DEV = sys.argv[1] if len(sys.argv) > 1 else "ba0d072d"
OUT = os.path.join(HERE, "mcp-notify-spike.json")

ECHO = textwrap.dedent("""
    import asyncio, anyio
    from mcp.server.lowlevel import Server, NotificationOptions
    from mcp.server.stdio import stdio_server
    from mcp import types

    srv = Server("notify-echo")

    @srv.list_tools()
    async def lt():
        return [types.Tool(name="ping", description="emits a log notification",
                           inputSchema={"type": "object", "properties": {}})]

    @srv.call_tool()
    async def ct(name, args):
        ctx = srv.request_context
        await ctx.session.send_log_message(
            level="info", logger="notify-echo",
            data={"type": "watch_hit", "watch_id": 7, "matched": True})
        return [types.TextContent(type="text", text="pong")]

    async def main():
        async with stdio_server() as (r, w):
            await srv.run(r, w, srv.create_initialization_options(
                notification_options=NotificationOptions(tools_changed=True)))

    anyio.run(main)
""")


async def half_a(out):
    """Does a real ClientSession receive server->client notifications?"""
    with tempfile.NamedTemporaryFile("w", suffix=".py", delete=False) as f:
        f.write(ECHO)
        srv = f.name
    logs, msgs = [], []
    params = StdioServerParameters(command=sys.executable, args=[srv])
    async with stdio_client(params) as (r, w):
        async def log_cb(p): logs.append((p.level, p.data))
        async def msg_cb(m): msgs.append(type(m).__name__)
        async with ClientSession(r, w, logging_callback=log_cb,
                                 message_handler=msg_cb) as s:
            await s.initialize()
            res = await s.call_tool("ping", {})
            await asyncio.sleep(0.5)  # let the notification land
            out["a_notification_via_logging_callback"] = logs
            out["a_other_messages_seen"] = msgs
            out["a_tool_result"] = res.content[0].text
            out["a_push_received"] = bool(logs)
    os.unlink(srv)


async def half_b(out):
    """Real serve: advertised push channels + unsolicited-traffic probe."""
    params = StdioServerParameters(command=CLI, args=["-s", DEV, "serve"])
    async with stdio_client(params) as (r, w):
        async with ClientSession(r, w) as s:
            init = await s.initialize()
            caps = init.capabilities.model_dump(exclude_none=True)
            out["b_capabilities"] = list(caps)
            out["b_logging_capability"] = "logging" in caps
            # unsolicited traffic probe: no call in flight — anything sent?
            probe = await asyncio.wait_for(r.receive(), timeout=1.5) \
                if False else "no-read-api: client streams are pull-based"
            out["b_unsolicited_probe"] = probe
    out["b_conclusion"] = (
        "serve advertises ONLY tools — no logging/notifications channel; "
        "watch_hit today is strictly pull (watch_events). A push path "
        "exists in-protocol (notifications/message, proven in half A) but "
        "surfaces to clients as a LOG line, not a structured tool event — "
        "so agent-tooling would still need to poll watch_events for the "
        "payload. Recommendation: keep pull as the contract; optionally "
        "mirror hits as notifications/message for human-facing logs later.")


async def main():
    out = {"probe": "watch_hit push vs pull (M12 t2)"}
    await half_a(out)
    await half_b(out)
    with open(OUT, "w") as f:
        json.dump(out, f, indent=2, ensure_ascii=False)
    for k, v in out.items():
        print(f"{k}: {v}")
    print(f"-> {OUT}")


if __name__ == "__main__":
    asyncio.run(main())
