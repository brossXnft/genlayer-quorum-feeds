//! Quorum Feeds: multi-source numeric feeds for GenLayer, written in Rust.
//!
//! Anyone defines a feed: 2-7 public HTTPS JSON sources, a JSON path per
//! source, a decimal scale and three consensus parameters. Anyone can push an
//! update. The leader fetches every source, drops outliers, and proposes the
//! median. Each validator fetches the same sources itself and accepts only if
//! its own median is within `tol_bps` of the leader's. Other contracts (Rust or
//! Python) read the result with `latest` / `latest_fresh`.
//!
//! Numbers are integers scaled by 10^decimals end to end: no floating point.

use glkit::genlayer_sdk::abi::entry::MessageData;
use glkit::genlayer_sdk::abi::entry::contract_def::{ConsensusStageData, Contract, ContractExt};
use glkit::genlayer_sdk::storage::{DynArray, Root};
use glkit::*;

glkit::genlayer_sdk::record!(State {
    feeds: Records,
    latest: Records,
    history: Records,
    ids: DynArray<String>,
});

fn state() -> State {
    Root::<State>::get().contract_instance().get()
}

const MAX_SOURCES: usize = 7;
const MAX_DECIMALS: u32 = 18;
const HISTORY: usize = 48;
const UA: &str = "quorum-feeds/1.0 (+https://github.com/brossXnft/genlayer-quorum-feeds)";

// ------------------------------------------------------------------ config

struct Source {
    name: String,
    url: String,
    path: String,
    invert: bool,
}

struct Feed {
    decimals: u32,
    tol_bps: u32,
    min_sources: usize,
    max_spread_bps: u32,
    sources: Vec<Source>,
}

fn parse_sources(v: &Value) -> R<Vec<Source>> {
    let arr = v.as_array().ok_or("sources must be a list")?;
    let mut out = Vec::new();
    for (i, s) in arr.iter().enumerate() {
        let src = Source {
            name: f_str(s, "name"),
            url: f_str(s, "url"),
            path: f_str(s, "path"),
            invert: f_bool(s, "invert"),
        };
        check_id(&src.name, 24, &format!("source #{i} name"))?;
        if !src.url.starts_with("https://") || src.url.len() > 300 {
            return Err(format!("source `{}`: url must be https:// and at most 300 chars", src.name));
        }
        if src.path.len() > 120 {
            return Err(format!("source `{}`: path too long", src.name));
        }
        if out.iter().any(|o: &Source| o.name == src.name) {
            return Err(format!("duplicate source name `{}`", src.name));
        }
        out.push(src);
    }
    if out.len() < 2 || out.len() > MAX_SOURCES {
        return Err(format!("a feed needs 2-{MAX_SOURCES} sources"));
    }
    Ok(out)
}

fn feed_from(cfg: &Value) -> R<Feed> {
    Ok(Feed {
        decimals: f_u64(cfg, "decimals") as u32,
        tol_bps: f_u64(cfg, "tol_bps") as u32,
        min_sources: f_u64(cfg, "min_sources") as usize,
        max_spread_bps: f_u64(cfg, "max_spread_bps") as u32,
        sources: parse_sources(f(cfg, "sources").ok_or("feed has no sources")?)?,
    })
}

fn validate(feed: &Feed) -> R<()> {
    if feed.decimals > MAX_DECIMALS {
        return Err(format!("decimals must be 0-{MAX_DECIMALS}"));
    }
    if feed.tol_bps == 0 || feed.tol_bps > 1000 {
        return Err("tol_bps must be 1-1000".into());
    }
    if feed.max_spread_bps < feed.tol_bps || feed.max_spread_bps > 5000 {
        return Err("max_spread_bps must be between tol_bps and 5000".into());
    }
    if feed.min_sources < 2 || feed.min_sources > feed.sources.len() {
        return Err("min_sources must be at least 2 and at most the number of sources".into());
    }
    Ok(())
}

// ------------------------------------------------------------------ observation (nondet only)

struct Obs {
    value: Option<BigInt>,
    kept: usize,
    report: Vec<Value>,
}

