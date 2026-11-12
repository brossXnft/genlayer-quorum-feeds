#!/usr/bin/env python3
"""End-to-end check of a deployed Quorum Feeds contract on Bradbury.

  e2e.py <quorum-address> [--skip-seed] [--consumer <address>]

1. create + update every feed in examples/feeds.json (skipped with --skip-seed)
2. read feeds() and latest_fresh for each
3. deploy examples/consumer.py (Python) pointing at the oracle, unless --consumer is given
4. consumer.snapshot("BTC-USD", 3600) as a transaction, then read consumer.get_last()
Prints a JSON summary with every GenLayer transaction id at the end.
"""
import importlib.util as u
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
TOOLS = os.environ.get("GL_TOOLS", os.path.join(HERE, "..", "tools"))
s = u.spec_from_file_location("glcall", os.path.join(TOOLS, "glcall.py")); c = u.module_from_spec(s); s.loader.exec_module(c)
g = c.g
import rlp

summary = {"txs": []}


def send(addr, method, args, label):
    ok, val = c.simulate(addr, method, args)
    print(f"[{label}] simulated: {'ok' if ok else 'FAILED'} {str(val)[:400]}")
    if not ok:
        summary["txs"].append({"label": label, "simulated_error": str(val)[:300]})
        return None
    r = c.call(addr, method, args, send=True, wait=12, label=label)
    summary["txs"].append({"label": label, "tx_id": r and r["tx_id"], "outcome": r and r.get("outcome")})
    return r


def main():
    a = sys.argv[1:]
    oracle = a[0]
    feeds = json.load(open(os.path.join(HERE, "..", "examples", "feeds.json")))
    if "--skip-seed" not in a:
        for fd in feeds:
            args = [fd["id"], fd["unit"], fd["description"], fd["decimals"], fd["tol_bps"], fd["min_sources"],
                    fd["max_spread_bps"], fd["sources"]]
            send(oracle, "create_feed", args, f"create_feed {fd['id']}")
        for fd in feeds:
            send(oracle, "update", [fd["id"]], f"update {fd['id']}")
    for fd in feeds:
        ok, val = c.simulate(oracle, "latest_fresh", [fd["id"], 3600], typ="read")
        print(f"[latest_fresh {fd['id']}] {'ok' if ok else 'FAILED'} {str(val)[:600]}")
        summary.setdefault("latest", {})[fd["id"]] = val if ok else f"error: {val}"

    consumer = a[a.index("--consumer") + 1] if "--consumer" in a else None
    if not consumer:
        code = open(os.path.join(HERE, "..", "examples", "consumer.py"), "rb").read()
        acct = g.account()
        txcd = rlp.encode([code, g.cd_encode({"args": [oracle]}), b"\x00"])
        r = g.send(acct, g.build(acct, "0x" + "00" * 20, txcd, g.DEPLOY_T), g.DEPLOY_T, False, "deploy consumer.py")
        if r and r["ok"]:
            print("outcome:", g.watch(r["tx_id"], r["block"], 12, stop_on_accept=False))
            consumer = r["recipient"]
        summary["consumer_deploy"] = r and r["tx_id"]
    summary["consumer"] = consumer
    print("consumer", consumer)
    send(consumer, "snapshot", ["BTC-USD", 3600], "consumer.snapshot BTC-USD")
    ok, val = c.simulate(consumer, "get_last", [], typ="read")
    print("[consumer.get_last]", ok, val)
    summary["consumer_last"] = val
    print("SUMMARY " + json.dumps(summary, default=str))


if __name__ == "__main__":
    main()
