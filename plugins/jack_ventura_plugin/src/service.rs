//! Package-owned service protocol and workflow. The host sees only capability
//! requests and opaque checkpoints; no service DTO crosses into host policy.
use super::*;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const ORIGIN: &str = "https://open-api.bingx.com";
const UID: &str = "/openApi/account/v1/uid";
const ORDER: &str = "/openApi/swap/v2/trade/order";
const POSITIONS: &str = "/openApi/swap/v2/user/positions";
const HISTORY: &str = "/openApi/swap/v2/trade/allOrders";
const FILLS: &str = "/openApi/swap/v2/trade/allFillOrders";
const OPEN: &str = "/openApi/swap/v2/trade/openOrders";
const LEVERAGE: &str = "/openApi/swap/v2/trade/leverage";
const MODE: &str = "/openApi/swap/v1/positionSide/dual";
const CONTRACT: &str = "/openApi/swap/v2/quote/contracts";
const BALANCE: &str = "/openApi/swap/v3/user/balance";
const CANDLE_PAGE: i64 = 96;
const CANDLES: &str = "/openApi/swap/v3/quote/klines";

type R<T> = Result<T, String>;

#[derive(Serialize, Deserialize)]
struct StopCoverage {
    summary: String,
    details: String,
    trigger: Option<f64>,
}

#[derive(Serialize, Deserialize)]
struct Checkpoint {
    version: u32,
    engine: State,
    #[serde(default)]
    intents: BTreeMap<String, String>,
    #[serde(default)]
    new_entries: bool,
    // One owned-trade locator, not a fill/history archive. Service observations
    // are reacquired on open/refresh and never become host-owned account truth.
    #[serde(default)]
    last_trade: Option<Value>,
}

impl Default for Checkpoint {
    fn default() -> Self {
        Self {
            version: 2,
            engine: State::new(Settings::default()),
            intents: BTreeMap::new(),
            new_entries: true,
            last_trade: None,
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Work {
    #[serde(skip)]
    view: Option<Value>,
    #[serde(skip)]
    pending: Option<Value>,
    #[serde(skip)]
    dirty: bool,
    queue: Vec<Value>,
    resume: Option<String>,
    job: Option<Value>,
    #[serde(default)]
    rows: Vec<Value>,
    #[serde(default)]
    verified: Option<String>,
    #[serde(default)]
    account: Value,
    #[serde(default)]
    lifecycle: Value,
    // Service observation for this action only; never part of the checkpoint.
    #[serde(default)]
    protection: Option<(PositionObservation, i64, StopCoverage)>,
    #[serde(default)]
    page: usize,
    #[serde(default)]
    candle_wait: bool,
    #[serde(default)]
    read_retries: u8,
    #[serde(default)]
    read_retry_wait: bool,
    #[serde(default)]
    sequence: usize,
    #[serde(default)]
    autonomous: bool,
    #[serde(default)]
    running: bool,
    #[serde(default)]
    orders_view: bool,
    #[serde(default)]
    trade_result: Value,
}

fn hash(value: &Value) -> String {
    format!("{:x}", Sha256::digest(serde_json::to_vec(value).unwrap()))
}

fn number(value: &Value) -> R<f64> {
    let n = value
        .as_f64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
        .ok_or("Invalid service number")?;
    if !n.is_finite() || n < 0.0 {
        return Err("Invalid service number".into());
    }
    Ok(n)
}
fn text(value: &Value) -> R<String> {
    match value {
        Value::String(s) if !s.is_empty() => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        _ => Err("Missing service identity".into()),
    }
}
fn account_id(uid: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!("bingx:LIVE:{uid}").as_bytes())
    )
}
fn client_id(c: &Checkpoint, p: &Value, exit: bool) -> String {
    // A new session keeps its persisted client identity after ordinary restart.
    let managed = c.engine.entry.as_ref();
    let evidence = if exit {
        managed
            .and_then(|e| e.profit_exit.as_ref())
            .filter(|e| serde_json::to_value(&e.plan).unwrap() == *p)
            .and_then(|e| e.evidence.as_ref())
    } else {
        managed
            .filter(|e| serde_json::to_value(&e.plan).unwrap() == *p)
            .and_then(|e| e.evidence.as_ref())
    };
    if let Some(evidence) = evidence {
        return evidence.client_order_id.clone();
    }
    let full = if exit {
        hash(&json!([
            "position.exit.place",
            entry_id(&p["entry_plan"]),
            p["position_id"],
            p["quantity"],
            p["average_price"],
            p["price"],
            p["prepared_at_ms"],
            p["expires_at_ms"]
        ]))
    } else {
        entry_id(p)
    };
    full[..40].into()
}
fn entry_id(p: &Value) -> String {
    hash(&json!([
        p["account_id"],
        p["symbol"],
        p["side"],
        p["timeframe"],
        p["origin"],
        p["first_known"],
        p["line_price"],
        p["price"],
        p["quantity"],
        p["stop_price"],
        p["margin"],
        p["leverage"],
        p["prepared_at_ms"],
        p["expires_at_ms"]
    ]))
}

pub(super) fn market_access() -> Value {
    json!({"version":2,"network":[{"origin":ORIGIN,"methods":["GET"],"credential":null}],"credentials":{}})
}
fn access(write: bool) -> Value {
    let mut access = market_access();
    access["network"].as_array_mut().unwrap().push(json!({"origin":ORIGIN,"methods":if write {json!(["GET","POST","DELETE"])} else {json!(["GET"])},"credential":"default"}));
    access["credentials"] = json!({"default":[{"id":"api_key","label":"API key"},{"id":"secret_key","label":"Secret key"}]});
    access
}
fn uri(path: &str, params: &BTreeMap<String, String>) -> String {
    fn encode(s: &str) -> String {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                    (b as char).to_string()
                }
                _ => format!("%{b:02X}"),
            })
            .collect()
    }
    let query = params
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{ORIGIN}{path}?{query}")
}
fn http(
    w: &mut Work,
    path: &str,
    method: &str,
    mut params: BTreeMap<String, String>,
    signed: bool,
    now: i64,
) -> Value {
    w.sequence += 1;
    params.insert("timestamp".into(), now.to_string());
    let mut r = json!({"kind":"http.request","id":format!("request-{}",w.sequence),"continuation":"service_result",
        "url":uri(path,&params),"method":method});
    if signed {
        r["credential"] = json!("default");
        r["auth"] = json!({"headers":{"X-BX-APIKEY":{"field":"api_key"}},"hmac_query":{"parameter":"signature","field":"secret_key"}});
    }
    r
}
fn params(items: &[(&str, String)]) -> BTreeMap<String, String> {
    items
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

fn engine(
    c: &mut Checkpoint,
    w: &mut Work,
    action: &str,
    snapshot: Option<Value>,
    settings: Option<Value>,
    now: i64,
) -> R<Value> {
    // Call the private strategy directly. Encoding a second invocation envelope
    // and decoding a second result consumed the sandbox budget for no behavior.
    let final_receipt = snapshot.as_ref().and_then(|v| v.get("entry")).cloned();
    let snapshot = snapshot.map(|v| serde_json::value::to_raw_value(&v).unwrap());
    let execution = c
        .engine
        .account
        .as_ref()
        .filter(|_| w.running)
        .map(|account| ExecutionProjection {
            account_id: account.account_id.clone(),
            symbol: c.engine.settings.symbol.clone(),
            allow_new_entries: c.new_entries,
        });
    let previous_entry = if action == "lifecycle" {
        c.engine.entry.clone()
    } else {
        None
    };
    let previous_exit = c
        .engine
        .entry
        .as_ref()
        .and_then(|e| e.profit_exit.as_ref())
        .map(|e| serde_json::to_value(&e.plan).unwrap());
    let scope = serde_json::value::to_raw_value(
        &json!({"version":1,"settings":c.engine.settings,"account":c.engine.account}),
    )
    .unwrap();
    let input = Input {
        schema_version: 1,
        plugin_id: PLUGIN_ID.into(),
        host_method: "workspace".into(),
        action: action.into(),
        observed_at_ms: now,
        state: if matches!(action, "list_orders" | "open_orders") {
            Some(&scope)
        } else {
            None
        },
        snapshot: snapshot.as_deref(),
        settings: settings
            .map(serde_json::from_value)
            .transpose()
            .map_err(|_| "Invalid settings")?,
        execution,
    };
    let output = if matches!(action, "list_orders" | "open_orders") {
        evaluate(input)?
    } else {
        evaluate_state(input, c.engine.clone())?
    };
    if let SavedState::Updated(state) = output.state {
        c.engine = state;
        if !matches!(action, "open" | "present" | "validate_exit") {
            w.dirty = true;
        }
    }
    w.queue.extend(output.requests);
    if let Some(resume) = output.resume_action {
        w.resume = Some(resume.into());
    }
    w.orders_view = matches!(&output.view, WorkspaceView::Orders(_));
    let view = match output.view {
        WorkspaceView::Trading(view) => view,
        WorkspaceView::Orders(view) => serde_json::to_value(view).unwrap(),
    };
    w.view = Some(view.clone());
    if output.retired_entry.is_some() {
        // All exact managed orders were terminal and the position was flat.
        // A retired plan no longer needs a private duplicate-send checkpoint.
        let plan = serde_json::to_value(output.retired_entry.unwrap()).unwrap();
        if let Some(entry) = previous_entry {
            let receipt: OrderEvidence = match final_receipt {
                Some(raw) => {
                    serde_json::from_value(raw).map_err(|_| "Invalid final entry receipt")?
                }
                None => entry
                    .evidence
                    .clone()
                    .ok_or("Missing final entry receipt")?,
            };
            if receipt.filled_quantity > 0.0 {
                c.last_trade = Some(
                    json!({"account_id":plan["account_id"],"symbol":plan["symbol"],
                "side":plan["side"],"from":plan["prepared_at_ms"],"closed_at":now,
                "entry_id":receipt.order_id,"client_id":receipt.client_order_id,
                "quantity":receipt.filled_quantity,"position_id":entry.observed_position_id,
                "tp_id":entry.profit_exit.as_ref().and_then(|e| e.evidence.as_ref()).and_then(|e| e.order_id.as_ref())}),
                );
                w.queue.retain(|j| j["kind"] != "trade.result.read");
                queue_trade_result(c, w);
            }
        }
        w.dirty = true;
        c.intents.remove(&entry_id(&plan));
        let prefix = format!("cancel:{}:", entry_id(&plan));
        c.intents.retain(|key, _| !key.starts_with(&prefix));
        if let Some(previous_exit) = previous_exit {
            c.intents
                .remove(&format!("exit:{}", client_id(c, &previous_exit, true)));
        }
    }
    Ok(view)
}

fn queue_trade_result(c: &Checkpoint, w: &mut Work) {
    if let Some(trade) = c.last_trade.as_ref().filter(|trade| {
        c.engine
            .account
            .as_ref()
            .is_some_and(|a| trade["account_id"] == a.account_id)
            && trade["symbol"] == c.engine.settings.symbol
    }) {
        w.trade_result = Value::Null;
        w.queue.insert(
            0,
            json!({"kind":"trade.result.read","symbol":trade["symbol"],"trade":trade}),
        );
    }
}

fn signed_number(value: &Value) -> R<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|v| v.parse().ok()))
        .filter(|v| v.is_finite())
        .ok_or_else(|| "Invalid trade accounting number".into())
}

fn fill_time(value: &Value) -> R<i64> {
    // allFillOrders supplies UTC filledTm; filledTime is its local-time display.
    let raw = value.as_str().ok_or("Missing fill time")?;
    let b = raw.as_bytes();
    if b.len() != 24
        || [4, 7, 10, 13, 16, 19, 23]
            .into_iter()
            .zip(b"--T::.Z")
            .any(|(i, separator)| b[i] != *separator)
    {
        return Err("Invalid UTC fill time".into());
    }
    let part = |start: usize, end: usize| -> R<i64> {
        if !b[start..end].iter().all(u8::is_ascii_digit) {
            return Err("Invalid UTC fill time".into());
        }
        raw[start..end]
            .parse()
            .map_err(|_| "Invalid UTC fill time".into())
    };
    let (year, month, day) = (part(0, 4)?, part(5, 7)?, part(8, 10)?);
    let (hour, minute, second, millis) =
        (part(11, 13)?, part(14, 16)?, part(17, 19)?, part(20, 23)?);
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let months = [
        31_i64,
        28 + i64::from(leap),
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if !(1970..=9999).contains(&year)
        || !(1..=12).contains(&month)
        || !(1..=months[(month - 1) as usize]).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return Err("Invalid UTC fill time".into());
    }
    let days = 365 * (year - 1970) + (year - 1) / 4 - 1969 / 4 - (year - 1) / 100
        + 1969 / 100
        + (year - 1) / 400
        - 1969 / 400
        + months[..(month - 1) as usize].iter().sum::<i64>()
        + day
        - 1;
    Ok((((days * 24 + hour) * 60 + minute) * 60 + second) * 1000 + millis)
}

fn position_id(row: &Value) -> Option<String> {
    row.get("positionId")
        .or_else(|| row.get("positionID"))
        .and_then(|v| text(v).ok())
        .filter(|v| v != "0")
}

fn unavailable_trade_result(w: &mut Work, error: &str) -> bool {
    if w.pending
        .as_ref()
        .is_none_or(|j| j["kind"] != "trade.result.read")
    {
        return false;
    }
    // Failure to reconstruct a completed trade must not strand management of
    // the current position or prevent fresh entries under the owner's Start.
    w.trade_result = json!({"confirmed":false,"summary":"Result not confirmed — exchange evidence unavailable or incomplete",
        "details":error});
    w.pending = None;
    w.job = None;
    true
}