fn observe(feed: &Feed) -> Obs {
    let mut got: Vec<(usize, BigInt)> = Vec::new();
    let mut report: Vec<Value> = Vec::new();
    for (i, s) in feed.sources.iter().enumerate() {
        let r = get_json(&s.url, &[("User-Agent", UA), ("Accept", "application/json")])
            .and_then(|j| json_decimal_text(&j, &s.path).ok_or_else(|| format!("no number at `{}`", s.path)))
            .and_then(|t| parse_scaled(&t, feed.decimals).ok_or_else(|| format!("not a number: {t}")))
            .and_then(|v| {
                if v.sign() != Sign::Plus {
                    return Err("value must be positive".into());
                }
                if s.invert {
                    // 1/v at the same scale: 10^(2d) / v
                    Ok(BigInt::from(10u8).pow(2 * feed.decimals) / v)
                } else {
                    Ok(v)
                }
            });
        match r {
            Ok(v) => {
                report.push(map([("source", s.name.clone().into()), ("value", Value::Number(v.clone()))]));
                got.push((i, v));
            }
            Err(e) => report.push(map([("source", s.name.clone().into()), ("error", Value::Str(e))])),
        }
    }
    let Some(m0) = median(got.iter().map(|(_, v)| v.clone()).collect()) else {
        return Obs { value: None, kept: 0, report };
    };
    // Drop sources further than max_spread_bps from the raw median, then take the median again.
    let kept: Vec<BigInt> = got
        .iter()
        .filter(|(_, v)| within_bps(v, &m0, feed.max_spread_bps))
        .map(|(_, v)| v.clone())
        .collect();
    for (i, v) in &got {
        if !within_bps(v, &m0, feed.max_spread_bps) {
            let name = &feed.sources[*i].name;
            for r in report.iter_mut() {
                if f_str(r, "source") == *name {
                    *r = with(r, "outlier", Value::Bool(true));
                }
            }
        }
    }
    let n = kept.len();
    Obs { value: if n >= feed.min_sources { median(kept) } else { None }, kept: n, report }
}

// ------------------------------------------------------------------ views

fn round_view(id: &str, cfg: &Value, round: &Value) -> Value {
    let d = f_u64(cfg, "decimals") as u32;
    let v = f_num(round, "value");
    map([
        ("feed", id.into()),
        ("value", Value::Number(v.clone())),
        ("decimals", Value::from(d as u64)),
        ("text", Value::Str(format_scaled(&v, d))),
        ("unit", Value::Str(f_str(cfg, "unit"))),
        ("at", Value::Number(f_num(round, "at"))),
        ("round", Value::Number(f_num(round, "round"))),
        ("kept", Value::Number(f_num(round, "kept"))),
        ("sources", f(round, "sources").cloned().unwrap_or(Value::Array(vec![]))),
        ("updater", f(round, "updater").cloned().unwrap_or(Value::Null)),
    ])
}

fn public_cfg(id: &str, cfg: &Value) -> Value {
    with(cfg, "id", id.into())
}

#[derive(Default)]
pub struct QuorumFeeds;

impl QuorumFeeds {
    fn create(&self, m: &MessageData, args: &[Value]) -> R<Value> {
        no_value(m)?;
        let st = state();
        let id = arg_str(args, 0, "id")?;
        check_id(&id, 32, "feed id")?;
        if st.feeds().contains_key(&id) {
            return Err(format!("feed `{id}` already exists"));
        }
        let cfg = map([
            ("owner", Value::Address(m.sender_address)),
            ("unit", Value::Str(arg_str(args, 1, "unit")?.chars().take(24).collect())),
            ("description", Value::Str(arg_str(args, 2, "description")?.chars().take(200).collect())),
            ("decimals", Value::from(arg_u64(args, 3, "decimals")?)),
            ("tol_bps", Value::from(arg_u64(args, 4, "tol_bps")?)),
            ("min_sources", Value::from(arg_u64(args, 5, "min_sources")?)),
            ("max_spread_bps", Value::from(arg_u64(args, 6, "max_spread_bps")?)),
            ("sources", arg(args, 7, "sources")?.clone()),
            ("created_at", Value::from(tx_time(m))),
        ]);
        validate(&feed_from(&cfg)?)?;
        rec_put(&st.feeds(), &id, &cfg);
        st.ids().append_slot().store(&id);
        Ok(public_cfg(&id, &cfg))
    }

