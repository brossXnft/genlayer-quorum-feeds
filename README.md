# Quorum Feeds

Multi-source numeric feeds for GenLayer, written in Rust. Anyone defines a feed from public JSON APIs; every update is the median of those sources, accepted only if each validator fetches the same sources itself and lands within a set tolerance of the leader. Other Intelligent Contracts, in Rust or Python, read the result with one view call and can refuse stale values.

- **Contract (Bradbury testnet):** `0x0aa3eefa8769c5a3c16a65eff8ef0bcdd4a3a3ba` ([explorer](https://scan.bradbury.genlayer.com/address/0x0aa3eefa8769c5a3c16a65eff8ef0bcdd4a3a3ba))
- **Language:** Rust, `genlayer_sdk` (genvm-executor `v0.3-dev` @ `6ec6c39`), about 250 KB of WASM, deployed with [genlayer-chunked-deploy](https://github.com/brossXnft/genlayer-chunked-deploy)
- **Live feeds:** see [Deployment](#deployment)

## The problem

A contract that settles on a price or a rate needs a number it can trust. Asking an LLM for it is not acceptable, a single API is a single point of failure, and strict equality between validators can never pass for a value that moves every second. GenLayer's equivalence principle allows a better rule, but every builder ends up re-implementing it: fetch a few sources, parse decimals without floating point, drop outliers, compare with a tolerance, and store the value with a timestamp. Quorum Feeds is that primitive, done once and shared.

## How consensus is used

`update(id)` runs one nondeterministic block:

1. **Leader** fetches every source of the feed, reads the number at each source's JSON path as exact decimal text, and converts it to an integer scaled by `10^decimals`. Sources further than `max_spread_bps` from the raw median are marked as outliers and dropped. If fewer than `min_sources` remain, the update fails. Otherwise the leader proposes the median of the kept values.
2. **Each validator** repeats the whole observation independently and votes *agree* only if its own median is within `tol_bps` of the leader's and the leader kept at least `min_sources`. A validator that cannot reach the quorum itself votes *disagree*. It agrees with a leader that reported "no quorum" only if it also has no quorum.
3. **Deterministic code** re-checks the accepted result (positive, quorum met), then stores the round: value, number of kept sources, per-source report, transaction timestamp, round number, and the updater. The last 48 rounds stay in `history`.

Why these choices:

| Choice | Reason |
|---|---|
| Median, not mean | One wrong or manipulated source cannot move it. |
| Outlier filter before the median | A source that is far off (stale cache, wrong market) is excluded and flagged in the report instead of silently skewing the value. |
| Tolerance comparison | Validators fetch a few seconds after the leader; prices move. `tol_bps` is the agreed noise band. |
| Integers scaled by `10^decimals` | GenVM forbids floating point in deterministic mode. Parsing keeps the API's exact decimal text (JSON parsed with `arbitrary_precision`), so `0.1` is never rounded. |
| Transaction timestamp | Consumers compare `at` with their own transaction time; `latest_fresh` does this for them. |

## Using a feed from another contract

Python (see [`examples/consumer.py`](examples/consumer.py), deployed and tested on Bradbury):

```python
r = gl.contract.get_at(QUORUM_FEEDS).view().latest_fresh("BTC-USD", 3600)
price_cents = r["value"]        # integer, r["decimals"] == 2
print(r["text"], r["unit"])     # "91234.5 USD"
```

`latest_fresh(id, max_age_s)` fails if the last update is older than `max_age_s` relative to the calling transaction, so a consumer never settles on stale data by accident. `latest(id)` returns the value without the age check.

Rust: send a `CallContract` with calldata `{"": "latest_fresh", "args": [id, max_age_s]}` (see `glkit`).

## Methods

| Method | Kind | Description |
|---|---|---|
| `create_feed(id, unit, description, decimals, tol_bps, min_sources, max_spread_bps, sources)` | write | New feed. `sources` is a list of `{name, url, path, invert?}`: 2 to 7 HTTPS JSON endpoints, `path` like `data.amount` or `result.XXBTZUSD.c.0`, `invert` for rates quoted the other way round. The caller becomes the feed owner. |
| `update(id)` | write, nondet | Fetch, agree, store a new round. Anyone can call it. |
| `set_sources(id, sources)` | write | Owner only. Replace the source list (same validation). |
| `latest(id)` / `latest_fresh(id, max_age_s)` | view | `{feed, value, decimals, text, unit, at, round, kept, sources, updater}` |
| `feed(id)` | view | Config plus latest round. |
| `history(id)` | view | Up to 48 recent rounds `{value, at, round, kept}`. |
| `feeds()` | view | All feeds with their latest value. |

Limits enforced by the contract: `decimals` ≤ 18, `1 ≤ tol_bps ≤ 1000`, `tol_bps ≤ max_spread_bps ≤ 5000`, `2 ≤ min_sources ≤ len(sources)`, URLs must be `https://`.

## Deployment

| | |
|---|---|
| Network | GenLayer Bradbury testnet (chain 4221) |
| Contract | [`0x0aa3eefa8769c5a3c16a65eff8ef0bcdd4a3a3ba`](https://scan.bradbury.genlayer.com/address/0x0aa3eefa8769c5a3c16a65eff8ef0bcdd4a3a3ba) |
| Code | `dist/quorum-feeds.wasm`, 257,662 bytes, SHA-256 `8163bf68aa1a6275e86d896f689b126298ee4733a63171431691d0cddf47593d` |
| Activation tx | [`0x5f8dc3b1c7e09917…`](https://scan.bradbury.genlayer.com/tx/0x5f8dc3b1c7e0991742aafced19323ab942e79992272a786722413b4d8be74396) |

`tools/build.sh` rebuilds the same bytes (Rust 1.98.1, binaryen version_133) so the SHA-256 can be compared with the code the chain runs, for example with the verifier at https://brossxnft.github.io/genlayer-chunked-deploy/.

Transactions that exercise the contract on Bradbury (all finalized with 5 validators unless noted):

| What | GenLayer tx |
|---|---|
| `create_feed` BTC-USD (4 exchanges, tol 50 bps, min 3, spread 200 bps) | [`0x2c32079d0a3c58e2…`](https://scan.bradbury.genlayer.com/tx/0x2c32079d0a3c58e25b3ba1df2587e1ef6a4eb32de3d4711a6cd7db82748f1ba2) |
| `create_feed` ETH-USD | [`0x7216d93370398e77…`](https://scan.bradbury.genlayer.com/tx/0x7216d93370398e77e3a6bc7a6fa165298b81a729dbc64477e10d7fbd3fb26a6f) |
| `create_feed` EUR-USD (ECB reference rate + 2 market sources) | [`0xfb78b752fe271cc3…`](https://scan.bradbury.genlayer.com/tx/0xfb78b752fe271cc348249937b5c23452de1e56f0c166aca1e1dc4cfe5f34d65f) |
| `update` BTC-USD: 4/4 sources kept, 71,921.35 USD, 5/5 agree | [`0x9b99472fb6f70e04…`](https://scan.bradbury.genlayer.com/tx/0x9b99472fb6f70e043b961320867b3d289667fdb870afb8559de9fa3fee71416e) |
| `update` ETH-USD: 4/4 sources kept, 2,019.96 USD, 5/5 agree | [`0x1d5dd5e87b71627c…`](https://scan.bradbury.genlayer.com/tx/0x1d5dd5e87b71627c3e9eca46b94da0aa13173ff8411d814c8fba063d18f1b0c4) |
| `update` EUR-USD: 3/3 sources kept, 1.1081 USD, 5/5 agree | [`0x4ea95045f3d42498…`](https://scan.bradbury.genlayer.com/tx/0x4ea95045f3d42498bba2cbdd1f760671d7b91118b53dd866bba75907800f752e) |

## Build and deploy

```bash
# Rust stable with the wasm32-wasip1 target, binaryen's wasm-opt
cargo build --release --target wasm32-wasip1
wasm-opt -Oz --enable-bulk-memory --enable-sign-ext --enable-mutable-globals \
  --enable-nontrapping-float-to-int target/wasm32-wasip1/release/quorum-feeds.wasm -o quorum-feeds.wasm
python tools/wasm_strip.py quorum-feeds.wasm quorum-feeds.wasm --version v0.3.0

# Rust contracts are larger than one Bradbury transaction; deploy in chunks:
python tools/chunked_deploy.py quorum-feeds.wasm --test-method version --send
python scripts/seed_feeds.py <address> --update --send
```

`cargo test` runs the unit tests of the decimal parser, median, tolerance and JSON path helpers (`glkit`).

## Layout

- `contract/src/main.rs`: the contract
- `glkit/`: small helpers over the raw `genlayer_sdk` host calls (web requests, nondet blocks, integer decimals, records in storage)
- `examples/`: feed definitions and the Python consumer
- `scripts/`, `tools/`: seeding, building and chunked deployment

## License

MIT