fn receive_trade_result(
    _c: &mut Checkpoint,
    w: &mut Work,
    mut j: Value,
    raw: &Value,
    now: i64,
) -> R<()> {
    let t = j["trade"].clone();
    let opening_side = if t["side"] == "long" { "BUY" } else { "SELL" };
    let closing_side = if opening_side == "BUY" { "SELL" } else { "BUY" };
    match j["stage"].as_str().ok_or("Missing result stage")? {
        "uid" => {
            let uid = text(&raw["uid"])?;
            if uid.len() > 30
                || !uid.bytes().all(|b| b.is_ascii_digit())
                || t["account_id"] != account_id(&uid)
            {
                return Err("Last trade belongs to another exchange account".into());
            }
            j["stage"] = json!("result_order");
        }
        "result_order" => {
            let r = raw.get("order").unwrap_or(raw);
            if text(r.get("orderId").unwrap_or(&r["orderID"]))? != text(&t["entry_id"])?
                || r["clientOrderId"] != t["client_id"]
                || r["symbol"] != t["symbol"]
                || r["side"] != opening_side
                || number(&r["executedQty"])? != number(&t["quantity"])?
                || !matches!(
                    r["status"].as_str(),
                    Some("FILLED" | "CANCELED" | "CANCELLED" | "EXPIRED")
                )
            {
                return Err("Last trade entry identity or final quantity is unverified".into());
            }
            let observed = t["position_id"].as_str().map(str::to_owned);
            let actual = position_id(r);
            if observed.is_some() && actual.is_some() && observed != actual {
                return Err("Last trade position identity changed".into());
            }
            j["position_id"] = json!(observed.or(actual));
            j["orders"] = json!([{ "id":t["entry_id"],"qty":t["quantity"],"side":opening_side,"kind":"entry" }]);
            j["stage"] = json!("result_history");
        }
        "result_history" => {
            let rows = raw
                .get("orders")
                .unwrap_or(raw)
                .as_array()
                .ok_or("Invalid trade order history")?;
            if rows.len() >= 100 {
                return Err(
                    "Last trade result incomplete: order history reached its bounded limit".into(),
                );
            }
            let mut ids = std::collections::BTreeSet::new();
            for r in rows {
                if r["symbol"] != t["symbol"] || r["side"] != closing_side {
                    continue;
                }
                let id = text(r.get("orderId").unwrap_or(&r["orderID"]))?;
                let tp = t["tp_id"].as_str() == Some(id.as_str());
                let matched_position = position_id(r).is_some_and(|p| j["position_id"] == p);
                if tp
                    && position_id(r).is_some()
                    && !j["position_id"].is_null()
                    && !matched_position
                {
                    return Err("Profit exit belongs to a different position".into());
                }
                if !tp && !matched_position {
                    continue;
                }
                let hedge = if t["side"] == "long" { "LONG" } else { "SHORT" };
                if r["positionSide"] != hedge
                    && !(r["positionSide"] == "BOTH"
                        && matches!(r["reduceOnly"].as_bool(), Some(true)))
                {
                    return Err("Last trade exit does not prove reducing scope".into());
                }
                let qty = number(&r["executedQty"])?;
                if qty == 0.0 {
                    continue;
                }
                if !ids.insert(id.clone())
                    || !matches!(
                        r["status"].as_str(),
                        Some("FILLED" | "CANCELED" | "CANCELLED" | "EXPIRED")
                    )
                {
                    return Err("Last trade has duplicate or nonterminal reducing orders".into());
                }
                let kind = if tp && r["type"] == "LIMIT" {
                    "TP"
                } else if matched_position
                    && matches!(r["type"].as_str(), Some("STOP" | "STOP_MARKET"))
                {
                    "SL"
                } else {
                    "Other"
                };
                j["orders"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"id":id,"qty":qty,"side":closing_side,"kind":kind}));
            }
            if j["orders"].as_array().unwrap().len() > 16 {
                return Err("Too many closing orders for bounded trade result".into());
            }
            j["index"] = json!(0);
            j["totals"] = json!({"entry_qty":0.,"entry_cost":0.,"exit_qty":0.,"exit_cost":0.,"fee":0.,"TP":0.,"SL":0.,"Other":0.});
            j["stage"] = json!("result_fills");
        }
        "result_fills" => {
            let index = j["index"].as_u64().ok_or("Missing result order index")? as usize;
            let order = j["orders"][index].clone();
            let rows = raw
                .get("fill_orders")
                .unwrap_or(raw)
                .as_array()
                .ok_or("Invalid trade fills")?;
            if rows.len() > 1000 {
                return Err("Too many fills for bounded trade result".into());
            }
            let mut ids = std::collections::BTreeSet::new();
            let mut quantity = 0.;
            for r in rows {
                let at = fill_time(&r["filledTm"])?;
                if r["symbol"] != t["symbol"]
                    || text(&r["orderId"])? != text(&order["id"])?
                    || r["side"] != order["side"]
                    || at < t["from"].as_i64().unwrap()
                    || at > t["closed_at"].as_i64().unwrap()
                    || r["currency"] != "USDT"
                {
                    return Err(
                        "Fill identity, time or fee currency does not match the trade".into(),
                    );
                }
                let qty = number(&r["volume"])?;
                let price = number(&r["price"])?;
                let fee = signed_number(&r["commission"])?;
                if qty == 0. || price == 0. {
                    return Err("Empty trade fill".into());
                }
                // This endpoint has no fill ID. Indistinguishable rows cannot
                // prove distinct executions, so never count them twice.
                if !ids.insert(hash(&json!([at, qty, price, fee]))) {
                    return Err("Duplicate or indistinguishable trade fills".into());
                }
                quantity += qty;
                let prefix = if order["kind"] == "entry" {
                    "entry"
                } else {
                    "exit"
                };
                for (key, value) in [
                    (format!("{prefix}_qty"), qty),
                    (format!("{prefix}_cost"), qty * price),
                    ("fee".into(), fee),
                ] {
                    let total = signed_number(&j["totals"][&key])? + value;
                    if !total.is_finite() {
                        return Err("Trade result overflow".into());
                    }
                    j["totals"][key] = json!(total);
                }
                if let Some(kind) = order["kind"].as_str().filter(|s| *s != "entry") {
                    j["totals"][kind] = json!(number(&j["totals"][kind])? + qty);
                }
            }
            if !same_price(quantity, number(&order["qty"])?) {
                return Err("Last trade fills are incomplete; result not confirmed".into());
            }
            j["index"] = json!(index + 1);
            if index + 1 == j["orders"].as_array().unwrap().len() {
                let totals = &j["totals"];
                if !same_price(number(&totals["entry_qty"])?, number(&totals["exit_qty"])?) {
                    return Err(
                        "Last trade closing fills do not cover the entry; result not confirmed"
                            .into(),
                    );
                }
                j["stage"] = json!("result_positions");
            }
        }
        "result_positions" => {
            let positions = positions(raw, &t["symbol"])?;
            if positions
                .as_array()
                .unwrap()
                .iter()
                .any(|p| j["position_id"].is_null() || p["position_id"] == j["position_id"])
            {
                return Err(
                    "Last trade position remains open; full closure is not confirmed".into(),
                );
            }
            j["stage"] = json!("result_open");
        }
        "result_open" => {
            let rows = raw
                .get("orders")
                .unwrap_or(raw)
                .as_array()
                .ok_or("Invalid final open orders")?;
            if rows.iter().filter(|r| r["symbol"] == t["symbol"]).any(|r| {
                let id = text(r.get("orderId").unwrap_or(&r["orderID"])).ok();
                j["orders"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|o| id.as_ref().is_some_and(|id| o["id"] == *id))
                    || position_id(r).is_some_and(|p| j["position_id"] == p)
            }) {
                return Err(
                    "Last trade still has a matching open order; full closure is not confirmed"
                        .into(),
                );
            }
            let a = &j["totals"];
            let qty = number(&a["entry_qty"])?;
            // Linear USDT contracts: complete matched base-quantity executions
            // establish trading PnL without inventing a missing service field.
            let pnl = (number(&a["exit_cost"])? - number(&a["entry_cost"])?)
                * if t["side"] == "long" { 1.0 } else { -1.0 };
            let reason = if same_price(number(&a["TP"])?, qty) {
                "TP"
            } else if same_price(number(&a["SL"])?, qty) {
                "SL"
            } else {
                "Other / mixed exit"
            };
            w.trade_result = json!({"confirmed":true,"summary":format!("{} · {} {}\nAverage entry {} → exit {}\nRealized PnL {:+.6} USDT · fees/rebates {:+.6} USDT\nAfter trading fees {:+.6} USDT · funding excluded",
                reason,qty,text(&t["symbol"])?,number(&a["entry_cost"])?/qty,number(&a["exit_cost"])?/qty,
                pnl,signed_number(&a["fee"])?,pnl+signed_number(&a["fee"])?),
                "details":format!("Last trade: exchange fills read at {}. Position {}. Orders {}. Trading PnL calculated from matched linear USDT fills. Matching position and open orders absent; funding is not included.",now,
                    j["position_id"].as_str().unwrap_or("unavailable"),j["orders"].as_array().unwrap().iter().map(|o| text(&o["id"]).unwrap()).collect::<Vec<_>>().join(", "))});
            w.pending = None;
            return Ok(());
        }
        _ => return Err("Unknown trade-result stage".into()),
    }
    w.job = Some(j);
    w.pending = None;
    Ok(())
}

fn presentation(c: &mut Checkpoint, w: &mut Work, now: i64) -> R<Value> {
    // Presentation is pure: no host-selected private continuation.
    let mut idle = Work::default();
    idle.running = w.running;
    let mut view = match w.view.take() {
        Some(view) => view,
        None => engine(c, &mut idle, "present", None, None, now)?,
    };
    view["access"] = access(true);
    view["confirmation"] = Value::Null;
    if c.engine.account.is_some() {
        view["schedule"] = json!({"version":2,"action":"cycle","interval_seconds":60,"access":access(true),
            "description":"Run Jack Ventura with the settings selected at Start. Permit reading the account, placing and cancelling managed orders and reducing exits. The package enforces the selected settings; network permission is not a host financial-loss guarantee. Existing exchange orders remain after revocation."});
    }
    if w.orders_view {
        return Ok(view);
    }
    let evidence = view["message"].as_str().unwrap_or("").to_owned();
    let protection = c
        .engine
        .entry
        .as_ref()
        .and_then(|e| e.position.as_ref())
        .and_then(|position| {
            w.protection
                .as_ref()
                .filter(|(seen, at, _)| {
                    seen.position_id == position.position_id
                        && seen.side == position.side
                        && seen.quantity == position.quantity
                        && seen.average_price == position.average_price
                        && *at <= now
                        && now - *at <= 60_000
                })
                .map(|(_, _, observation)| observation)
        });
    let mut message = if let Some(entry) = c.engine.entry.as_ref() {
        if !entry.attempted {
            "Entry not sent\nStart Jack to calculate and manage entries from current market and account data.".into()
        } else if entry.not_sent() {
            "Entry not sent\nNo exchange order was created. Jack recalculates on the next running cycle.".into()
        } else if let Some(position) = entry.position.as_ref() {
            format!(
                "{} · {} {} {}\nAverage fill {}",
                if protection.is_some() {
                    "Position observed"
                } else {
                    "Saved position — refresh required"
                },
                position.side.to_uppercase(),
                position.quantity,
                entry.plan.symbol,
                position.average_price
            )
        } else if let Some(receipt) = entry.evidence.as_ref() {
            format!(
                "Entry {} · {} at {}\nFilled {} of {}. Refresh to observe the current position.",
                receipt.status,
                entry.plan.symbol,
                entry.plan.price,
                receipt.filled_quantity,
                entry.plan.quantity
            )
        } else {
            "Entry outcome unknown\nRefresh its exact identity; no repeat order is sent blindly."
                .into()
        }
    } else if c.engine.account.is_none() {
        "Connect an account to trade\nChoose your instrument, entry margin and stop budget before Start.".into()
    } else if w.running && c.new_entries {
        "Jack is running\nWaiting for an eligible entry. Orders and positions are managed automatically; no further entry confirmation is needed.".into()
    } else if c.engine.retired_before_ms > 0 {
        if w.trade_result["confirmed"] == true {
            "Last managed trade is closed.".into()
        } else {
            "Previous position/order is no longer active\nRead exchange executions to confirm its result.".into()
        }
    } else {
        "Ready to observe the market\nStart enables new entries within your saved settings and permissions.".into()
    };
    let mut stop_details = String::new();
    if let Some(position) = c.engine.entry.as_ref().and_then(|e| e.position.as_ref()) {
        message.push_str(&format!(
            "\nStop protection: {}",
            protection
                .map(|p| p.summary.as_str())
                .unwrap_or("Unverified — refresh the position and its stop orders.")
        ));
        if let Some(p) = protection {
            stop_details = p.details.clone();
            if let (Some(trigger), Some((target, _, _))) =
                (p.trigger, position_stop(&c.engine, position))
            {
                if (position.side == "long" && trigger < target)
                    || (position.side == "short" && trigger > target)
                {
                    message.push_str("\nStop trigger is beyond the fill-based budget estimate.");
                }
            }
        }
        message.push_str(
            &match c.engine.entry.as_ref().and_then(|e| e.profit_exit.as_ref()) {
                Some(exit) => format!(
                    "\nProfit exit: {} at {} · {} / {} filled",
                    exit.evidence.as_ref().map(|e| e.status.as_str()).unwrap_or(
                        if exit.attempted {
                            "unverified"
                        } else {
                            "not sent"
                        }
                    ),
                    exit.plan.price,
                    exit.evidence
                        .as_ref()
                        .map(|e| e.filled_quantity)
                        .unwrap_or(0.0),
                    exit.plan.quantity
                ),
                None => "\nProfit exit: no confirmed target yet.".into(),
            },
        );
    }
    if !c.new_entries {
        message
            .push_str("\nNew entries off. Existing orders and positions remain at the exchange.");
    }
    if let Some(result) = w.trade_result["summary"].as_str() {
        message.push_str("\n\nLast trade · ");
        message.push_str(result);
    } else if c.last_trade.is_some() {
        message.push_str("\n\nLast trade result requires a fresh exchange read.");
    }
    view["message"] = json!(message);
    view["summary"] = json!(format!(
        "BingX · LIVE · {} · {}\n{} · Entry margin {} USDT · Stop budget {}% of margin",
        c.engine.settings.symbol,
        if c.engine.settings.timeframes == "all" {
            "All six timeframes"
        } else {
            c.engine.settings.timeframes.as_str()
        },
        c.engine
            .account
            .as_ref()
            .map(|a| a.account_label.as_str())
            .unwrap_or("Account not connected"),
        c.engine.settings.entry_margin,
        c.engine.settings.stop_percent
    ));
    view["details"] = json!(format!(
        "{}\n\n{}\n{}",
        view["details"].as_str().unwrap_or(""),
        format!(
            "{}\n{}",
            evidence,
            w.trade_result["details"].as_str().unwrap_or("")
        ),
        stop_details
    ));
    view["details_title"] = json!("Orders and calculation evidence");
    // One public trading journey. Private prepare/place/cancel commands are
    // used by the same cycle executor, never a parallel manual-entry workflow.
    let connected = c.engine.account.is_some();
    let managed = c
        .engine
        .entry
        .as_ref()
        .filter(|e| e.attempted && !e.not_sent());
    let mut actions = Vec::new();
    if connected {
        actions.push(if w.running && c.new_entries {
            json!({"id":"stop","label":"Stop new entries"})
        } else {
            json!({"id":"start","label":"Start Jack","host":"workspace.start"})
        });
    }
    actions.push(json!({"id":"connect","label":if connected {"Reconnect account"} else {"Connect account"},"host":"workspace.authorize"}));
    actions.push(match managed {
        Some(entry) => json!({"id":"refresh_order","label":if entry.position.is_some() {"Refresh position"} else {"Refresh order"}}),
        None => json!({"id":"inspect","label":"Refresh market"}),
    });
    if connected {
        actions.push(json!({"id":"list_orders","label":"Open orders"}));
    }
    view["actions"] = json!(actions);
    if let Some(field) = view["fields"]
        .as_array_mut()
        .and_then(|fields| fields.iter_mut().find(|f| f["id"] == "timeframes"))
    {
        field["type"] = json!("choice");
        field["label"] = json!("Timeframe");
        field["source"] = json!({"action":"timeframes"});
        field.as_object_mut().unwrap().remove("advanced");
    }
    Ok(view)
}

