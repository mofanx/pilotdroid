#!/usr/bin/env python3
"""Real-MCP-client handshake check against `pilotdroid serve` (t6).

Uses the official python `mcp` SDK (v1.26.x): stdio transport, real
initialize -> notifications/initialized -> tools/list -> tools/call
sequence driven by ClientSession. Exit 0 iff all steps succeed.

Usage: python3 mcp_client_check.py [serial]
"""
import asyncio
import json
import os
import sys

from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.join(HERE, "..", "..")
CLI = os.path.join(ROOT, "target", "debug", "pilotdroid-cli")
DEV = sys.argv[1] if len(sys.argv) > 1 else "ba0d072d"


async def main():
    params = StdioServerParameters(command=CLI, args=["-s", DEV, "serve"])
    async with stdio_client(params) as (read, write):
        async with ClientSession(read, write) as session:
            init = await session.initialize()  # real MCP handshake
            print("serverInfo:", init.serverInfo.name, init.serverInfo.version)
            print("protocolVersion:", init.protocolVersion)
            print("capabilities:", list(init.capabilities.model_dump(exclude_none=True)))

            tools = await session.list_tools()
            names = [t.name for t in tools.tools]
            print("tools (%d):" % len(names), names)

            call = await session.call_tool("app_current", {})
            assert not call.isError, call.content
            print("app_current:", call.content[0].text)

            call2 = await session.call_tool("tap", {"x": 540, "y": 1200})
            assert not call2.isError, call2.content
            data = json.loads(call2.content[0].text)
            print("tap ok=%s ms=%.1f" % (data["ok"], data["ms"]))

            # malformed tool name -> isError content, not protocol error
            bad = await session.call_tool("no_such_tool", {})
            print("bad tool isError:", bad.isError)


if __name__ == "__main__":
    asyncio.run(main())
    print("MCP CLIENT HANDSHAKE PASS")
