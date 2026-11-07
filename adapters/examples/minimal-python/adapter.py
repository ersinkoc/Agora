#!/usr/bin/env python3
"""Minimal L3 adapter (docs/specs/adapter-protocol.md) in Python, standard library only.

It speaks newline-delimited JSON-RPC 2.0 on stdio and exists to prove that an adapter in another language
passes the conformance transcript in schema/fixtures/adapter/. It does not drive a real agent.
"""
import json
import sys

NAME, VERSION, KIND = "minimal-python", "0.1.0", "example"


def send(obj):
    sys.stdout.write(json.dumps(obj, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def result(req_id, res):
    send({"jsonrpc": "2.0", "id": req_id, "result": res})


def main():
    session = None
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except ValueError:
            continue  # tolerate garbage; never crash
        method, req_id, params = msg.get("method"), msg.get("id"), msg.get("params") or {}
        if method == "initialize":
            result(req_id, {"adapterName": NAME, "version": VERSION, "agentKind": KIND,
                            "capabilities": ["chat", "cancel"]})
        elif method == "session/start":
            session = "s-1"
            result(req_id, {"sessionId": session})
            send({"jsonrpc": "2.0", "method": "state/update",
                  "params": {"sessionId": session, "state": "AGENT_STATE_IDLE"}})
        elif method == "session/prompt":
            result(req_id, {})
            send({"jsonrpc": "2.0", "method": "chat/append",
                  "params": {"sessionId": session, "text": {"text": "echo: " + params.get("text", "")}}})
        elif method == "session/stop":
            session = None
            result(req_id, {})
        elif method == "shutdown":
            result(req_id, {})
            return
        elif req_id is not None:
            send({"jsonrpc": "2.0", "id": req_id, "error": {"code": -32601, "message": "method not found"}})
        # unknown notifications are ignored


if __name__ == "__main__":
    main()