fn stop_coverage(raw: &Value, position: &PositionObservation) -> R<StopCoverage> {
    let rows = raw
        .get("orders")
        .unwrap_or(raw)
        .as_array()
        .ok_or("Invalid stop-order observation")?;
    let mut stops = Vec::new();
    let mut ids = std::collections::BTreeSet::new();
    for r in rows {
        let side = if position.side == "long" {
            "SELL"
        } else {
            "BUY"
        };
        let hedge_side = if position.side == "long" {
            "LONG"
        } else {
            "SHORT"
        };
        let flag = |field: &str| r[field] == true || r[field] == "true";
        if r["type"] != "STOP_MARKET"
            || ![json!("NEW"), json!("PENDING")].contains(&r["status"])
            || r["side"] != side
            || !(r["positionSide"] == hedge_side
                || (r["positionSide"] == "BOTH" && (flag("reduceOnly") || flag("closePosition"))))
        {
            continue;
        }
        if let Some(id) = r.get("positionId").or_else(|| r.get("positionID")) {
            if text(id)? != position.position_id {
                continue;
            }
        }
        let id = text(r.get("orderID").unwrap_or(&r["orderId"]))?;
        if id.len() > 30 || !id.bytes().all(|b| b.is_ascii_digit()) || !ids.insert(id.clone()) {
            return Err("Ambiguous stop-order identity".into());
        }
        let trigger = number(&r["stopPrice"])?;
        let source = text(&r["workingType"])?;
        if trigger <= 0.0
            || !["MARK_PRICE", "CONTRACT_PRICE", "INDEX_PRICE"].contains(&source.as_str())
        {
            return Err("Stop trigger is unavailable".into());
        }
        let quantity = if flag("closePosition") {
            position.quantity
        } else {
            let original = number(&r["origQty"])?;
            let filled = number(&r["executedQty"])?;
            if filled > original {
                return Err("Invalid stop remaining quantity".into());
            }
            original - filled
        };
        stops.push((
            quantity,
            format!("order {id}: trigger {trigger} ({source}), remaining {quantity}"),
            trigger,
        ));
    }
    if stops.is_empty() {
        return Ok(StopCoverage {
            summary: "No matching active STOP_MARKET confirmed".into(),
            details: "A requested stop or a stop-limit is not verified market-stop coverage."
                .into(),
            trigger: None,
        });
    }
    let coverage = if stops.len() > 1 {
        "Multiple matching stops — review possible duplicates; quantities are not added"
    } else if stops[0].0 == 0.0 {
        "No observed quantity covered"
    } else if stops[0].0 >= position.quantity {
        "Full observed quantity covered"
    } else {
        "Partial observed quantity covered"
    };
    Ok(StopCoverage {
        summary: if stops.len() == 1 { format!("{coverage} · Trigger {} · Qty {}", stops[0].2, stops[0].0) } else { coverage.into() },
        details: format!("Position {} ({}). {}. Observation only; trigger acceptance does not guarantee execution price or a maximum loss.", position.position_id, position.quantity, stops.iter().take(4).map(|(_, text, _)| text.as_str()).collect::<Vec<_>>().join("; ")),
        trigger: if stops.len() == 1 { Some(stops[0].2) } else { None },
    })
}

fn service_response(result: &Value) -> R<Value> {
    if result["outcome"] != "response"
        || result["status"]
            .as_u64()
            .is_none_or(|s| !(200..300).contains(&s))
    {
        return Err("Service response unavailable; no blind resubmission".into());
    }
    let raw: Value = serde_json::from_str(result["body"].as_str().ok_or("Missing response body")?)
        .map_err(|_| "Unreadable service response")?;
    Ok(raw)
}

fn order(c: &Checkpoint, job: &Value, raw: &Value, now: i64, exit: bool) -> R<Value> {
    let p = &job["plan"];
    let entry = if exit { &p["entry_plan"] } else { p };
    let r = if raw["order"].is_object() {
        &raw["order"]
    } else {
        raw
    };
    let expected = client_id(c, p, exit);
    let id = text(r.get("orderID").unwrap_or(&r["orderId"]))?;
    let expected_order = c
        .engine
        .entry
        .as_ref()
        .and_then(|e| {
            if exit {
                e.profit_exit.as_ref().and_then(|e| e.evidence.as_ref())
            } else {
                e.evidence.as_ref()
            }
        })
        .and_then(|e| e.order_id.as_deref());
    let side = if (entry["side"] == "long") != exit {
        "BUY"
    } else {
        "SELL"
    };
    let position_side = if entry["side"] == "long" {
        "LONG"
    } else {
        "SHORT"
    };
    if r["symbol"] != entry["symbol"]
        || r["clientOrderId"] != expected
        || r["side"] != side
        || r["type"] != "LIMIT"
        || ![json!(position_side), json!("BOTH")].contains(&r["positionSide"])
        || (exit
            && r["positionSide"] == "BOTH"
            && ![json!(true), json!("true")].contains(&r["reduceOnly"]))
        || number(&r["origQty"])? != number(&p["quantity"])?
        || number(&r["price"])? != number(&p["price"])?
        || expected_order.is_some_and(|expected| expected != id)
    {
        return Err("Service order changed its exact identity or reducing scope".into());
    }
    let status = match r["status"].as_str() {
        Some("NEW" | "PENDING") => "open",
        Some("PARTIALLY_FILLED") => "partial",
        Some("FILLED") => "filled",
        Some("CANCELED" | "CANCELLED") => "cancelled",
        Some("REJECTED") => "rejected",
        Some("EXPIRED") => "expired",
        _ => return Err("Unknown service order status".into()),
    };
    Ok(
        json!({"account_id":entry["account_id"],"symbol":entry["symbol"],"client_order_id":expected,"order_id":id,"status":status,
        "filled_quantity":number(&r["executedQty"])?,"average_price":number(&r["avgPrice"])?,"observed_at_ms":now}),
    )
}
fn positions(raw: &Value, symbol: &Value) -> R<Value> {
    let rows = raw.as_array().ok_or("Invalid position list")?;
    if rows.len() > 64 {
        return Err("Too many position observations".into());
    }
    let mut out = Vec::new();
    for r in rows {
        if &r["symbol"] != symbol {
            continue;
        }
        let amount = r["positionAmt"]
            .as_f64()
            .or_else(|| r["positionAmt"].as_str().and_then(|s| s.parse().ok()))
            .ok_or("Invalid position quantity")?;
        if !amount.is_finite() {
            return Err("Invalid position quantity".into());
        }
        if amount == 0.0 {
            continue;
        }
        let side = match r["positionSide"].as_str() {
            Some("LONG") => "long",
            Some("SHORT") => "short",
            Some("BOTH") => {
                if amount > 0.0 {
                    "long"
                } else {
                    "short"
                }
            }
            _ => return Err("Invalid position side".into()),
        };
        out.push(json!({"position_id":text(&r["positionId"])?,"side":side,"quantity":amount.abs(),"average_price":number(&r["avgPrice"])?}));
    }
    Ok(json!(out))
}
fn open_orders(raw: &Value, symbol: &Value) -> R<Value> {
    let data = if raw["orders"].is_array() {
        &raw["orders"]
    } else {
        raw
    };
    let rows = data.as_array().ok_or("Invalid open orders")?;
    if rows.len() > 64 {
        return Err("Too many open orders".into());
    }
    let mut out = Vec::new();
    for r in rows {
        if &r["symbol"] != symbol {
            continue;
        }
        out.push(json!({"order_id":text(r.get("orderId").unwrap_or(&r["orderID"]))?,"side":r["side"],"position_side":r["positionSide"],"type":r["type"],"status":r["status"],
            "price":number(&r["price"])?.to_string(),"stop_price":if r["stopPrice"].is_null() || r["stopPrice"]=="" {"0".into()} else {number(&r["stopPrice"])?.to_string()},
            "quantity":number(&r["origQty"])?.to_string(),"filled_quantity":number(&r["executedQty"])?.to_string()}));
    }
    Ok(json!(out))
}

fn bars(raw: &Value) -> R<Vec<Value>> {
    let rows = raw.as_array().ok_or("Invalid candle response")?;
    let mut out = Vec::new();
    for r in rows {
        let fields = if let Some(a) = r.as_array() {
            if a.len() < 5 {
                return Err("Incomplete candle".into());
            }
            a[..5].to_vec()
        } else {
            vec![
                r["time"].clone(),
                r["open"].clone(),
                r["high"].clone(),
                r["low"].clone(),
                r["close"].clone(),
            ]
        };
        let time = number(&fields[0])?;
        if time.fract() != 0.0 {
            return Err("Invalid candle time".into());
        }
        let bar = json!([
            time as i64,
            number(&fields[1])?,
            number(&fields[2])?,
            number(&fields[3])?,
            number(&fields[4])?
        ]);
        serde_json::from_value::<Candle>(bar.clone())
            .map_err(|_| "Invalid candle")?
            .validate()?;
        out.push(bar);
    }
    out.sort_by_key(|r| r[0].as_i64().unwrap());
    if out.windows(2).any(|b| b[0][0] == b[1][0]) {
        return Err("Duplicate candle".into());
    }
    Ok(out)
}
fn request_for(c: &mut Checkpoint, w: &mut Work, now: i64) -> R<Option<Value>> {
    if w.read_retry_wait {
        w.read_retry_wait = false;
        return Ok(Some(json!({"kind":"runtime.wait","id":"read_retry",
            "continuation":"service_result","milliseconds":5000})));
    }
    loop {
        if w.job.is_none() {
            if !w.queue.is_empty() {
                w.job = Some(w.queue.remove(0));
                w.account = Value::Null;
                w.lifecycle = Value::Null;
                w.rows.clear();
                w.page = 0;
            } else if let Some(resume) = w.resume.take() {
                engine(c, w, &resume, None, None, now)?;
                continue;
            } else if w.autonomous {
                // The package, not the host, chooses whether its current plan
                // calls for cancellation, a reducing exit, or another entry.
                let parsed = &c.engine;
                let action = if parsed.entry.as_ref().is_some_and(|e| !e.attempted) && c.new_entries
                {
                    Some("place_entry")
                } else if pending_cancellation(&parsed, now).is_some() && c.new_entries {
                    Some("cancel_entry")
                } else if parsed
                    .entry
                    .as_ref()
                    .and_then(|e| e.profit_exit.as_ref())
                    .is_some_and(|e| !e.attempted)
                {
                    Some("place_exit")
                } else {
                    None
                };
                w.autonomous = false; // At most one semantic mutation per wake.
                if let Some(action) = action {
                    engine(c, w, action, None, None, now)?;
                    continue;
                }
                return Ok(None);
            } else {
                return Ok(None);
            }
        }
        let mut j = w.job.take().unwrap();
        w.pending = Some(j.clone());
        let mut kind = j["kind"]
            .as_str()
            .ok_or("Unknown package operation")?
            .to_owned();
        if kind == "entry.dispatch" {
            if now - j["checked_at"].as_i64().unwrap_or(0) > 60_000 {
                return Err("Final market validation is stale; no entry sent".into());
            }
            kind = "order.entry.place".into();
            j["kind"] = json!(kind);
            j["stage"] = json!("mutation");
            j["intent"] = json!(entry_id(&j["plan"]));
        }
        if kind == "market.candles.read" {
            if w.candle_wait {
                w.candle_wait = false;
                w.job = Some(j);
                return Ok(Some(json!({"kind":"runtime.wait","id":"candle_wait",
                    "continuation":"service_result","milliseconds":1200})));
            }
            if !w.rows.is_empty() {
                let count = 12.min(w.rows.len());
                let page = w.rows.drain(..count).collect::<Vec<_>>();
                let at = j["at"].as_i64().unwrap();
                let span = duration(j["timeframe"].as_str().unwrap())?;
                let last = j["last"].as_i64().unwrap();
                let final_batch =
                    w.rows.is_empty() && page.last().unwrap()[0].as_i64() == Some(last);
                let snap = json!({"symbol":j["symbol"],"timeframe":j["timeframe"],"candles":page,
                    "current_price":j["current"][4],"current_candle":j["current"],
                    "history_start_ms":j["first"],"history_end_ms":last,"batch_complete":final_batch});
                let next = if j["validate"] == true {
                    "validate_entry"
                } else {
                    "market"
                };
                // Keep the validation observation identical across streamed
                // chunks; actual expiry is checked again before dispatch.
                engine(c, w, next, Some(snap), None, at)?;
                if final_batch {
                    for job in &mut w.queue {
                        if job["kind"] == "entry.dispatch" {
                            job["checked_at"] = json!(at);
                        }
                    }
                    w.job = None;
                } else {
                    w.job = Some(j);
                }
                let _ = span;
                // Pace the next HTTP page through the common wait primitive.
                // Internal strategy chunks need no service throttle; reuse the
                // final chunk continuation rather than adding one per page.
                return Ok(Some(if w.rows.is_empty() {
                    json!({"kind":"runtime.wait","id":"chunk","continuation":"service_result","milliseconds":1200})
                } else {
                    json!({"kind":"runtime.yield","id":"chunk","continuation":"service_result"})
                }));
            }
            if j["stage"].is_null() {
                j["stage"] = json!("quote");
                j["at"] = json!(now);
            }
            let at = j["at"].as_i64().unwrap();
            let tf = j["timeframe"].as_str().ok_or("Missing timeframe")?;
            let span = duration(tf)?;
            let current = at - at % span;
            let mut p = params(&[("symbol", text(&j["symbol"])?), ("interval", tf.into())]);
            if j["stage"] == "quote" {
                p.insert("limit".into(), "2".into());
            } else {
                p.insert("limit".into(), CANDLE_PAGE.to_string());
                let start = j["first"].as_i64().unwrap() + w.page as i64 * span;
                p.insert("startTime".into(), start.to_string());
                p.insert(
                    "endTime".into(),
                    // The service excludes endTime; the package's history_end
                    // remains the last closed candle, inclusive.
                    (start + CANDLE_PAGE * span).min(current).to_string(),
                );
            }
            w.job = Some(j);
            return Ok(Some(http(w, CANDLES, "GET", p, false, now)));
        }
        if j["stage"].is_null() {
            j["stage"] = json!("uid");
            j["at"] = json!(now);
        }
        let symbol = if j["plan"]["entry_plan"].is_object() {
            text(&j["plan"]["entry_plan"]["symbol"])?
        } else if j["plan"].is_object() {
            text(&j["plan"]["symbol"])?
        } else {
            text(&j["symbol"])?
        };
        let mut p = params(&[("symbol", symbol)]);
        if kind == "trade.result.read" {
            let trade = &j["trade"];
            let from = trade["from"].as_i64().ok_or("Missing trade start")?;
            let end = trade["closed_at"].as_i64().ok_or("Missing trade closure")?;
            if from <= 0 || end < from || end - from > 7 * 86_400_000 {
                return Err(
                    "Last trade result unavailable: exchange history window exceeds seven days"
                        .into(),
                );
            }
            let stage = text(&j["stage"])?;
            let path = match stage.as_str() {
                "uid" => {
                    p.clear();
                    UID
                }
                "result_order" => {
                    p.insert("orderId".into(), text(&trade["entry_id"])?);
                    ORDER
                }
                "result_history" => {
                    p.extend(params(&[
                        ("startTime", from.to_string()),
                        ("endTime", end.to_string()),
                        ("limit", "100".into()),
                    ]));
                    HISTORY
                }
                "result_fills" => {
                    p.extend(params(&[
                        ("tradingUnit", "COIN".into()),
                        ("startTs", from.to_string()),
                        ("endTs", end.to_string()),
                        (
                            "orderId",
                            text(&j["orders"][j["index"].as_u64().unwrap_or(0) as usize]["id"])?,
                        ),
                    ]));
                    FILLS
                }
                "result_positions" => POSITIONS,
                "result_open" => OPEN,
                _ => return Err("Invalid trade-result continuation".into()),
            };
            w.job = Some(j);
            return Ok(Some(http(w, path, "GET", p, true, now)));
        }
        let stage = j["stage"].as_str().unwrap();
        let path = match stage {
            "uid" => {
                p.clear();
                UID
            }
            "balance" => {
                p.clear();
                BALANCE
            }
            "leverage" => LEVERAGE,
            "contract" => CONTRACT,
            "positions" => POSITIONS,
            "open" => OPEN,
            "mode" => MODE,
            "order" | "exit" => {
                let exit =
                    stage == "exit" || (kind == "position.exit.place" && !j["intent"].is_null());
                let plan = if stage == "exit" && kind == "order.snapshot.read" {
                    &j["exit_plan"]
                } else if kind == "position.exit.place" && j["intent"].is_null() {
                    &j["plan"]["entry_plan"]
                } else {
                    &j["plan"]
                };
                p.insert("clientOrderId".into(), client_id(c, plan, exit));
                ORDER
            }
            "mutation" => ORDER,
            _ => return Err("Unknown service stage".into()),
        };
        let mut method = "GET";
        if stage == "mutation" {
            method = if kind == "order.entry.cancel" {
                "DELETE"
            } else {
                "POST"
            };
            let intent = text(&j["intent"])?;
            if c.intents.contains_key(&intent) {
                return Err("Delivery is already uncertain; reconcile its exact ID".into());
            }
            let plan = &j["plan"];
            if kind != "order.entry.cancel" && now > plan["expires_at_ms"].as_i64().unwrap_or(0) {
                return Err("Plan expired during preflight; no mutation sent".into());
            }
            if kind == "order.entry.cancel" {
                p.insert("orderId".into(), text(&j["order_id"])?);
            } else {
                let exit = kind == "position.exit.place";
                let entry = if exit { &plan["entry_plan"] } else { plan };
                let side = if (entry["side"] == "long") != exit {
                    "BUY"
                } else {
                    "SELL"
                };
                p.extend(params(&[
                    ("side", side.into()),
                    ("type", "LIMIT".into()),
                    (
                        "timeInForce",
                        if exit {
                            "GTC".into()
                        } else {
                            "PostOnly".into()
                        },
                    ),
                    ("quantity", number(&plan["quantity"])?.to_string()),
                    ("price", number(&plan["price"])?.to_string()),
                    ("clientOrderId", client_id(c, plan, exit)),
                ]));
                let hedge = j["hedge"].as_bool().ok_or("Unknown position mode")?;
                p.insert(
                    "positionSide".into(),
                    if hedge {
                        if entry["side"] == "long" {
                            "LONG"
                        } else {
                            "SHORT"
                        }
                    } else {
                        "BOTH"
                    }
                    .into(),
                );
                if exit {
                    p.insert("positionId".into(), text(&plan["position_id"])?);
                    if !hedge {
                        p.insert("reduceOnly".into(), "true".into());
                    }
                } else {
                    p.insert("stopLoss".into(),json!({"type":"STOP_MARKET","stopPrice":plan["stop_price"],"workingType":"CONTRACT_PRICE"}).to_string());
                }
            }
            if c.intents.len() >= 128 {
                return Err("Resolve pending delivery evidence before another effect".into());
            }
            w.dirty = true;
            c.intents.insert(intent, kind.clone()); // Durable before host dispatch.
        }
        let signed = stage != "contract";
        w.job = Some(j);
        return Ok(Some(http(w, path, method, p, signed, now)));
    }
}

