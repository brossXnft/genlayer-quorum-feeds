#!/usr/bin/env python3
"""Create the feeds in examples/feeds.json on a deployed Quorum Feeds contract.

  seed_feeds.py <contract> [--only BTC-USD,ETH-USD] [--update] [--send]

Without --send: simulate each call on a node (gen_call) and print the result.
--update also pushes a first value for each feed.
Needs glcall.py / gl_tx.py from the tools folder next to this script (or $GL_TOOLS).
"""
import importlib.util as u
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
TOOLS = os.environ.get("GL_TOOLS", os.path.join(HERE, "..", "tools"))
s = u.spec_from_file_location("glcall", os.path.join(TOOLS, "glcall.py")); c = u.module_from_spec(s); s.loader.exec_module(c)
g = c.g


def sim(addr, method, args):
    import rlp
    data = rlp.encode([g.cd_encode(g.call_obj(method, args)), b"\x00"])
    r = g.rpc("gen_call", [{"type": "write", "to": addr, "from": g.account().address, "data": "0x" + data.hex(),
                           "transaction_hash_variant": "latest-nonfinal"}], url=g.GEN_RPC)
    d = r.get("data") or ""
    val = g.cd_decode(bytes.fromhex(d.removeprefix("0x"))) if d else None
    ok = not (r.get("status") or {}).get("code")
    return ok, val if ok else (val or r.get("stderr", "")[-400:])


def main():
    a = sys.argv[1:]
    if not a:
        print(__doc__); sys.exit(1)
    addr, send = a[0], "--send" in a
    only = set(a[a.index("--only") + 1].split(",")) if "--only" in a else None
    feeds = json.load(open(os.path.join(HERE, "..", "examples", "feeds.json")))
    for fd in feeds:
        if only and fd["id"] not in only:
            continue
        args = [fd["id"], fd["unit"], fd["description"], fd["decimals"], fd["tol_bps"], fd["min_sources"],
                fd["max_spread_bps"], fd["sources"]]
        ok, val = sim(addr, "create_feed", args)
        print(f"[{fd['id']}] create (simulated): {'ok' if ok else 'FAILED'} {str(val)[:300]}")
        if send and ok:
            c.call(addr, "create_feed", args, send=True, wait=10, label=f"create_feed {fd['id']}")
        if "--update" in a:
            ok, val = sim(addr, "update", [fd["id"]])
            print(f"[{fd['id']}] update (simulated): {'ok' if ok else 'FAILED'} {str(val)[:600]}")
            if send and ok:
                c.call(addr, "update", [fd["id"]], send=True, wait=10, label=f"update {fd['id']}")


if __name__ == "__main__":
    main()