    fn update(&self, m: &MessageData, args: &[Value]) -> R<Value> {
        no_value(m)?;
        let st = state();
        let id = arg_str(args, 0, "id")?;
        let cfg = rec_get(&st.feeds(), &id).ok_or_else(|| format!("unknown feed `{id}`"))?;
        let feed = feed_from(&cfg)?;
        let res = run_nondet(&map([("op", "observe".into()), ("id", id.clone().into())]))?;
        // Deterministic re-check of what consensus accepted.
        let value = f_num(&res, "value");
        let kept = f_u64(&res, "kept") as usize;
        if value.sign() != Sign::Plus || kept < feed.min_sources {
            return Err("consensus result is below the feed's quorum".into());
        }
        let prev = rec_get(&st.latest(), &id);
        let round_no = prev.as_ref().map(|p| f_u64(p, "round")).unwrap_or(0) + 1;
        let round = map([
            ("value", Value::Number(value)),
            ("kept", Value::from(kept as u64)),
            ("sources", f(&res, "sources").cloned().unwrap_or(Value::Array(vec![]))),
            ("at", Value::from(tx_time(m))),
            ("round", Value::from(round_no)),
            ("updater", Value::Address(m.sender_address)),
        ]);
        rec_put(&st.latest(), &id, &round);
        let mut hist = rec_get(&st.history(), &id).and_then(|h| h.as_array().map(|a| a.to_vec())).unwrap_or_default();
        hist.push(map([
            ("value", Value::Number(f_num(&round, "value"))),
            ("at", Value::Number(f_num(&round, "at"))),
            ("round", Value::from(round_no)),
            ("kept", Value::from(kept as u64)),
        ]));
        if hist.len() > HISTORY {
            hist.drain(..hist.len() - HISTORY);
        }
        rec_put(&st.history(), &id, &Value::Array(hist));
        Ok(round_view(&id, &cfg, &round))
    }

    fn latest(&self, m: &MessageData, args: &[Value], max_age: Option<u64>) -> R<Value> {
        let st = state();
        let id = arg_str(args, 0, "id")?;
        let cfg = rec_get(&st.feeds(), &id).ok_or_else(|| format!("unknown feed `{id}`"))?;
        let round = rec_get(&st.latest(), &id).ok_or_else(|| format!("feed `{id}` has no value yet"))?;
        if let Some(max_age) = max_age {
            let age = tx_time(m) - f_i64(&round, "at");
            if age > max_age as i64 {
                return Err(format!("feed `{id}` is stale: last update {age}s ago, max {max_age}s"));
            }
        }
        Ok(round_view(&id, &cfg, &round))
    }
}

impl ContractExt for QuorumFeeds {
    type Error = String;

    fn handle_deploy(&mut self, _m: MessageData, _a: Vec<Value>, _k: calldata::Map) -> R<Value> {
        Ok(Value::Null)
    }