fn receive(c: &mut Checkpoint, w: &mut Work, result: &Value, now: i64) -> R<()> {
    if matches!(
        result["id"].as_str(),
        Some("chunk" | "candle_wait" | "read_retry")
    ) {
        return Ok(());
    }
    let mut j = w.job.take().ok_or("No pending package continuation")?;
    w.pending = Some(j.clone());
    let kind = text(&j["kind"])?;
    if j["stage"] == "mutation" {
        // HTTP acceptance is not an order receipt. Reconcile even on response
        // loss; a confirmed non-dispatch alone may release a delivery intent.
        if result["outcome"] == "not_dispatched" {
            if let Some(intent) = j["intent"].as_str() {
                c.intents.remove(intent);
                w.dirty = true;
            }
            if kind != "order.entry.cancel" {
                let exit = kind == "position.exit.place";
                let p = &j["plan"];
                let entry = if exit { &p["entry_plan"] } else { p };
                let e = json!({"account_id":entry["account_id"],"symbol":entry["symbol"],"client_order_id":client_id(c,p,exit),"order_id":null,
                    "status":"not_sent","filled_quantity":0.0,"average_price":0.0,"observed_at_ms":now,"error_message":"Host confirmed non-dispatch"});
                engine(
                    c,
                    w,
                    if exit { "exit" } else { "order" },
                    Some(e),
                    None,
                    now,
                )?;
                return Ok(());
            }
        }
        j["stage"] = json!("order");
        w.job = Some(j);
        return Ok(());
    }
    let response = service_response(result)?;
    if response["code"] != 0 {
        // Only a read rejected before returning data is repeated. Bound retries
        // across the entire action; mutations keep their reconcile-only path.
        if kind == "market.candles.read" && response["code"] == 100410 && w.read_retries < 2 {
            w.read_retries += 1;
            w.read_retry_wait = true;
            w.job = Some(j);
            return Ok(());
        }
        return Err(format!("Service rejected read: {}", response["code"]));
    }
    let raw = response["data"].clone();
    if kind == "market.candles.read" {
        let at = j["at"].as_i64().unwrap();
        let span = duration(j["timeframe"].as_str().unwrap())?;
        let current = at - at % span;
        let rows = bars(&raw)?;
        if j["stage"] == "quote" {
            let latest = rows.last().ok_or("Missing current candle")?;
            if latest[0].as_i64() != Some(current) {
                return Err("Current candle is unavailable".into());
            }
            j["current"] = latest.clone();
            j["first"] = json!((current - 600 * span).max(0));
            j["last"] = json!(current - span);
            j["stage"] = json!("history");
            w.candle_wait = true;
        } else {
            let start = j["first"].as_i64().unwrap() + w.page as i64 * span;
            let expected =
                ((j["last"].as_i64().unwrap() - start) / span + 1).clamp(0, CANDLE_PAGE) as usize;
            if rows.len() != expected
                || rows
                    .iter()
                    .enumerate()
                    .any(|(i, r)| r[0].as_i64() != Some(start + i as i64 * span))
            {
                return Err("Partial or gapped candle history; no effect sent".into());
            }
            w.page += rows.len();
            w.rows = rows;
        }
        w.job = Some(j);
        return Ok(());
    }
    if kind == "trade.result.read" {
        return receive_trade_result(c, w, j, &raw, now);
    }
    let stage = text(&j["stage"])?;
    let symbol = if j["plan"]["entry_plan"].is_object() {
        j["plan"]["entry_plan"]["symbol"].clone()
    } else if j["plan"].is_object() {
        j["plan"]["symbol"].clone()
    } else {
        j["symbol"].clone()
    };
    match stage.as_str() {
        "uid" => {
            let uid = text(&raw["uid"])?;
            if uid.len() > 30 || !uid.bytes().all(|b| b.is_ascii_digit()) {
                return Err("Invalid account UID".into());
            }
            let id = account_id(&uid);
            let expected = if j["plan"]["entry_plan"].is_object() {
                &j["plan"]["entry_plan"]["account_id"]
            } else if j["plan"].is_object() {
                &j["plan"]["account_id"]
            } else {
                &j["account_id"]
            };
            if !expected.is_null() && expected != &json!(id) {
                return Err("Reconnect the exact account owning the managed attempt".into());
            }
            w.verified = Some(id.clone());
            w.account = json!({"account_id":id,"account_label":format!("BingX LIVE / account ending {}",&uid[uid.len().saturating_sub(4)..])});
            j["stage"] = json!(
                if kind == "account.connect" || kind == "account.snapshot.read" {
                    "balance"
                } else if kind == "order.snapshot.read" && j["scope"] == "open" {
                    "open"
                } else if kind == "order.entry.place" {
                    "positions"
                } else {
                    "order"
                }
            );
        }
        "balance" => {
            let matches = raw
                .as_array()
                .ok_or("Invalid balance")?
                .iter()
                .filter(|r| r["asset"] == "USDT")
                .collect::<Vec<_>>();
            if matches.len() != 1 {
                return Err("Ambiguous account balance".into());
            }
            w.account["available_margin"] = json!(number(&matches[0]["availableMargin"])?);
            j["stage"] = json!("leverage");
        }
        "leverage" => {
            for (key, field) in [
                ("long_leverage", "longLeverage"),
                ("short_leverage", "shortLeverage"),
            ] {
                let value = number(&raw[field])?;
                if value.fract() != 0.0 || !(1.0..=10000.0).contains(&value) {
                    return Err("Invalid account leverage".into());
                }
                w.account[key] = json!(value as u32);
            }
            if kind == "order.entry.place" {
                let key = if j["plan"]["side"] == "long" {
                    "long_leverage"
                } else {
                    "short_leverage"
                };
                if w.account[key] != j["plan"]["leverage"] {
                    return Err("Leverage changed; no entry sent".into());
                }
                j["stage"] = json!("mode");
            } else {
                j["stage"] = json!("contract");
            }
        }
        "contract" => {
            let matches = raw
                .as_array()
                .ok_or("Invalid contracts")?
                .iter()
                .filter(|r| r["symbol"] == symbol)
                .collect::<Vec<_>>();
            if matches.len() != 1
                || matches[0]["status"] != 1
                || ![json!(true), json!("true")].contains(&matches[0]["apiStateOpen"])
            {
                return Err("Instrument unavailable or ambiguous".into());
            }
            let r = matches[0];
            let mut a = w.account.take();
            for (target, source) in [
                ("price_precision", "pricePrecision"),
                ("quantity_precision", "quantityPrecision"),
            ] {
                let n = number(&r[source])?;
                if n.fract() != 0.0 || n > 12.0 {
                    return Err("Invalid contract precision".into());
                }
                a[target] = json!(n as u32);
            }
            a["min_quantity"] = json!(number(&r["tradeMinQuantity"])?);
            a["min_notional"] = json!(number(&r["tradeMinUSDT"])?);
            a["symbol"] = symbol;
            a["endpoint"] = json!("LIVE");
            a["observed_at_ms"] = json!(now);
            engine(c, w, "account", Some(a), None, now)?;
            return Ok(());
        }
        "order" => {
            let exit = kind == "position.exit.place" && !j["intent"].is_null();
            let original = if kind == "position.exit.place" && !exit {
                json!({"plan":j["plan"]["entry_plan"]})
            } else {
                j.clone()
            };
            let e = order(c, &original, &raw, now, exit)?;
            if kind == "order.snapshot.read" && j["scope"] == "lifecycle" {
                w.lifecycle =
                    json!({"account_id":j["plan"]["account_id"],"symbol":symbol,"entry":e});
                j["stage"] = json!("positions");
            } else if kind == "order.entry.cancel" {
                if e["status"] != "open" || number(&e["filled_quantity"])? != 0.0 {
                    engine(c, w, "order", Some(e), None, now)?;
                    return Ok(());
                }
                j["stage"] = json!("positions");
                w.lifecycle = json!({"entry":e});
            } else if kind == "position.exit.place" && j["intent"].is_null() {
                // A new reducing send first verifies the original entry.
                w.lifecycle = json!({"entry":e});
                j["stage"] = json!("positions");
            } else {
                engine(
                    c,
                    w,
                    if exit { "exit" } else { "order" },
                    Some(e),
                    None,
                    now,
                )?;
                return Ok(());
            }
        }
        "positions" => {
            let p = positions(&raw, &symbol)?;
            j["positions_observed_at_ms"] = json!(now);
            if kind == "order.entry.place" {
                if !p.as_array().unwrap().is_empty() {
                    return Err("Current position exists; no new entry sent".into());
                }
                j["stage"] = json!("open");
            } else if kind == "order.snapshot.read" {
                w.lifecycle["positions"] = p;
                j["stage"] = json!("open");
            } else if kind == "order.entry.cancel" {
                if !p.as_array().unwrap().is_empty() {
                    return Err("Position changed; no cancellation sent".into());
                }
                j["intent"] = json!(format!(
                    "cancel:{}:{}",
                    entry_id(&j["plan"]),
                    text(&j["order_id"])?
                ));
                j["stage"] = json!("mutation");
            } else {
                let rows = p.as_array().unwrap();
                let plan = &j["plan"];
                if rows.len() != 1
                    || rows[0]["position_id"] != plan["position_id"]
                    || rows[0]["side"] != plan["entry_plan"]["side"]
                    || number(&rows[0]["quantity"])? != number(&plan["quantity"])?
                    || !same_price(
                        number(&rows[0]["average_price"])?,
                        number(&plan["average_price"])?,
                    )
                    || number(&w.lifecycle["entry"]["filled_quantity"])?
                        < number(&plan["quantity"])?
                {
                    return Err("Reducing position changed; no exit sent".into());
                }
                engine(c, w, "validate_exit", None, None, now)?;
                j["stage"] = json!("mode");
            }
        }
        "open" => {
            if kind == "order.entry.place" {
                let rows = if raw["orders"].is_array() {
                    &raw["orders"]
                } else {
                    &raw
                };
                let client = client_id(c, &j["plan"], false);
                if rows
                    .as_array()
                    .ok_or("Invalid open orders")?
                    .iter()
                    .any(|r| r["symbol"] == symbol && r["clientOrderId"] == client)
                {
                    return Err("This exact entry already exists".into());
                }
                j["stage"] = json!("leverage");
            } else {
                let orders = open_orders(&raw, &symbol)?;
                if j["scope"] == "open" {
                    engine(
                        c,
                        w,
                        "open_orders",
                        Some(
                            json!({"account_id":w.verified,"symbol":symbol,"orders":orders,"observed_at_ms":now}),
                        ),
                        None,
                        now,
                    )?;
                    return Ok(());
                }
                // A unique current service position may be observed without
                // adopting it. Presentation additionally requires the strategy's
                // exact managed-fill match after lifecycle reconciliation.
                w.protection = None;
                if let Some(rows) = w.lifecycle["positions"].as_array() {
                    if let [position] = rows.as_slice() {
                        let position: PositionObservation =
                            serde_json::from_value(position.clone())
                                .map_err(|_| "Invalid protection position")?;
                        let scoped = raw
                            .get("orders")
                            .unwrap_or(&raw)
                            .as_array()
                            .ok_or("Invalid open orders")?
                            .iter()
                            .filter(|r| r["symbol"] == symbol)
                            .cloned()
                            .collect::<Vec<_>>();
                        let at = j["positions_observed_at_ms"]
                            .as_i64()
                            .ok_or("Missing position observation time")?;
                        w.protection = Some((
                            position.clone(),
                            at,
                            stop_coverage(&json!(scoped), &position)?,
                        ));
                    }
                }
                w.lifecycle["orders"] = orders;
                if j["exit_plan"].is_object() {
                    j["stage"] = json!("exit");
                } else {
                    let mut snapshot = w.lifecycle.take();
                    snapshot["observed_at_ms"] = json!(now);
                    engine(c, w, "lifecycle", Some(snapshot), None, now)?;
                    return Ok(());
                }
            }
        }
        "exit" => {
            let e = order(c, &json!({"plan":j["exit_plan"]}), &raw, now, true)?;
            let mut snapshot = w.lifecycle.take();
            snapshot["exit"] = e;
            snapshot["observed_at_ms"] = json!(now);
            engine(c, w, "lifecycle", Some(snapshot), None, now)?;
            return Ok(());
        }
        "mode" => {
            let hedge = match raw["dualSidePosition"].clone() {
                Value::Bool(v) => v,
                Value::String(v) if v == "true" => true,
                Value::String(v) if v == "false" => false,
                _ => return Err("Position mode unavailable".into()),
            };
            j["hedge"] = json!(hedge);
            if kind == "order.entry.place" {
                // Final market validation remains entirely package owned.
                let tf = if j["plan"]["timeframe"] == "1m" {
                    "1m"
                } else {
                    "5m"
                };
                w.queue.insert(
                    0,
                    json!({"kind":"entry.dispatch","plan":j["plan"],"hedge":hedge}),
                );
                w.queue.insert(0,json!({"kind":"market.candles.read","symbol":symbol,"timeframe":tf,"validate":true}));
                return Ok(());
            }
            j["intent"] = json!(format!("exit:{}", client_id(c, &j["plan"], true)));
            j["stage"] = json!("mutation");
        }
        _ => return Err("Invalid service continuation".into()),
    }
    w.job = Some(j);
    Ok(())
}

