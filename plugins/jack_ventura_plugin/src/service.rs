//! Package-owned service protocol and workflow. The host sees only capability
//! requests and opaque checkpoints; no service DTO crosses into host policy.
use super::*;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const ORIGIN: &str = "https://open-api.bingx.com";
const UID: &str = "/openApi/account/v1/uid";
const ORDER: &str = "/openApi/swap/v2/trade/order";
const POSITIONS: &str = "/openApi/swap/v2/user/positions";
const OPEN: &str = "/openApi/swap/v2/trade/openOrders";
const LEVERAGE: &str = "/openApi/swap/v2/trade/leverage";
const MODE: &str = "/openApi/swap/v1/positionSide/dual";
const CONTRACT: &str = "/openApi/swap/v2/quote/contracts";
const BALANCE: &str = "/openApi/swap/v3/user/balance";
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
}

impl Default for Checkpoint {
    fn default() -> Self {
        Self {
            version: 2,
            engine: State::new(Settings::default()),
            intents: BTreeMap::new(),
            new_entries: true,
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
    sequence: usize,
    #[serde(default)]
    autonomous: bool,
    #[serde(default)]
    running: bool,
    #[serde(default)]
    orders_view: bool,
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
            "description":format!("Run Jack Ventura for {}. Package settings: margin {} USDT, requested stop {}%. Permit reading the account, placing and cancelling managed orders and reducing exits. The package enforces these settings; network permission is not a host financial-loss guarantee. Existing exchange orders remain after revocation.", c.engine.settings.symbol, c.engine.settings.entry_margin,c.engine.settings.stop_percent)});
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
            format!("Entry prepared · {} {} at {}\nNot sent. Market and account evidence are checked before dispatch.", entry.plan.side.to_uppercase(), entry.plan.quantity, entry.plan.price)
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
    } else if c.engine.retired_before_ms > 0 {
        "Previous position/order is no longer active\nClosure reason and PnL are not inferred from a flat position.".into()
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
    view["message"] = json!(message);
    view["summary"] = json!(format!(
        "BingX · LIVE · {}\n{} · Entry margin {} USDT · Stop budget {}% of margin",
        c.engine.settings.symbol,
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
        evidence,
        stop_details
    ));
    view["details_title"] = json!("Orders and calculation evidence");
    if let Some(actions) = view["actions"].as_array_mut() {
        actions.sort_by_key(|a| match a["id"].as_str().unwrap_or("") {
            "stop" | "refresh_order" => 0,
            "start" => 1,
            "connect" if c.engine.account.is_none() => 0,
            "connect" => 3,
            _ => 2,
        });
        for action in actions {
            action["label"] = match action["id"].as_str() {
                Some("refresh_order") => json!(if c
                    .engine
                    .entry
                    .as_ref()
                    .is_some_and(|e| e.position.is_some())
                {
                    "Refresh position"
                } else {
                    "Refresh order"
                }),
                Some("list_orders") => json!("Open orders"),
                Some("inspect") => json!("Refresh market"),
                Some("start") => json!("Start Jack"),
                _ => action["label"].clone(),
            };
        }
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

fn data(result: &Value) -> R<Value> {
    if result["outcome"] != "response"
        || result["status"]
            .as_u64()
            .is_none_or(|s| !(200..300).contains(&s))
    {
        return Err("Service response unavailable; no blind resubmission".into());
    }
    let raw: Value = serde_json::from_str(result["body"].as_str().ok_or("Missing response body")?)
        .map_err(|_| "Unreadable service response")?;
    if raw["code"] != 0 {
        return Err(format!("Service rejected read: {}", raw["code"]));
    }
    Ok(raw["data"].clone())
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
                return Ok(Some(
                    json!({"kind":"runtime.yield","id":"chunk","continuation":"service_result"}),
                ));
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
                p.insert("limit".into(), "24".into());
                let start = j["first"].as_i64().unwrap() + w.page as i64 * span;
                p.insert("startTime".into(), start.to_string());
                p.insert(
                    "endTime".into(),
                    (start + 23 * span).min(current - span).to_string(),
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
    if result["id"] == "chunk" {
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
    let raw = data(result)?;
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
        } else {
            let start = j["first"].as_i64().unwrap() + w.page as i64 * span;
            let expected = ((j["last"].as_i64().unwrap() - start) / span + 1).clamp(0, 24) as usize;
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

// A choice reads the public service catalog, never the strategy checkpoint.
// Preserve the opaque strategy checkpoint while this independent read runs.
fn instrument_choices(input: &Invocation) -> R<PackageOutput> {
    let mut requests = Vec::new();
    let view = if input.action == "instruments" {
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
            "instruments" | "instruments_capabilities" | "instruments_result"
        ) {
            return instrument_choices(&input);
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
            engine(&mut c, &mut w, "start", None, Some(configured), now)?;
            w.queue.clear();
            w.resume = None;
            w.autonomous = false;
            Ok(())
        } else {
            let settings = input.settings;
            if action == "connect" {
                w.autonomous = false;
            }
            engine(&mut c, &mut w, action, None, settings, now).map(|_| ())
        };
        if let Err(error) = operation {
            proven_preflight_failure(&mut c, &mut w, now)?;
            failure = Some(error);
            w = Work {
                dirty: w.dirty,
                running: enabled,
                ..Work::default()
            };
        }
        let request = if failure.is_none() {
            match request_for(&mut c, &mut w, now) {
                Ok(r) => r,
                Err(error) => {
                    proven_preflight_failure(&mut c, &mut w, now)?;
                    failure = Some(error);
                    w = Work {
                        dirty: w.dirty,
                        running: enabled,
                        ..Work::default()
                    };
                    None
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
        assert!(data(&json!({"outcome":"unknown","body":"{}"})).is_err());
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
        let now = 100_000_001;
        let state = json!({"private_shape":"replacement","unresolved_delivery":"keep"});
        let start = invoke("instruments", state.clone(), Value::Null, Value::Null, now);
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