    fn handle_method(&mut self, m: MessageData, method: String, args: Vec<Value>, _k: calldata::Map) -> R<Value> {
        match method.as_str() {
            // writes
            "create_feed" => self.create(&m, &args),
            "update" => self.update(&m, &args),
            "set_sources" => {
                no_value(&m)?;
                let st = state();
                let id = arg_str(&args, 0, "id")?;
                let cfg = rec_get(&st.feeds(), &id).ok_or_else(|| format!("unknown feed `{id}`"))?;
                if f_addr(&cfg, "owner").map(|a| a.raw()) != Some(m.sender_address.raw()) {
                    return Err("only the feed owner can change its sources".into());
                }
                let cfg = with(&cfg, "sources", arg(&args, 1, "sources")?.clone());
                validate(&feed_from(&cfg)?)?;
                rec_put(&st.feeds(), &id, &cfg);
                Ok(public_cfg(&id, &cfg))
            }
            // views
            "latest" => self.latest(&m, &args, None),
            "latest_fresh" => {
                let max_age = arg_u64(&args, 1, "max_age_s")?;
                self.latest(&m, &args, Some(max_age))
            }
            "feed" => {
                let st = state();
                let id = arg_str(&args, 0, "id")?;
                let cfg = rec_get(&st.feeds(), &id).ok_or_else(|| format!("unknown feed `{id}`"))?;
                let latest = rec_get(&st.latest(), &id).map(|r| round_view(&id, &cfg, &r)).unwrap_or(Value::Null);
                Ok(map([("config", public_cfg(&id, &cfg)), ("latest", latest)]))
            }
            "history" => {
                let st = state();
                let id = arg_str(&args, 0, "id")?;
                Ok(rec_get(&st.history(), &id).unwrap_or(Value::Array(vec![])))
            }
            "feeds" => {
                let st = state();
                let ids = st.ids();
                let mut out = Vec::new();
                for i in 0..ids.len() {
                    let id = ids.index(i).load();
                    let Some(cfg) = rec_get(&st.feeds(), &id) else { continue };
                    let latest = rec_get(&st.latest(), &id).map(|r| round_view(&id, &cfg, &r)).unwrap_or(Value::Null);
                    out.push(map([
                        ("id", id.clone().into()),
                        ("unit", Value::Str(f_str(&cfg, "unit"))),
                        ("description", Value::Str(f_str(&cfg, "description"))),
                        ("sources", Value::from(f(&cfg, "sources").and_then(|s| s.as_array()).map(|a| a.len()).unwrap_or(0) as u64)),
                        ("latest", latest),
                    ]));
                }
                Ok(Value::Array(out))
            }
            "version" => Ok("quorum-feeds 1.0.0".into()),
            _ => Err(format!("unknown method: {method}")),
        }
    }
}

impl Contract for QuorumFeeds {
    fn handle_main(&mut self, message: MessageData, data: bytes::Bytes) -> Result<Value, String> {
        ContractExt::handle_main(self, message, data.to_vec())
    }

    fn handle_sandbox(&mut self, _m: MessageData, _d: bytes::Bytes) -> Result<Vec<u8>, String> {
        Err("sandbox not supported".into())
    }

    fn handle_consensus_stage(&mut self, _m: MessageData, data: bytes::Bytes, stage: ConsensusStageData) -> Result<Value, String> {
        let req = calldata::decode(&data).map_err(|e| format!("{e:?}"))?;
        if f_str(&req, "op") != "observe" {
            return Err("unknown nondet op".into());
        }
        let id = f_str(&req, "id");
        let cfg = rec_get(&state().feeds(), &id).ok_or("unknown feed")?;
        let feed = feed_from(&cfg)?;
        let mine = observe(&feed);
        match Stage::from(stage) {
            Stage::Leader => match mine.value {
                Some(v) => Ok(map([
                    ("value", Value::Number(v)),
                    ("kept", Value::from(mine.kept as u64)),
                    ("sources", Value::Array(mine.report)),
                ])),
                None => Err(format!(
                    "only {} of {} sources agreed (need {}): {}",
                    mine.kept,
                    feed.sources.len(),
                    feed.min_sources,
                    short(&Value::Array(mine.report))
                )),
            },
            Stage::Validator(lr) => {
                // Agree with a leader error only if we also cannot reach quorum.
                if leader_user_error(&lr).is_some() {
                    return Ok(Value::Bool(mine.value.is_none()));
                }
                let Some(l) = leader_value(&lr) else { return Ok(Value::Bool(false)) };
                let lv = f_num(l, "value");
                let lk = f_u64(l, "kept") as usize;
                Ok(Value::Bool(match mine.value {
                    Some(v) => lk >= feed.min_sources && lv.sign() == Sign::Plus && within_bps(&lv, &v, feed.tol_bps),
                    None => false,
                }))
            }
        }
    }
}

glkit::genlayer_sdk::contract_main!(QuorumFeeds);