fn proven_preflight_failure(c: &mut Checkpoint, w: &mut Work, now: i64) -> R<()> {
    let Some(j) = w.pending.take() else {
        return Ok(());
    };
    let kind = j["kind"].as_str().unwrap_or("");
    let exit = kind == "position.exit.place";
    if !["order.entry.place", "entry.dispatch", "position.exit.place"].contains(&kind) {
        return Ok(());
    }
    let p = &j["plan"];
    let managed = serde_json::to_value(if exit {
        c.engine
            .entry
            .as_ref()
            .and_then(|e| e.profit_exit.as_ref())
            .map(|e| (&e.plan, e.attempted, e.evidence.is_none()))
            .map(|(p, a, n)| json!({"plan":p,"attempted":a,"no_evidence":n}))
    } else {
        c.engine.entry.as_ref().map(
            |e| json!({"plan":e.plan,"attempted":e.attempted,"no_evidence":e.evidence.is_none()}),
        )
    })
    .unwrap();
    let identity = if exit {
        format!("exit:{}", client_id(c, p, true))
    } else {
        entry_id(p)
    };
    if managed["plan"] != *p
        || managed["attempted"] != true
        || managed["no_evidence"] != true
        || c.intents.contains_key(&identity)
    {
        return Ok(());
    }
    let entry = if exit { &p["entry_plan"] } else { p };
    let e = json!({"account_id":entry["account_id"],"symbol":entry["symbol"],"client_order_id":client_id(c,p,exit),
        "order_id":null,"status":"not_sent","filled_quantity":0.0,"average_price":0.0,"observed_at_ms":now,
        "error_message":"Package preflight ended before dispatch"});
    engine(
        c,
        w,
        if exit { "exit" } else { "order" },
        Some(e),
        None,
        now,
    )?;
    Ok(())
}

#[derive(Deserialize)]
struct Invocation<'a> {
    api_version: u32,
    schema_version: u32,
    plugin_id: String,
    host_method: String,
    observed_at_ms: i64,
    action: String,
    #[serde(borrow)]
    state: Option<&'a RawValue>,
    #[serde(borrow)]
    context: Option<&'a RawValue>,
    settings: Option<Value>,
    execution: Option<Value>,
    result: Option<Value>,
}
#[derive(Serialize)]
struct PackageOutput {
    state: Box<RawValue>,
    context: Box<RawValue>,
    view: Value,
    requests: Vec<Value>,
}
#[derive(Serialize)]
struct PackageEnvelope {
    schema_version: u32,
    status: &'static str,
    result: Option<PackageOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_code: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_message: Option<String>,
}

// Choices never interpret or rewrite the strategy checkpoint.
fn choices(input: &Invocation) -> R<PackageOutput> {
    let mut requests = Vec::new();
    let mut view = if input.action == "timeframes" {
        json!({"title":"Choose timeframe","message":"All uses 1D, 4H, 1H, 30m, 15m and 5m. A managed order keeps its original plan.","options":["all","5m","1m"]})
    } else if input.action == "instruments" {
        requests.push(json!({"kind":"api.describe","id":"instrument-api","continuation":"instruments_capabilities"}));
        json!({"title":"Choose instrument","message":"Checking the common response capabilities"})
    } else if input.action == "instruments_capabilities" {
        let catalog = input
            .result
            .as_ref()
            .map(|r| &r["catalog"])
            .ok_or("Common HTTP capabilities unavailable")?;
        let optional = catalog["http.request"]["optional"]
            .as_array()
            .ok_or("Common HTTP capabilities unavailable")?;
        if !optional.contains(&json!("response_json_paths")) {
            return Err("Update Capsule to load the complete instrument list; this host lacks bounded JSON response selection".into());
        }
        if catalog["http.request"]["max_response_json_source_bytes"]
            .as_u64()
            .unwrap_or(0)
            < 1024 * 1024
        {
            return Err("Update Capsule to load the complete contract catalog; the common HTTP source bound is too small".into());
        }
        requests.push(json!({"kind":"http.request","id":"instruments","continuation":"instruments_result",
            "url":format!("{ORIGIN}{CONTRACT}"),"method":"GET","max_response_bytes":1024*1024,
            "response_json_paths":{"code":["code"],"symbols":["data","*","symbol"],"statuses":["data","*","status"],"open":["data","*","apiStateOpen"]}}));
        json!({"title":"Choose instrument","message":"Loading current available contracts"})
    } else {
        let response = input.result.as_ref().ok_or("Missing instrument result")?;
        if response["outcome"] != "response" || response["status"] != 200 {
            return Err("Instrument list unavailable; retry the service read".into());
        }
        let contracts = &response["json"];
        let names = contracts["symbols"]
            .as_array()
            .ok_or("Missing contract symbols")?;
        let statuses = contracts["statuses"]
            .as_array()
            .ok_or("Missing contract statuses")?;
        let open = contracts["open"]
            .as_array()
            .ok_or("Missing contract API availability")?;
        if contracts["code"] != 0
            || names.len() > 4096
            || names.len() != statuses.len()
            || names.len() != open.len()
        {
            return Err("Instrument list unavailable or exceeds package bounds".into());
        }
        let mut symbols: Vec<&str> = names
            .iter()
            .zip(statuses)
            .zip(open)
            .filter_map(|((name, status), open)| {
                let s = name.as_str()?;
                (*status == 1
                    && open == "true"
                    && s.len() > 5
                    && s.len() <= 64
                    && s.ends_with("-USDT")
                    && s[..s.len() - 5]
                        .bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()))
                .then_some(s)
            })
            .collect();
        symbols.sort_unstable();
        symbols.dedup();
        if symbols.is_empty() {
            return Err("No USDT contracts currently open for API entry".into());
        }
        json!({"title":"Choose instrument","options":symbols,"message":"Active USDT contracts open for API entry. Availability is checked again before entry."})
    };
    view["details"] = json!("");
    for field in ["fields", "actions", "columns", "rows"] {
        view[field] = json!([]);
    }
    Ok(PackageOutput {
        state: input
            .state
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| serde_json::value::to_raw_value(&Checkpoint::default()).unwrap()),
        context: serde_json::value::to_raw_value(&Value::Null).unwrap(),
        view,
        requests,
    })
}

pub(super) fn evaluate_json(raw: &[u8]) -> Vec<u8> {
    fn run(raw: &[u8]) -> R<PackageOutput> {
        if raw.len() > INPUT_LIMIT {
            return Err("Input exceeds the bounded ABI".into());
        }
        let input: Invocation =
            serde_json::from_slice(raw).map_err(|_| "Invalid invocation JSON")?;
        if input.api_version != 2
            || input.schema_version != 1
            || input.plugin_id != PLUGIN_ID
            || input.host_method != "workspace"
        {
            return Err("Common capability API v2 is required".into());
        }
        let now = input.observed_at_ms;
        if now <= 0 {
            return Err("Invalid observation time".into());
        }
        let action = input.action.as_str();
        if matches!(
            action,
            "timeframes" | "instruments" | "instruments_capabilities" | "instruments_result"
        ) {
            return choices(&input);
        }
        if matches!(
            action,
            "prepare_entry" | "place_entry" | "place_exit" | "cancel_entry"
        ) {
            return Err("Use Start Jack for automatic order management, or Stop new entries. Manual exchange actions remain available at the exchange.".into());
        }
        let mut c = match input.state {
            None => Checkpoint::default(),
            Some(raw) => serde_json::from_str::<Checkpoint>(raw.get()).map_err(|_| "This package starts a new workspace session. Old local test checkpoints are not migrated; remove the package with saved-data deletion, then reinstall for a fresh session.")?,
        };
        if c.version != 2 {
            return Err("Unsupported package checkpoint".into());
        }
        if action == "service_result" {
            c.engine.validate()?;
        }
        let enabled = input
            .execution
            .as_ref()
            .is_some_and(|e| e["running"] == true);
        let mut w = if action == "service_result" {
            serde_json::from_str::<Work>(input.context.ok_or("Missing package continuation")?.get())
                .map_err(|_| "Invalid package continuation")?
        } else {
            Work {
                autonomous: enabled && action == "cycle",
                running: enabled,
                ..Work::default()
            }
        };
        let mut failure = None;
        let operation = if action == "service_result" {
            receive(
                &mut c,
                &mut w,
                input.result.as_ref().ok_or("Missing operation result")?,
                now,
            )
            .map(|_| ())
        } else if action == "stop" {
            c.new_entries = false;
            w.dirty = true;
            Ok(())
        } else if action == "start" {
            c.new_entries = true;
            w.dirty = true;
            let configured = input
                .settings
                .clone()
                .unwrap_or_else(|| serde_json::to_value(&c.engine.settings).unwrap());
            // An explicit Start replaces the previous executor's selection.
            // Its active flag is not a veto on the owner's newly chosen symbol.
            let running = w.running;
            w.running = false;
            engine(&mut c, &mut w, "start", None, Some(configured), now)?;
            w.running = running;
            w.queue.clear();
            w.resume = None;
            w.autonomous = false;
            Ok(())
        } else {
            let settings = input.settings;
            if action == "connect" {
                w.autonomous = false;
            }
            if matches!(action, "open" | "cycle" | "refresh_order") {
                // The host durably saves a dispatch intent before executing
                // HTTP. A restart before that checkpoint is proven non-send,
                // unlike an intent with a lost service response.
                let interrupted = c.engine.entry.as_ref().and_then(|entry| {
                    if entry.attempted && entry.evidence.is_none() {
                        Some(json!({"kind":"order.entry.place","plan":entry.plan}))
                    } else {
                        entry
                            .profit_exit
                            .as_ref()
                            .filter(|exit| exit.attempted && exit.evidence.is_none())
                            .map(|exit| json!({"kind":"position.exit.place","plan":exit.plan}))
                    }
                });
                if let Some(job) = interrupted {
                    w.pending = Some(job);
                    proven_preflight_failure(&mut c, &mut w, now)?;
                }
            }
            engine(&mut c, &mut w, action, None, settings, now).map(|_| {
                if matches!(action, "open" | "cycle" | "refresh_order" | "inspect") {
                    queue_trade_result(&c, &mut w);
                }
            })
        };
        if let Err(error) = operation {
            if !unavailable_trade_result(&mut w, &error) {
                proven_preflight_failure(&mut c, &mut w, now)?;
                failure = Some(error);
                w = Work {
                    dirty: w.dirty,
                    running: enabled,
                    trade_result: w.trade_result.take(),
                    ..Work::default()
                };
            }
        }
        let request = if failure.is_none() {
            loop {
                match request_for(&mut c, &mut w, now) {
                    Ok(r) => break r,
                    Err(error) if unavailable_trade_result(&mut w, &error) => continue,
                    Err(error) => {
                        proven_preflight_failure(&mut c, &mut w, now)?;
                        failure = Some(error);
                        w = Work {
                            dirty: w.dirty,
                            running: enabled,
                            trade_result: w.trade_result.take(),
                            ..Work::default()
                        };
                        break None;
                    }
                }
            }
        } else {
            None
        };
        let mut view = if request.is_some() {
            // Intermediate continuations are never shown by the host. Do not
            // rebuild a complete chart between every bounded HTTP/chunk step.
            json!({"title":"Jack Ventura","message":"Reading service evidence","details":"","fields":[],"actions":[],"rows":[],"columns":[]})
        } else {
            presentation(&mut c, &mut w, now)?
        };
        if let Some(error) = failure {
            view["message"] = json!(format!(
                "{}\n{}",
                view["message"].as_str().unwrap_or(""),
                error
            ));
        }
        // Serialize large opaque state once, then embed its original bytes.
        let state = if !w.dirty && input.state.is_some() {
            input.state.unwrap().to_owned()
        } else {
            serde_json::value::to_raw_value(&c).map_err(|_| "Invalid checkpoint")?
        };
        let context = serde_json::value::to_raw_value(&w).map_err(|_| "Invalid continuation")?;
        if state.get().len() > 32 * 1024 || context.get().len() > 16 * 1024 {
            return Err("Package checkpoint or continuation exceeds its bound".into());
        }
        Ok(PackageOutput {
            state,
            context,
            view,
            requests: request.into_iter().collect(),
        })
    }
    let out = match run(raw) {
        Ok(result) => PackageEnvelope {
            schema_version: 1,
            status: "executed",
            result: Some(result),
            error_code: None,
            error_message: None,
        },
        Err(error) => PackageEnvelope {
            schema_version: 1,
            status: "rejected",
            result: None,
            error_code: Some("invalid_args"),
            error_message: Some(error),
        },
    };
    serde_json::to_vec(&out).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn e(c: &Checkpoint) -> Value {
        serde_json::to_value(&c.engine).unwrap()
    }

    fn invoke(action: &str, state: Value, context: Value, result: Value, now: i64) -> Value {
        serde_json::from_slice(&evaluate_json(&serde_json::to_vec(&json!({
            "api_version":2,"schema_version":1,"plugin_id":PLUGIN_ID,"host_method":"workspace",
            "action":action,"state":state,"context":context,"result":result,"observed_at_ms":now,
            "execution":{"enabled":false,"scope":null}
        })).unwrap())).unwrap()
    }
    fn managed(status: &str, filled: f64) -> Checkpoint {
        let mut engine = crate::tests::lifecycle_state(status, filled);
        let account = account_id("123456789");
        engine["account"]["account_id"] = json!(account);
        engine["entry"]["plan"]["account_id"] = json!(account);
        engine["entry"]["evidence"]["account_id"] = json!(account);
        Checkpoint {
            version: 2,
            engine: serde_json::from_value(engine).unwrap(),
            new_entries: true,
            ..Checkpoint::default()
        }
    }
    fn observe(
        mut c: Checkpoint,
        status: &str,
        filled: f64,
        position: bool,
        manual: bool,
        now: i64,
    ) -> Checkpoint {
        let plan = e(&c)["entry"]["plan"].clone();
        let client = client_id(&c, &plan, false);
        let filled = number(&plan["quantity"]).unwrap() * filled;
        let mut w = Work {
            job: Some(json!({"kind":"order.snapshot.read","scope":"lifecycle","plan":plan})),
            ..Work::default()
        };
        for _ in 0..8 {
            if c.engine.entry.is_none() {
                return c;
            }
            let Some(request) = request_for(&mut c, &mut w, now).unwrap() else {
                return c;
            };
            assert_eq!(request["method"], "GET");
            let url = request["url"].as_str().unwrap();
            let raw = if url.contains(UID) {
                json!({"uid":"123456789"})
            } else if url.contains(ORDER) {
                json!({"orderId":"123","symbol":plan["symbol"],"clientOrderId":client,"side":"BUY","positionSide":"LONG","type":"LIMIT",
                    "origQty":plan["quantity"],"price":plan["price"],"executedQty":filled,"avgPrice":if filled>0.0 {96.5}else{0.0},"status":status})
            } else if url.contains(POSITIONS) {
                if position {
                    json!([{"symbol":plan["symbol"],"positionId":"456","positionSide":"LONG","positionAmt":filled,"avgPrice":96.5}])
                } else {
                    json!([])
                }
            } else if url.contains(OPEN) {
                if manual {
                    json!([{"symbol":plan["symbol"],"orderId":"789","clientOrderId":"","side":"BUY","positionSide":"LONG","type":"TRIGGER_LIMIT","status":"NEW","price":"90","stopPrice":"91","origQty":"2","executedQty":"0"}])
                } else {
                    json!([])
                }
            } else {
                panic!("Unexpected capability request {request}")
            };
            receive(&mut c,&mut w,&json!({"outcome":"response","status":200,"body":json!({"code":0,"data":raw}).to_string()}),now).unwrap();
        }
        panic!("Unbounded observation")
    }
    fn trade_result_fixture(
        reason: &str,
        side: &str,
        mut change: impl FnMut(&str, &mut Value),
    ) -> R<(Checkpoint, Work)> {
        let now = 1_793_916_970_001;
        let mut c = managed("filled", 1.0);
        let plan = serde_json::to_value(&c.engine.entry.as_ref().unwrap().plan).unwrap();
        let qty = number(&plan["quantity"])?;
        c.last_trade = Some(
            json!({"account_id":plan["account_id"],"symbol":plan["symbol"],"side":side,
            "from":now-1000,"closed_at":now,"entry_id":"123","client_id":"b".repeat(40),
            "position_id":"456","quantity":qty,"tp_id":if reason=="TP" {json!("999")} else {Value::Null}}),
        );
        c.engine.entry = None;
        let original = serde_json::to_value(&c).unwrap();
        let opening = if side == "long" { "BUY" } else { "SELL" };
        let closing = if side == "long" { "SELL" } else { "BUY" };
        let mut w = Work::default();
        queue_trade_result(&c, &mut w);
        for _ in 0..16 {
            let Some(r) = request_for(&mut c, &mut w, now)? else {
                assert_eq!(
                    serde_json::to_value(&c).unwrap(),
                    original,
                    "Exchange history must remain transient"
                );
                return Ok((c, w));
            };
            assert_eq!(r["method"], "GET");
            let url = r["url"].as_str().unwrap();
            let stage = text(&w.job.as_ref().unwrap()["stage"])?;
            let mut raw = match stage.as_str() {
                "uid" => json!({"uid":"123456789"}),
                "result_order" => {
                    json!({"order":{"orderId":"123","clientOrderId":"b".repeat(40),"symbol":"BTC-USDT",
                    "side":opening,"status":"FILLED","executedQty":qty,"positionID":"456"}})
                }
                "result_history" => {
                    json!({"orders":[{"orderId":"999","positionID":"456","symbol":"BTC-USDT","side":closing,
                    "positionSide":if side=="long" {"LONG"} else {"SHORT"},"status":"FILLED","executedQty":qty,"type":if reason=="SL" {"STOP_MARKET"} else if reason=="TP" {"LIMIT"} else {"MARKET"}},
                    {"orderId":"789","positionID":"other","symbol":"BTC-USDT","side":"SELL","executedQty":qty}]})
                }
                "result_fills" => {
                    assert!(
                        url.contains("startTs=")
                            && url.contains("endTs=")
                            && url.contains("tradingUnit=COIN")
                    );
                    let entry = url.contains("orderId=123&");
                    json!({"fill_orders":[{"orderId":if entry {"123"} else {"999"},
                        "symbol":"BTC-USDT","side":if entry {opening} else {closing},"volume":qty.to_string(),
                        "price":if entry {"96.5"} else {"105.0"},"commission":if entry {"-0.01"} else {"-0.02"},"currency":"USDT",
                        "filledTm":"2026-11-05T22:16:10.000Z","filledTime":"2026-11-06T06:16:10.000+08:00"}]})
                }
                "result_positions" => json!([]),
                "result_open" => {
                    json!({"orders":[{"orderId":"789","symbol":"BTC-USDT","positionID":"other"}]})
                }
                _ => panic!("Unexpected result stage {stage}"),
            };
            change(&stage, &mut raw);
            receive(
                &mut c,
                &mut w,
                &json!({"outcome":"response","status":200,"body":json!({"code":0,"data":raw}).to_string()}),
                now,
            )?;
            // Every continuation is serializable with the unchanged bounds;
            // a fresh invocation needs no host interpretation of its shape.
            let context = serde_json::to_vec(&w).unwrap();
            assert!(context.len() < 16 * 1024);
            w = serde_json::from_slice(&context).unwrap();
        }
        panic!("Unbounded result reads")
    }

    #[test]
    fn closed_trade_reads_exact_fills_fees_and_reason_without_archiving_service_history() {
        for side in ["long", "short"] {
            for reason in ["TP", "SL", "Other"] {
                let (c, mut w) = trade_result_fixture(reason, side, |_, _| {}).unwrap();
                assert!(w.trade_result["summary"]
                    .as_str()
                    .unwrap()
                    .starts_with(reason));
                assert!(w.trade_result["summary"]
                    .as_str()
                    .unwrap()
                    .contains("-0.030000"));
                let qty = number(&c.last_trade.as_ref().unwrap()["quantity"]).unwrap();
                let pnl = 8.5 * qty * if side == "long" { 1.0 } else { -1.0 };
                assert!(w.trade_result["summary"]
                    .as_str()
                    .unwrap()
                    .contains(&format!("Realized PnL {pnl:+.6}")));
                assert!(w.trade_result["summary"]
                    .as_str()
                    .unwrap()
                    .contains(&format!("After trading fees {:+.6}", pnl - 0.03)));
                assert!(w.trade_result["details"]
                    .as_str()
                    .unwrap()
                    .contains("123, 999"));
                // Reopen loses transient observations, retains only the locator.
                let mut restarted: Checkpoint =
                    serde_json::from_value(serde_json::to_value(&c).unwrap()).unwrap();
                w = Work::default();
                queue_trade_result(&restarted, &mut w);
                assert!(w.trade_result.is_null());
                assert!(request_for(&mut restarted, &mut w, 1_793_916_970_002)
                    .unwrap()
                    .unwrap()["url"]
                    .as_str()
                    .unwrap()
                    .contains(UID));
            }
        }
    }

    #[test]
    fn distinct_partial_fills_preserve_weighted_prices_and_signed_rebates() {
        let (c, w) = trade_result_fixture("TP", "long", |stage, raw| {
            if stage != "result_fills" {
                return;
            }
            let mut first = raw["fill_orders"][0].clone();
            first["volume"] = json!(number(&first["volume"]).unwrap() / 2.0);
            first["commission"] = json!(0.005);
            let mut second = first.clone();
            first["filledTm"] = json!("2026-11-05T22:16:09.500Z");
            second["price"] = json!(number(&second["price"]).unwrap() + 2.0);
            *raw = json!({"fill_orders":[first,second]});
        })
        .unwrap();
        let qty = number(&c.last_trade.as_ref().unwrap()["quantity"]).unwrap();
        let result = w.trade_result["summary"].as_str().unwrap();
        assert!(result.contains("Average entry 97.5 → exit 106"));
        assert!(result.contains("fees/rebates +0.020000"));
        assert!(result.contains(&format!("After trading fees {:+.6}", 8.5 * qty + 0.02)));
    }

    #[test]
    fn fill_timestamps_keep_utc_milliseconds_and_reject_impossible_dates() {
        for (raw, expected) in [
            ("1970-01-01T00:00:00.000Z", 0),
            ("2000-02-29T00:00:00.001Z", 951_782_400_001),
            ("2024-02-29T00:00:00.000Z", 1_709_164_800_000),
            ("2026-11-05T22:16:10.000Z", 1_793_916_970_000),
        ] {
            assert_eq!(fill_time(&json!(raw)).unwrap(), expected);
        }
        for raw in [
            "2026-02-29T00:00:00.000Z",
            "2100-02-29T00:00:00.000Z",
            "2026-00-01T00:00:00.000Z",
            "2026-13-01T00:00:00.000Z",
            "2026-04-31T00:00:00.000Z",
            "2026-11-05T24:00:00.000Z",
            "2026-11-05T22:60:00.000Z",
            "2026-11-05T22:16:60.000Z",
            "2026-11-05T22:16:10.000+08:00",
            "2026-11-05T22:16:10Z",
            "é026-11-05T22:16:10.000Z",
        ] {
            assert!(fill_time(&json!(raw)).is_err(), "{raw}");
        }
    }

    #[test]
    fn unavailable_last_result_keeps_current_management_queue_and_never_creates_a_retry_loop() {
        let mut c = managed("filled", 1.0);
        let mut w = Work {
            pending: Some(json!({"kind":"trade.result.read"})),
            job: Some(json!({"kind":"trade.result.read"})),
            queue: vec![json!({"kind":"order.snapshot.read","scope":"open","symbol":"BTC-USDT"})],
            autonomous: true,
            running: true,
            ..Work::default()
        };
        assert!(unavailable_trade_result(&mut w, "Service unavailable"));
        assert_eq!(w.trade_result["confirmed"], false);
        assert!(w.autonomous && w.running);
        assert_eq!(w.queue.len(), 1);
        let request = request_for(&mut c, &mut w, 100_000_001).unwrap().unwrap();
        assert!(request["url"].as_str().unwrap().contains(UID));
        assert!(!unavailable_trade_result(
            &mut w,
            "Current management failure"
        ));
    }

    #[test]
    fn closure_locator_uses_the_final_fill_quantity_and_never_persists_result_observations() {
        let mut c = managed("partial", 0.5);
        c = observe(c, "PARTIALLY_FILLED", 0.5, true, false, 100_000_000);
        c = observe(c, "FILLED", 1.0, false, false, 100_000_001);
        assert!(c.engine.entry.is_none());
        assert_eq!(
            c.last_trade.as_ref().unwrap()["quantity"],
            json!(crate::tests::lifecycle_state("filled", 1.0)["entry"]["plan"]["quantity"])
        );
        let raw = serde_json::to_string(&c).unwrap();
        assert!(
            !raw.contains("realizedPnl")
                && !raw.contains("trade_result")
                && !raw.contains("fill_orders")
        );
    }

    #[test]
    fn incomplete_foreign_duplicate_and_still_open_trade_evidence_never_claims_a_result() {
        for case in 0..16 {
            let result = trade_result_fixture("TP", "long", |stage, raw| match (case, stage) {
                (0, "uid") => raw["uid"] = json!("987"),
                (1, "result_order") => raw["order"]["clientOrderId"] = json!("foreign"),
                (2, "result_history") => raw["orders"][0]["positionID"] = json!("other"),
                (3, "result_history") => raw["orders"][0]["status"] = json!("PARTIALLY_FILLED"),
                (4, "result_fills") => raw["fill_orders"][0]["volume"] = json!(0.001),
                (5, "result_fills") => raw["fill_orders"][0]["orderId"] = json!("other"),
                (6, "result_fills") => {
                    let row = raw["fill_orders"][0].clone();
                    raw["fill_orders"].as_array_mut().unwrap().push(row);
                }
                (7, "result_fills") => raw["fill_orders"][0]["commission"] = Value::Null,
                (8, "result_positions") => {
                    *raw = json!([{"symbol":"BTC-USDT","positionId":"456","positionSide":"LONG","positionAmt":"0.01","avgPrice":"96.5"}])
                }
                (9, "result_open") => raw["orders"][0]["positionID"] = json!("456"),
                (10, "result_history") => raw["orders"] = json!([]),
                (11, "result_fills") => raw["fill_orders"][0]["currency"] = json!("USDC"),
                (12, "result_fills") => raw["fill_orders"][0]["filledTm"] = Value::Null,
                (13, "result_fills") => {
                    raw["fill_orders"][0]["filledTm"] = json!("2026-11-05T22:16:11.000Z")
                }
                (14, "result_fills") => {
                    raw["fill_orders"][0]["filledTm"] = json!("2026-11-05T22:16:08.000Z")
                }
                (15, "result_fills") => raw["fill_orders"][0]["currency"] = Value::Null,
                _ => {}
            });
            assert!(result.is_err(), "case {case} must not confirm a result");
        }
    }

    #[test]
    fn transient_candle_rejections_have_two_read_only_retries_per_action() {
        let now = 1_800_000_000_000_i64;
        let mut c = Checkpoint::default();
        let original = serde_json::to_value(&c).unwrap();
        let job = json!({"kind":"market.candles.read","symbol":"ACH-USDT", "timeframe":"1m"});
        let mut w = Work {
            job: Some(job),
            ..Work::default()
        };
        let rejection = json!({"status":200,"outcome":"response","body":"{\"code\":100410}"});
        for _ in 0..2 {
            let read = request_for(&mut c, &mut w, now).unwrap().unwrap();
            assert_eq!(read["method"], "GET");
            receive(&mut c, &mut w, &rejection, now).unwrap();
            let wait = request_for(&mut c, &mut w, now).unwrap().unwrap();
            assert_eq!(wait["kind"], "runtime.wait");
            assert_eq!(wait["milliseconds"], 5000);
            receive(&mut c, &mut w, &json!({"id":"read_retry"}), now + 5000).unwrap();
        }
        request_for(&mut c, &mut w, now).unwrap();
        assert!(receive(&mut c, &mut w, &rejection, now).is_err());
        assert_eq!(serde_json::to_value(&c).unwrap(), original);
    }
    #[test]
    fn candle_reads_wait_without_replaying_or_consuming_the_pending_job() {
        let now = 1_800_000_000_000_i64;
        let mut c = Checkpoint::default();
        let mut w = Work {
            job: Some(json!({"kind":"market.candles.read","symbol":"ACH-USDT", "timeframe":"1m"})),
            ..Work::default()
        };
        let quote = request_for(&mut c, &mut w, now).unwrap().unwrap();
        assert_eq!(quote["kind"], "http.request");
        receive(
            &mut c,
            &mut w,
            &json!({"status":200,"outcome":"response",
            "body":json!({"code":0,"data":[[now,100,101,99,100]]}).to_string()}),
            now,
        )
        .unwrap();
        let wait = request_for(&mut c, &mut w, now).unwrap().unwrap();
        assert_eq!(wait["kind"], "runtime.wait");
        assert_eq!(wait["milliseconds"], 1200);
        let context = serde_json::to_value(&w).unwrap();
        let mut w: Work = serde_json::from_value(context.clone()).unwrap();
        receive(&mut c, &mut w, &json!({"id":"candle_wait"}), now + 1200).unwrap();
        assert_eq!(serde_json::to_value(&w).unwrap(), context);
        let history = request_for(&mut c, &mut w, now + 1200).unwrap().unwrap();
        assert_eq!(history["kind"], "http.request");
        assert_ne!(history["id"], quote["id"]);
        assert!(history["url"].as_str().unwrap().contains("startTime="));
    }
    #[test]
    fn market_http_pages_exclude_end_time_and_keep_strict_gap_checks() {
        let current = 1_800_000_000_000_i64;
        let span = 60_000_i64;
        for count in [600, 24, 3] {
            let start = current - count * span;
            let mut c = Checkpoint::default();
            let mut w = Work {
                job: Some(
                    json!({"kind":"market.candles.read","symbol":"ACH-USDT", "timeframe":"1m",
                    "stage":"history","at":current,"first":start,"last":current-span}),
                ),
                ..Work::default()
            };
            let request = request_for(&mut c, &mut w, current).unwrap().unwrap();
            let query: BTreeMap<_, _> = request["url"]
                .as_str()
                .unwrap()
                .split_once('?')
                .unwrap()
                .1
                .split('&')
                .map(|p| p.split_once('=').unwrap())
                .collect();
            let end = (start + CANDLE_PAGE * span).min(current);
            assert_eq!(query["startTime"], start.to_string());
            assert_eq!(query["endTime"], end.to_string());
            let pending = serde_json::to_value(&w).unwrap();
            for gapped in [false, true] {
                let mut w: Work = serde_json::from_value(pending.clone()).unwrap();
                let mut rows: Vec<_> = (start..end)
                    .step_by(span as usize)
                    .map(|t| json!([t, 100, 101, 99, 100]))
                    .collect();
                if gapped {
                    rows.remove(0);
                }
                let result = receive(
                    &mut c,
                    &mut w,
                    &json!({"outcome":"response","status":200,
                    "body":json!({"code":0,"data":rows}).to_string()}),
                    current,
                );
                assert_eq!(result.is_ok(), !gapped);
                if !gapped {
                    assert_eq!(w.rows.len(), count.min(CANDLE_PAGE) as usize);
                    assert!(w.rows.iter().all(|r| r[0].as_i64().unwrap() < current));
                }
            }
        }
    }
    #[test]
    fn exported_api_has_only_common_operations_and_an_opaque_checkpoint() {
        let out = invoke("open", Value::Null, Value::Null, Value::Null, 100_000_000);
        assert_eq!(out["status"], "executed", "{out}");
        assert!(out["result"]["requests"].as_array().unwrap().is_empty());
        let view = &out["result"]["view"];
        assert_eq!(view["actions"][0]["host"], "workspace.authorize");
        assert_eq!(out["result"]["state"]["version"], 2);
        let old: Value =
            serde_json::from_slice(&evaluate_json(br#"{"schema_version":1}"#)).unwrap();
        assert_eq!(old["status"], "rejected");
        let old_state = invoke(
            "open",
            serde_json::to_value(State::new(Settings::default())).unwrap(),
            Value::Null,
            Value::Null,
            100_000_000,
        );
        assert_eq!(old_state["status"], "rejected");
        assert!(old_state["error_message"]
            .as_str()
            .unwrap()
            .contains("not migrated"));
    }

    #[test]
    fn explicit_start_uses_selected_symbol_and_preserves_previous_managed_identity_without_dispatch(
    ) {
        let c = managed("open", 0.0);
        let previous = invoke(
            "present",
            serde_json::to_value(&c).unwrap(),
            Value::Null,
            Value::Null,
            100_000_000,
        );
        let old = serde_json::to_value(&c.engine.entry).unwrap();
        let mut settings = serde_json::to_value(&c.engine.settings).unwrap();
        settings["symbol"] = json!("MNT-USDT");
        settings["entry_margin"] = json!(2.0);
        settings["stop_percent"] = json!(15.0);
        settings["timeframes"] = json!("1m");
        let raw = json!({"api_version":2,"schema_version":1,"plugin_id":PLUGIN_ID,"host_method":"workspace",
            "action":"start","state":c,"settings":settings,"observed_at_ms":100_000_001,"execution":{"running":true}});
        let out: Value =
            serde_json::from_slice(&evaluate_json(&serde_json::to_vec(&raw).unwrap())).unwrap();
        assert_eq!(out["status"], "executed", "{out}");
        assert_eq!(out["result"]["state"]["engine"]["settings"], settings);
        // A remote Start may reuse the previous workflow declaration. Mutable
        // choices belong in settings/fields, not a second copy in its prose.
        assert_eq!(
            out["result"]["view"]["schedule"],
            previous["result"]["view"]["schedule"]
        );
        for field in out["result"]["view"]["fields"].as_array().unwrap() {
            assert_eq!(field["value"], settings[field["id"].as_str().unwrap()]);
        }
        assert!(out["result"]["state"]["engine"]["entry"].is_null());
        assert_eq!(out["result"]["state"]["engine"]["parked"][0]["entry"], old);
        assert!(out["result"]["requests"].as_array().unwrap().is_empty());
    }

    #[test]
    fn public_trading_uses_one_start_stop_workflow_and_keeps_manual_commands_private() {
        let now = 100_000_001;
        let mut c = managed("open", 0.0);
        let saved = serde_json::to_value(&c).unwrap();
        let view = presentation(
            &mut c,
            &mut Work {
                running: true,
                ..Work::default()
            },
            now,
        )
        .unwrap();
        let ids: Vec<_> = view["actions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["stop", "connect", "refresh_order", "list_orders"]);
        assert_eq!(serde_json::to_value(&c).unwrap(), saved);
        for action in ["prepare_entry", "place_entry", "place_exit", "cancel_entry"] {
            let output = invoke(action, saved.clone(), Value::Null, Value::Null, now);
            assert_eq!(output["status"], "rejected");
            assert!(output["result"].is_null());
        }
        let stopped = invoke("stop", saved.clone(), Value::Null, Value::Null, now);
        assert_eq!(stopped["status"], "executed", "{stopped}");
        assert_eq!(stopped["result"]["state"]["engine"], saved["engine"]);
        assert_eq!(stopped["result"]["state"]["new_entries"], false);
        assert!(stopped["result"]["requests"].as_array().unwrap().is_empty());
        let reopened = invoke(
            "present",
            stopped["result"]["state"].clone(),
            Value::Null,
            Value::Null,
            now + 1,
        );
        assert_eq!(reopened["result"]["view"]["actions"][0]["id"], "start");
        assert_eq!(
            reopened["result"]["view"]["actions"][0]["host"],
            "workspace.start"
        );
    }

    #[test]
    fn timeframe_chooser_uses_the_existing_choice_contract_without_reading_private_state() {
        let private =
            json!({"different_package_schema":"keep unchanged","pending":"not ours to interpret"});
        let output = invoke(
            "timeframes",
            private.clone(),
            Value::Null,
            Value::Null,
            100_000_001,
        );
        assert_eq!(output["status"], "executed", "{output}");
        assert_eq!(output["result"]["state"], private);
        assert_eq!(
            output["result"]["view"]["options"],
            json!(["all", "5m", "1m"])
        );
        assert!(output["result"]["requests"].as_array().unwrap().is_empty());
        for key in ["fields", "actions", "columns", "rows"] {
            assert!(output["result"]["view"][key].is_array());
        }
        let view = invoke(
            "present",
            Value::Null,
            Value::Null,
            Value::Null,
            100_000_001,
        );
        let field = view["result"]["view"]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["id"] == "timeframes")
            .unwrap();
        assert_eq!(field["type"], "choice");
        assert_eq!(field["source"]["action"], "timeframes");
        assert_ne!(field["advanced"], true);
    }

    #[test]
    fn raw_service_lifecycle_ignores_manual_orders_but_keeps_partial_exposure() {
        let c = managed("partial", 0.5);
        let c = observe(c, "PARTIALLY_FILLED", 0.5, true, true, 100_000_000);
        assert_eq!(
            number(&e(&c)["entry"]["evidence"]["filled_quantity"]).unwrap(),
            number(&e(&c)["entry"]["plan"]["quantity"]).unwrap() / 2.0
        );
        assert_eq!(e(&c)["entry"]["position"]["position_id"], "456");
        let c = managed("filled", 1.0);
        let c = observe(c, "FILLED", 1.0, false, true, 100_000_000);
        assert!(e(&c)["entry"].is_object());
        let c: Checkpoint = serde_json::from_slice(&serde_json::to_vec(&c).unwrap()).unwrap();
        let c = observe(c, "FILLED", 1.0, false, true, 100_000_001);
        assert!(e(&c)["entry"].is_null());
    }
    #[test]
    fn cancelled_unfilled_entry_retires_without_adopting_an_owner_order() {
        let c = observe(
            managed("open", 0.0),
            "CANCELED",
            0.0,
            false,
            true,
            100_000_000,
        );
        assert!(e(&c)["entry"].is_null());
    }
    #[test]
    fn uncertain_dispatch_identity_survives_restart_and_blocks_a_second_send() {
        let mut c = managed("open", 0.0);
        let plan = e(&c)["entry"]["plan"].clone();
        let intent = entry_id(&plan);
        c.intents.insert(intent.clone(), "order.entry.place".into());
        let saved = serde_json::to_value(&c).unwrap();
        let reopened = invoke("open", saved.clone(), Value::Null, Value::Null, 100_000_001);
        assert!(reopened["result"]["requests"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(reopened["result"]["state"]["intents"], saved["intents"]);
        let mut w = Work {
            job: Some(
                json!({"kind":"entry.dispatch","plan":plan,"hedge":true,"checked_at":100_000_001}),
            ),
            ..Work::default()
        };
        assert!(request_for(&mut c, &mut w, 100_000_001).is_err());
        assert_eq!(c.intents.len(), 1);
    }
    #[test]
    fn restart_before_dispatch_checkpoint_recovers_as_not_sent_but_keeps_uncertain_delivery() {
        for dispatched in [false, true] {
            let mut c = managed("open", 0.0);
            c.engine.entry.as_mut().unwrap().evidence = None;
            let plan = e(&c)["entry"]["plan"].clone();
            if dispatched {
                c.intents
                    .insert(entry_id(&plan), "order.entry.place".into());
            }
            let reopened = invoke(
                "open",
                serde_json::to_value(&c).unwrap(),
                Value::Null,
                Value::Null,
                100_000_001,
            );
            assert_eq!(reopened["status"], "executed");
            let receipt = &reopened["result"]["state"]["engine"]["entry"]["evidence"];
            if dispatched {
                assert!(receipt.is_null());
            } else {
                assert_eq!(receipt["status"], "not_sent");
            }
            assert!(reopened["result"]["requests"]
                .as_array()
                .unwrap()
                .is_empty());
        }
    }

    #[test]
    fn one_unknown_mutation_is_checkpointed_then_only_queried_after_restart() {
        let now = 100_000_001;
        let mut c = managed("open", 0.0);
        c.engine.entry.as_mut().unwrap().evidence = None;
        let plan = e(&c)["entry"]["plan"].clone();
        let mut w = Work {
            job: Some(json!({"kind":"entry.dispatch","plan":plan,"hedge":false,"checked_at":now})),
            ..Work::default()
        };
        let send = request_for(&mut c, &mut w, now).unwrap().unwrap();
        assert_eq!(send["method"], "POST");
        assert!(send["url"].as_str().unwrap().contains("stopLoss="));
        assert!(send["url"].as_str().unwrap().contains("positionSide=BOTH"));
        assert_eq!(c.intents.len(), 1);
        receive(&mut c, &mut w, &json!({"outcome":"unknown"}), now).unwrap();
        let query = request_for(&mut c, &mut w, now).unwrap().unwrap();
        assert_eq!(query["method"], "GET");
        assert!(receive(
            &mut c,
            &mut w,
            &json!({"outcome":"response","status":200,"body":"{\"code\":100004}"}),
            now
        )
        .is_err());
        proven_preflight_failure(&mut c, &mut w, now).unwrap();
        assert!(e(&c)["entry"]["evidence"].is_null());
        let reopened = invoke(
            "open",
            serde_json::to_value(&c).unwrap(),
            Value::Null,
            Value::Null,
            now + 1,
        );
        assert_eq!(reopened["status"], "executed", "{reopened}");
        assert_eq!(
            reopened["result"]["state"]["intents"],
            serde_json::to_value(c.intents).unwrap()
        );
        assert!(reopened["result"]["requests"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["method"] != "POST"));
    }
    #[test]
    fn confirmed_host_non_dispatch_releases_only_the_new_intent() {
        let now = 100_000_001;
        let mut c = managed("open", 0.0);
        c.engine.entry.as_mut().unwrap().evidence = None;
        let mut w = Work {
            job: Some(
                json!({"kind":"entry.dispatch","plan":e(&c)["entry"]["plan"],"hedge":true,"checked_at":now}),
            ),
            ..Work::default()
        };
        request_for(&mut c, &mut w, now).unwrap();
        receive(&mut c, &mut w, &json!({"outcome":"not_dispatched"}), now).unwrap();
        assert!(c.intents.is_empty());
        assert_eq!(e(&c)["entry"]["evidence"]["status"], "not_sent");
    }
    #[test]
    fn account_mismatch_during_new_preflight_is_proven_not_sent() {
        let now = 100_000_001;
        let mut c = managed("open", 0.0);
        c.engine.entry.as_mut().unwrap().evidence = None;
        let mut w = Work {
            job: Some(json!({"kind":"order.entry.place","plan":e(&c)["entry"]["plan"]})),
            ..Work::default()
        };
        let read = request_for(&mut c, &mut w, now).unwrap().unwrap();
        assert_eq!(read["method"], "GET");
        let out = invoke(
            "service_result",
            serde_json::to_value(c).unwrap(),
            serde_json::to_value(w).unwrap(),
            json!({"outcome":"response","status":200,"body":"{\"code\":0,\"data\":{\"uid\":\"999999999\"}}"}),
            now,
        );
        assert_eq!(out["status"], "executed", "{out}");
        assert_eq!(
            out["result"]["state"]["engine"]["entry"]["evidence"]["status"],
            "not_sent"
        );
        assert!(out["result"]["requests"].as_array().unwrap().is_empty());
        assert!(out["result"]["view"]["message"]
            .as_str()
            .unwrap()
            .contains("exact account"));
    }
    #[test]
    fn malformed_partial_and_foreign_service_results_cannot_become_order_evidence() {
        let c = managed("open", 0.0);
        let j = json!({"plan":e(&c)["entry"]["plan"]});
        for raw in [json!({}), json!({"orderId":"999","symbol":"OTHER-USDT"})] {
            assert!(order(&c, &j, &raw, 100_000_000, false).is_err());
        }
        assert!(service_response(&json!({"outcome":"unknown","body":"{}"})).is_err());
        assert!(bars(&json!([[0, 1, 2]])).is_err());
    }

    fn protected_position() -> PositionObservation {
        PositionObservation {
            position_id: "456".into(),
            side: "long".into(),
            quantity: 1.0,
            average_price: 96.5,
        }
    }

    fn stop() -> Value {
        json!({"symbol":"BTC-USDT","orderId":"789","side":"SELL","positionSide":"LONG",
            "type":"STOP_MARKET","status":"NEW","price":"0","stopPrice":"95.5","workingType":"CONTRACT_PRICE",
            "origQty":"1","executedQty":"0","reduceOnly":true})
    }

    #[test]
    fn stop_observation_distinguishes_quantity_scope_and_trigger_without_effects() {
        let p = protected_position();
        assert!(stop_coverage(&json!([stop()]), &p)
            .unwrap()
            .summary
            .contains("Full observed quantity"));
        let mut partial = stop();
        partial["origQty"] = json!("0.5");
        assert!(stop_coverage(&json!([partial]), &p)
            .unwrap()
            .summary
            .contains("Partial observed quantity"));
        let mut close = stop();
        close["closePosition"] = json!("true");
        close["origQty"] = json!("0.1");
        assert!(stop_coverage(&json!([close]), &p)
            .unwrap()
            .summary
            .contains("Full observed quantity"));
        let mut both = stop();
        both["positionSide"] = json!("BOTH");
        both["reduceOnly"] = json!(false);
        assert!(stop_coverage(&json!([both]), &p)
            .unwrap()
            .summary
            .starts_with("No matching"));
        for (field, value) in [
            ("side", json!("BUY")),
            ("positionSide", json!("SHORT")),
            ("positionId", json!("999")),
            ("type", json!("STOP")),
            ("type", json!("TAKE_PROFIT_MARKET")),
            ("status", json!("CANCELED")),
            ("status", json!("FILLED")),
        ] {
            let mut wrong = stop();
            wrong[field] = value;
            assert!(
                stop_coverage(&json!([wrong]), &p)
                    .unwrap()
                    .summary
                    .starts_with("No matching"),
                "{field}"
            );
        }
        let mut short = protected_position();
        short.side = "short".into();
        let mut short_stop = stop();
        short_stop["side"] = json!("BUY");
        short_stop["positionSide"] = json!("SHORT");
        short_stop["stopPrice"] = json!("98");
        short_stop["workingType"] = json!("MARK_PRICE");
        let text = stop_coverage(&json!([short_stop]), &short).unwrap();
        assert!(
            text.summary.contains("Full observed quantity")
                && text.details.contains("98 (MARK_PRICE)")
        );
        let mut duplicate = stop();
        duplicate["orderId"] = json!("790");
        assert!(stop_coverage(&json!([stop(), duplicate]), &p)
            .unwrap()
            .summary
            .starts_with("Multiple matching"));
        assert!(stop_coverage(&json!([stop(), stop()]), &p).is_err());
        let mut malformed = stop();
        malformed["stopPrice"] = json!("NaN");
        assert!(stop_coverage(&json!([malformed]), &p).is_err());
    }

    #[test]
    fn lifecycle_stop_display_is_transient_and_expires_without_authorizing_a_mutation() {
        let now = 100_000_001;
        let mut c = managed("filled", 1.0);
        let plan = e(&c)["entry"]["plan"].clone();
        let quantity = number(&plan["quantity"]).unwrap();
        let mut p = protected_position();
        p.quantity = quantity;
        let mut current_stop = stop();
        current_stop["origQty"] = json!(quantity);
        let mut w = Work {
            job: Some(
                json!({"kind":"order.snapshot.read","scope":"lifecycle","stage":"open",
            "plan":plan,"positions_observed_at_ms":now}),
            ),
            lifecycle: json!({"account_id":c.engine.account.as_ref().unwrap().account_id,"symbol":"BTC-USDT",
                "entry":c.engine.entry.as_ref().unwrap().evidence,"positions":[p]}),
            ..Work::default()
        };
        receive(
            &mut c,
            &mut w,
            &json!({"outcome":"response","status":200,
            "body":json!({"code":0,"data":{"orders":[current_stop]}}).to_string()}),
            now,
        )
        .unwrap();
        assert!(w.queue.is_empty() && c.intents.is_empty());
        let message = presentation(&mut c, &mut w, now).unwrap()["message"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(
            message.contains("Full observed quantity")
                && presentation(&mut c, &mut w, now).unwrap()["details"]
                    .as_str()
                    .unwrap()
                    .contains("order 789"),
            "{message}"
        );
        let state = serde_json::to_value(&c).unwrap();
        assert!(message.contains("beyond the fill-based budget estimate"));
        assert!(!state.to_string().contains("protection") && !state.to_string().contains("789"));
        let resumed: Work = serde_json::from_value(serde_json::to_value(&w).unwrap()).unwrap();
        assert!(resumed.protection.is_some()); // Same action across bounded continuations.
        w.protection.as_mut().unwrap().0.position_id = "999".into();
        assert!(presentation(&mut c, &mut w, now).unwrap()["message"]
            .as_str()
            .unwrap()
            .contains("Stop protection: Unverified"));
        w.protection.as_mut().unwrap().0.position_id = "456".into();
        let mut failed = serde_json::to_value(&w).unwrap();
        failed["job"] =
            json!({"kind":"order.snapshot.read","scope":"lifecycle","stage":"uid","plan":plan});
        let lost = invoke(
            "service_result",
            state.clone(),
            failed,
            json!({"outcome":"unknown"}),
            now,
        );
        assert_eq!(lost["status"], "executed", "{lost}");
        assert!(lost["result"]["view"]["message"]
            .as_str()
            .unwrap()
            .contains("Stop protection: Unverified"));
        assert!(lost["result"]["requests"].as_array().unwrap().is_empty());
        let stale = presentation(&mut c, &mut w, now + 60_001).unwrap();
        assert!(stale["message"]
            .as_str()
            .unwrap()
            .contains("Stop protection: Unverified"));
        let reopened = invoke("present", state, Value::Null, Value::Null, now + 1);
        assert_eq!(reopened["status"], "executed", "{reopened}");
        assert!(reopened["result"]["view"]["message"]
            .as_str()
            .unwrap()
            .contains("Stop protection: Unverified"));
        assert!(reopened["result"]["requests"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn open_orders_keep_their_own_observation_view_beside_a_managed_entry() {
        let mut c = managed("filled", 1.0);
        let account = c.engine.account.as_ref().unwrap().account_id.clone();
        let mut w = Work {
            verified: Some(account),
            job: Some(
                json!({"kind":"order.snapshot.read","scope":"open","stage":"open","symbol":"BTC-USDT"}),
            ),
            ..Work::default()
        };
        receive(&mut c, &mut w, &json!({"outcome":"response","status":200,"body":json!({"code":0,"data":{"orders":[stop()]}}).to_string()}), 100_000_001).unwrap();
        let view = presentation(&mut c, &mut w, 100_000_001).unwrap();
        assert!(view["message"]
            .as_str()
            .unwrap()
            .contains("1 open orders returned"));
        assert_eq!(view["details_title"], "Open orders");
        assert_eq!(view["rows"][0][0], "789");
        assert!(c.intents.is_empty() && w.queue.is_empty());
    }
    #[test]
    fn public_instrument_selection_preserves_opaque_state_and_requires_the_common_ability() {
        let complete_view = |response: &Value| {
            let view = &response["result"]["view"];
            for field in ["title", "message", "details"] {
                assert!(view[field].is_string());
            }
            for field in ["fields", "actions", "columns", "rows"] {
                assert!(view[field].is_array());
            }
        };
        let now = 100_000_001;
        let state = json!({"private_shape":"replacement","unresolved_delivery":"keep"});
        let start = invoke("instruments", state.clone(), Value::Null, Value::Null, now);
        complete_view(&start);
        assert_eq!(start["result"]["state"], state);
        assert_eq!(start["result"]["requests"][0]["kind"], "api.describe");
        let old = invoke(
            "instruments_capabilities",
            state.clone(),
            Value::Null,
            json!({"catalog":{"http.request":{"optional":["max_response_bytes"]}}}),
            now,
        );
        assert_eq!(old["status"], "rejected");
        assert!(old["error_message"]
            .as_str()
            .unwrap()
            .contains("Update Capsule"));
        let available = invoke(
            "instruments_capabilities",
            state.clone(),
            Value::Null,
            json!({"catalog":{"http.request":{"optional":["response_json_paths"],"max_response_json_source_bytes":1048576}}}),
            now,
        );
        assert_eq!(available["result"]["state"], state);
        complete_view(&available);
        let request = &available["result"]["requests"][0];
        assert_eq!(request["method"], "GET");
        assert!(request.get("credential").is_none());
        assert_eq!(
            request["response_json_paths"]["symbols"],
            json!(["data", "*", "symbol"])
        );
        for symbol in ["ACH-USDT", "NEW-USDT"] {
            let result = json!({"outcome":"response","status":200,"json":json!({"code":0,"symbols":[symbol,"BTC-USDT",symbol,"BTC-USDC","invalid-USDT","CLOSED-USDT","INACTIVE-USDT"],"statuses":[1,1,1,1,1,1,25],"open":["true","true","true","true","true","false","true"]})});
            let options = invoke(
                "instruments_result",
                state.clone(),
                Value::Null,
                result,
                now,
            );
            let mut expected = vec![symbol, "BTC-USDT"];
            complete_view(&options);
            expected.sort();
            assert_eq!(options["result"]["view"]["options"], json!(expected));
            assert_eq!(options["result"]["state"], state);
            assert!(options["result"]["requests"].as_array().unwrap().is_empty());
        }
        for result in [
            json!({"outcome":"unknown"}),
            json!({"outcome":"response","status":200,"json":json!({"code":0,"symbols":["BTC-USDT"],"statuses":[],"open":["true"]})}),
            json!({"outcome":"response","status":200,"body":"{bad"}),
            json!({"outcome":"response","status":200,"body":"{\"code\":17,\"symbols\":[\"BTC-USDT\"]}"}),
        ] {
            let failed = invoke(
                "instruments_result",
                state.clone(),
                Value::Null,
                result,
                now,
            );
            assert_eq!(failed["status"], "rejected");
            assert!(failed["result"].is_null());
        }
    }
}
