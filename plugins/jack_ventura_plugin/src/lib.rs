use serde::{Deserialize, Serialize};
use serde_json::{json, value::RawValue, Value};

pub const PLUGIN_ID: &str = "hivra.contract.jack-ventura.v1";
const TIMEFRAMES: [(&str, i64); 6] = [
    ("1d", 86_400_000),
    ("4h", 14_400_000),
    ("1h", 3_600_000),
    ("30m", 1_800_000),
    ("15m", 900_000),
    ("5m", 300_000),
];
const INPUT_LIMIT: usize = 64 * 1024;
const ENTRY_HISTORY_WINDOW: i64 = 48 * 60 * 60 * 1000;

#[no_mangle]
pub extern "C" fn hivra_plugin_abi_version() -> u32 {
    2
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    symbol: String,
    detection_length: usize,
    margin: f64,
    #[serde(default = "default_entry_margin")]
    entry_margin: f64,
    #[serde(default = "default_stop_percent")]
    stop_percent: f64,
}

fn default_entry_margin() -> f64 {
    1.0
}
fn default_stop_percent() -> f64 {
    20.0
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            symbol: "BTC-USDT".into(),
            detection_length: 7,
            margin: 6.9,
            entry_margin: default_entry_margin(),
            stop_percent: default_stop_percent(),
        }
    }
}

impl Settings {
    fn validate(&self) -> Result<(), String> {
        let parts: Vec<_> = self.symbol.split('-').collect();
        if self.symbol.len() > 64
            || parts.len() != 2
            || parts.iter().any(|p| {
                p.is_empty()
                    || !p
                        .chars()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            })
        {
            return Err("Use an exchange symbol such as BTC-USDT".into());
        }
        if !(3..=13).contains(&self.detection_length)
            || !self.margin.is_finite()
            || !(4.0..=9.0).contains(&self.margin)
        {
            return Err("Detection length must be 3..13 and cluster margin 4..9".into());
        }
        if !self.entry_margin.is_finite()
            || self.entry_margin <= 0.0
            || !self.stop_percent.is_finite()
            || self.stop_percent <= 0.0
            || self.stop_percent > 100.0
        {
            return Err(
                "Margin must be positive; stop must be greater than 0 and at most 100% of margin"
                    .into(),
            );
        }
        Ok(())
    }
}

// Compact candle wire: open time, open, high, low, close. Time is integral.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Candle(i64, f64, f64, f64, f64);

impl Candle {
    fn validate(&self) -> Result<(), String> {
        if self.0 < 0
            || [self.1, self.2, self.3, self.4]
                .iter()
                .any(|p| !p.is_finite() || *p <= 0.0)
            || self.3 > self.1.min(self.4)
            || self.2 < self.1.max(self.4)
            || self.3 > self.2
        {
            return Err("Invalid OHLC candle".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct Swing {
    direction: i8,
    time: i64,
    price: f64,
}

impl Serialize for Swing {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        (self.direction, self.time, self.price).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Swing {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Wire;
        impl<'de> serde::de::Visitor<'de> for Wire {
            type Value = Swing;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a compact swing or previous saved swing")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut a: A) -> Result<Swing, A::Error> {
                let direction = a
                    .next_element()?
                    .ok_or_else(|| serde::de::Error::custom("Missing direction"))?;
                let time = a
                    .next_element()?
                    .ok_or_else(|| serde::de::Error::custom("Missing time"))?;
                let price = a
                    .next_element()?
                    .ok_or_else(|| serde::de::Error::custom("Missing price"))?;
                if a.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::custom("Extra swing field"));
                }
                Ok(Swing {
                    direction,
                    time,
                    price,
                })
            }
            // One-way import of 0.1.6 state; every subsequent write is compact.
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut a: A) -> Result<Swing, A::Error> {
                #[derive(Deserialize)]
                #[serde(field_identifier, rename_all = "snake_case")]
                enum Field {
                    Direction,
                    Time,
                    Price,
                }
                let (mut direction, mut time, mut price) = (None, None, None);
                while let Some(key) = a.next_key::<Field>()? {
                    match key {
                        Field::Direction if direction.is_none() => {
                            direction = Some(a.next_value()?)
                        }
                        Field::Time if time.is_none() => time = Some(a.next_value()?),
                        Field::Price if price.is_none() => price = Some(a.next_value()?),
                        _ => {
                            return Err(serde::de::Error::custom(
                                "Unknown or duplicate swing field",
                            ))
                        }
                    }
                }
                Ok(Swing {
                    direction: direction
                        .ok_or_else(|| serde::de::Error::custom("Missing direction"))?,
                    time: time.ok_or_else(|| serde::de::Error::custom("Missing time"))?,
                    price: price.ok_or_else(|| serde::de::Error::custom("Missing price"))?,
                })
            }
        }
        deserializer.deserialize_any(Wire)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Level {
    origin: i64,
    first_known: i64,
    price: f64,
    top: f64,
    bottom: f64,
    touched: bool,
    #[serde(default)]
    consumed: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Frame {
    timeframe: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    checked_at_ms: Option<i64>,
    formation_start: Option<i64>,
    last_open: Option<i64>,
    atr: Option<f64>,
    seed_sum: f64,
    seed_count: usize,
    bars: Vec<Candle>,
    swings: Vec<Swing>,
    buy: Vec<Level>,
    sell: Vec<Level>,
}

impl Frame {
    fn process(&mut self, bar: Candle, settings: &Settings, duration: i64, form_levels: bool) {
        let tr = match self.bars.last() {
            Some(previous) => (bar.2 - bar.3)
                .max((bar.2 - previous.4).abs())
                .max((bar.3 - previous.4).abs()),
            None => bar.2 - bar.3,
        };
        self.atr = match self.atr {
            Some(atr) => Some((atr * 9.0 + tr) / 10.0),
            None => {
                self.seed_count += 1;
                self.seed_sum += tr;
                (self.seed_count == 10).then_some(self.seed_sum / 10.0)
            }
        };
        self.bars.push(bar.clone());
        let length = settings.detection_length;
        if self.bars.len() >= length + 2 {
            let pivot = self.bars[self.bars.len() - 2].clone();
            let left = &self.bars[self.bars.len() - length - 2..self.bars.len() - 2];
            // Pine selects the most recent equal extreme: ties on the left
            // are allowed, but an equal high/low on the right is not a pivot.
            let high = pivot.2 > bar.2 && left.iter().all(|b| pivot.2 >= b.2);
            let low = pivot.3 < bar.3 && left.iter().all(|b| pivot.3 <= b.3);
            for (exists, direction, price) in [(high, 1, pivot.2), (low, -1, pivot.3)] {
                if !exists {
                    continue;
                }
                match self.swings.first_mut() {
                    Some(last) if last.direction == direction => {
                        if (direction == 1 && price > last.price)
                            || (direction == -1 && price < last.price)
                        {
                            *last = Swing {
                                direction,
                                time: pivot.0,
                                price,
                            };
                        }
                    }
                    _ => {
                        self.swings.insert(
                            0,
                            Swing {
                                direction,
                                time: pivot.0,
                                price,
                            },
                        );
                        self.swings.truncate(50);
                    }
                }
                if form_levels {
                    if let Some(atr) = self.atr {
                        self.cluster(
                            direction,
                            price,
                            atr * settings.margin / 10.0,
                            bar.0 + duration,
                        );
                    }
                }
            }
        }
        for level in &mut self.buy {
            level.touched |= bar.0 >= level.first_known && bar.2 >= level.price;
        }
        for level in &mut self.sell {
            level.touched |= bar.0 >= level.first_known && bar.3 <= level.price;
        }
        self.last_open = Some(bar.0);
        if self.bars.len() > length + 1 {
            self.bars.remove(0);
        }
    }

    fn cluster(&mut self, direction: i8, pivot: f64, width: f64, known: i64) {
        let mut matches = Vec::new();
        for point in &self.swings {
            if point.direction != direction {
                continue;
            }
            if (direction == 1 && point.price > pivot + width)
                || (direction == -1 && point.price < pivot - width)
            {
                break;
            }
            if point.price > pivot - width && point.price < pivot + width {
                matches.push(point);
            }
        }
        if matches.len() < 3 {
            return;
        }
        let last = matches.last().unwrap();
        let origin = last.time;
        let price = last.price;
        let highest = matches
            .iter()
            .map(|m| m.price)
            .fold(f64::NEG_INFINITY, f64::max);
        let lowest = matches
            .iter()
            .map(|m| m.price)
            .fold(f64::INFINITY, f64::min);
        let center = (highest + lowest) / 2.0;
        let levels = if direction == 1 {
            &mut self.buy
        } else {
            &mut self.sell
        };
        if let Some(level) = levels.first_mut().filter(|l| l.origin == origin) {
            level.price = price;
            level.top = center + width;
            level.bottom = center - width;
        } else {
            levels.insert(
                0,
                Level {
                    origin,
                    first_known: known,
                    price,
                    top: center + width,
                    bottom: center - width,
                    touched: false,
                    consumed: false,
                },
            );
            levels.truncate(3);
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    version: u32,
    settings: Settings,
    frames: Vec<Frame>,
    current_price: Option<f64>,
    observed_at: Option<i64>,
    complete: bool,
    #[serde(default)]
    account: Option<AccountSnapshot>,
    #[serde(default)]
    entry: Option<ManagedEntry>,
    #[serde(default)]
    retired_before_ms: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    parked: Vec<ParkedInstrument>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ParkedInstrument {
    settings: Settings,
    account: AccountSnapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    entry: Option<ManagedEntry>,
    retired_before_ms: i64,
}

impl State {
    fn select_instrument(&mut self, settings: Settings) -> Result<(), String> {
        if self.settings.symbol == settings.symbol {
            return Ok(());
        }
        let entry = self.entry.as_ref().filter(|e| e.attempted && !e.not_sent());
        if entry.is_some() || self.retired_before_ms > 0 {
            if self.parked.len() >= 4
                && !self
                    .parked
                    .iter()
                    .any(|p| p.settings.symbol == settings.symbol)
            {
                return Err("Reconcile a saved instrument before selecting another".into());
            }
            self.parked.push(ParkedInstrument {
                settings: self.settings.clone(),
                account: self
                    .account
                    .clone()
                    .ok_or("Saved instrument has no account")?,
                entry: entry.cloned(),
                retired_before_ms: self.retired_before_ms,
            });
        }
        let account = self.account.clone();
        let mut next = State::new(settings.clone());
        next.parked = std::mem::take(&mut self.parked);
        if let Some(index) = next
            .parked
            .iter()
            .position(|p| p.settings.symbol == settings.symbol)
        {
            let saved = next.parked.remove(index);
            if account
                .as_ref()
                .is_some_and(|a| a.account_id != saved.account.account_id)
            {
                return Err("Saved instrument belongs to another account".into());
            }
            next.settings = saved.settings;
            next.account = Some(saved.account);
            next.entry = saved.entry;
            next.retired_before_ms = saved.retired_before_ms;
        } else {
            // Keep only the account identity until a fresh snapshot for the
            // selected instrument replaces this old-symbol observation.
            next.account = account;
        }
        *self = next;
        Ok(())
    }

    fn request_market(&mut self, requests: &mut Vec<Value>) {
        self.complete = false;
        for frame in &mut self.frames {
            frame.checked_at_ms = None;
        }
        for (tf, _) in TIMEFRAMES {
            requests.push(json!({"kind":"market.candles.read", "provider":"bingx",
                "symbol":self.settings.symbol, "timeframe":tf, "limit":600}));
        }
    }

    fn request_recovery(&self, requests: &mut Vec<Value>) {
        if let Some(account) = &self.account {
            if account.symbol != self.settings.symbol {
                requests.push(json!({"kind":"account.snapshot.read", "provider":"bingx",
                    "symbol":self.settings.symbol, "account_id":account.account_id}));
                return;
            }
            if !self.entry.as_ref().is_some_and(|e| e.attempted) {
                requests.push(json!({"kind":"order.snapshot.read", "scope":"durable",
                    "provider":"bingx", "account_id":account.account_id}));
            }
        }
    }

    fn new(settings: Settings) -> Self {
        Self {
            version: 1,
            settings,
            frames: Vec::new(),
            current_price: None,
            observed_at: None,
            complete: false,
            account: None,
            entry: None,
            retired_before_ms: 0,
            parked: Vec::new(),
        }
    }

    fn validate(&self) -> Result<(), String> {
        self.settings.validate()?;
        if let Some(account) = &self.account {
            account.validate()?;
        }
        if let Some(entry) = &self.entry {
            entry.validate()?;
            if self.account.as_ref().map(|a| &a.account_id) != Some(&entry.plan.account_id)
                || self.settings.symbol != entry.plan.symbol
                || entry.plan.prepared_at_ms <= self.retired_before_ms
            {
                return Err("Managed entry belongs to another account or instrument".into());
            }
        }
        if self.version != 1 || self.frames.len() > 6 || self.retired_before_ms < 0 {
            return Err("Unsupported plugin state".into());
        }
        if self.parked.len() > 4 {
            return Err("Too many saved instruments".into());
        }
        let mut symbols = std::collections::BTreeSet::new();
        symbols.insert(self.settings.symbol.as_str());
        for saved in &self.parked {
            if !symbols.insert(saved.settings.symbol.as_str())
                || saved
                    .entry
                    .as_ref()
                    .is_some_and(|entry| !entry.attempted || entry.not_sent())
                || (saved.entry.is_none() && saved.retired_before_ms == 0)
                || saved.account.symbol != saved.settings.symbol
            {
                return Err("Invalid saved instrument".into());
            }
            State {
                version: 1,
                settings: saved.settings.clone(),
                frames: Vec::new(),
                current_price: None,
                observed_at: None,
                complete: false,
                account: Some(saved.account.clone()),
                entry: saved.entry.clone(),
                retired_before_ms: saved.retired_before_ms,
                parked: Vec::new(),
            }
            .validate()?;
        }
        let mut seen = std::collections::BTreeSet::new();
        for frame in &self.frames {
            let duration = duration(&frame.timeframe)?;
            if !seen.insert(&frame.timeframe)
                || frame.bars.len() > self.settings.detection_length + 1
                || frame.swings.len() > 50
                || frame.buy.len() > 3
                || frame.sell.len() > 3
                || frame.seed_count > 10
                || !frame.seed_sum.is_finite()
                || frame.atr.is_some_and(|v| !v.is_finite() || v < 0.0)
                || frame.checked_at_ms.is_some_and(|t| {
                    t <= 0
                        || self.observed_at.is_none_or(|quote| t > quote)
                        || frame.last_open != Some(t - t % duration - duration)
                })
            {
                return Err("Invalid bounded frame state".into());
            }
            for bar in &frame.bars {
                bar.validate()?;
            }
            if frame.bars.windows(2).any(|b| b[1].0 - b[0].0 != duration)
                || frame.bars.last().map(|b| b.0) != frame.last_open
            {
                return Err("Invalid frame continuity".into());
            }
            for level in frame.buy.iter().chain(&frame.sell) {
                if ![level.price, level.top, level.bottom]
                    .iter()
                    .all(|v| v.is_finite())
                    || level.price <= 0.0
                    || level.top < level.bottom
                    || level.origin >= level.first_known
                {
                    return Err("Invalid saved level".into());
                }
            }
        }
        Ok(())
    }
}

fn duration(timeframe: &str) -> Result<i64, String> {
    TIMEFRAMES
        .iter()
        .find(|(tf, _)| *tf == timeframe)
        .map(|(_, d)| *d)
        .ok_or_else(|| "Unsupported timeframe".into())
}

#[derive(Deserialize)]
struct Input<'a> {
    schema_version: u32,
    plugin_id: String,
    host_method: String,
    action: String,
    observed_at_ms: i64,
    #[serde(borrow)]
    state: Option<&'a RawValue>,
    settings: Option<Settings>,
    execution: Option<ExecutionProjection>,
    #[serde(borrow)]
    snapshot: Option<&'a RawValue>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecutionProjection {
    account_id: String,
    symbol: String,
    allow_new_entries: bool,
}

impl ExecutionProjection {
    fn validate_scope(&self, state: &State) -> Result<(), String> {
        if state.account.as_ref().map(|a| a.account_id.as_str()) != Some(self.account_id.as_str())
            || state.settings.symbol != self.symbol
        {
            return Err("Execution authority belongs to another account or instrument".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountSnapshot {
    symbol: String,
    endpoint: String,
    account_id: String,
    account_label: String,
    available_margin: f64,
    long_leverage: u32,
    short_leverage: u32,
    price_precision: u32,
    quantity_precision: u32,
    min_quantity: f64,
    min_notional: f64,
    observed_at_ms: i64,
}

impl AccountSnapshot {
    fn validate(&self) -> Result<(), String> {
        if self.endpoint != "LIVE"
            || self.account_id.len() != 64
            || !self
                .account_id
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            || self.account_label.len() > 128
            || self.account_label.is_empty()
            || !self.symbol.ends_with("-USDT")
            || self.symbol.len() > 64
            || self.long_leverage == 0
            || self.short_leverage == 0
            || self.long_leverage > 10000
            || self.short_leverage > 10000
            || self.price_precision > 12
            || self.quantity_precision > 12
            || self.observed_at_ms <= 0
            || [self.available_margin, self.min_quantity, self.min_notional]
                .iter()
                .any(|v| !v.is_finite() || *v < 0.0)
        {
            return Err("Invalid BingX account evidence".into());
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct Snapshot {
    symbol: String,
    timeframe: String,
    candles: Vec<Candle>,
    current_price: f64,
    history_start_ms: Option<i64>,
    history_end_ms: Option<i64>,
    batch_complete: Option<bool>,
    current_candle: Option<Candle>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EntryPlan {
    account_id: String,
    symbol: String,
    side: String,
    timeframe: String,
    origin: i64,
    first_known: i64,
    line_price: f64,
    price: f64,
    quantity: f64,
    stop_price: f64,
    margin: f64,
    leverage: u32,
    prepared_at_ms: i64,
    expires_at_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagedEntry {
    plan: EntryPlan,
    attempted: bool,
    evidence: Option<OrderEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    validation: Option<(i64, i64)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    observed_position_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lifecycle_observed_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    flat_observed_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    position: Option<PositionObservation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    profit_exit: Option<ManagedExit>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExitPlan {
    entry_plan: EntryPlan,
    position_id: String,
    quantity: f64,
    average_price: f64,
    price: f64,
    prepared_at_ms: i64,
    expires_at_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagedExit {
    plan: ExitPlan,
    attempted: bool,
    evidence: Option<OrderEvidence>,
}

impl ManagedExit {
    fn request(&self) -> Value {
        json!({"kind":"position.exit.place", "provider":"bingx", "plan":self.plan})
    }

    fn observe(&mut self, evidence: OrderEvidence, now: i64) -> Result<(), String> {
        let p = &self.plan;
        if evidence.account_id != p.entry_plan.account_id
            || evidence.symbol != p.entry_plan.symbol
            || evidence.client_order_id.len() != 40
            || !evidence
                .client_order_id
                .bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            || evidence.observed_at_ms > now
            || now - evidence.observed_at_ms > 60_000
            || evidence.observed_at_ms < p.prepared_at_ms
            || !evidence.filled_quantity.is_finite()
            || evidence.filled_quantity < 0.0
            || evidence.filled_quantity > p.quantity
            || !evidence.average_price.is_finite()
            || evidence.average_price < 0.0
            || (evidence.filled_quantity > 0.0 && evidence.average_price <= 0.0)
            || ![
                "open",
                "partial",
                "filled",
                "cancelled",
                "expired",
                "rejected",
                "not_sent",
            ]
            .contains(&evidence.status.as_str())
            || (evidence.status == "filled" && evidence.filled_quantity != p.quantity)
            || (evidence.status == "partial"
                && (evidence.filled_quantity == 0.0 || evidence.filled_quantity == p.quantity))
            || evidence.order_id.as_ref().is_some_and(|id| {
                id.is_empty() || id.len() > 30 || !id.bytes().all(|c| c.is_ascii_digit())
            })
            || (evidence.order_id.is_none()
                && !["not_sent", "rejected"].contains(&evidence.status.as_str()))
            || (evidence.order_id.is_none() && evidence.filled_quantity != 0.0)
            || evidence
                .error_message
                .as_ref()
                .is_some_and(|m| m.len() > 2048)
            || self.evidence.as_ref().is_some_and(|old| {
                evidence.observed_at_ms < old.observed_at_ms
                    || evidence.client_order_id != old.client_order_id
                    || (old.order_id.is_some() && old.order_id != evidence.order_id)
                    || evidence.filled_quantity < old.filled_quantity
                    || (["filled", "cancelled", "expired", "rejected"]
                        .contains(&old.status.as_str())
                        && evidence.status != old.status)
            })
        {
            return Err("Exit evidence changed identity, fill history or scope".into());
        }
        self.evidence = Some(evidence);
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LifecycleSnapshot {
    account_id: String,
    symbol: String,
    entry: OrderEvidence,
    positions: Vec<PositionObservation>,
    orders: Vec<OpenOrder>,
    observed_at_ms: i64,
    #[serde(default)]
    exit: Option<OrderEvidence>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PositionObservation {
    position_id: String,
    side: String,
    quantity: f64,
    average_price: f64,
}

impl PositionObservation {
    fn validate(&self) -> Result<(), String> {
        if self.position_id.is_empty()
            || self.position_id.len() > 30
            || !self.position_id.bytes().all(|c| c.is_ascii_digit())
            || !["long", "short"].contains(&self.side.as_str())
            || !self.quantity.is_finite()
            || self.quantity <= 0.0
            || !self.average_price.is_finite()
            || self.average_price <= 0.0
        {
            return Err("Invalid current position evidence".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OrderEvidence {
    account_id: String,
    symbol: String,
    client_order_id: String,
    order_id: Option<String>,
    status: String,
    filled_quantity: f64,
    average_price: f64,
    observed_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error_message: Option<String>,
}

// Read-only provider observations, never adopted as a managed entry or saved history.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenOrdersSnapshot {
    account_id: String,
    symbol: String,
    observed_at_ms: i64,
    orders: Vec<OpenOrder>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenOrder {
    order_id: String,
    side: String,
    position_side: String,
    r#type: String,
    status: String,
    price: String,
    stop_price: String,
    quantity: String,
    filled_quantity: String,
}

impl ManagedEntry {
    fn not_sent(&self) -> bool {
        self.evidence
            .as_ref()
            .is_some_and(|e| e.status == "not_sent")
    }

    fn validate(&self) -> Result<(), String> {
        let p = &self.plan;
        duration(&p.timeframe)?;
        if p.account_id.len() != 64
            || !p
                .account_id
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            || !["long", "short"].contains(&p.side.as_str())
            || p.symbol.len() > 64
            || !p.symbol.ends_with("-USDT")
            || p.origin < 0
            || p.first_known <= 0
            || p.prepared_at_ms <= 0
            || p.origin >= p.first_known
            || p.first_known > p.prepared_at_ms
            || p.expires_at_ms
                .checked_sub(p.prepared_at_ms)
                .is_none_or(|age| age <= 0 || age > ENTRY_HISTORY_WINDOW)
            || p.leverage == 0
            || [p.line_price, p.price, p.quantity, p.stop_price, p.margin]
                .iter()
                .any(|n| !n.is_finite() || *n <= 0.0)
            || (p.side == "long" && (p.price > p.line_price || p.stop_price >= p.price))
            || (p.side == "short" && (p.price < p.line_price || p.stop_price <= p.price))
        {
            return Err("Invalid managed entry".into());
        }
        if let Some(e) = &self.evidence {
            if e.account_id != p.account_id
                || e.symbol != p.symbol
                || e.client_order_id.len() != 40
                || !e
                    .client_order_id
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
                || e.order_id.as_ref().is_some_and(|id| {
                    id.is_empty() || id.len() > 30 || !id.bytes().all(|c| c.is_ascii_digit())
                })
                || ![
                    "unknown",
                    "not_sent",
                    "open",
                    "partial",
                    "filled",
                    "cancelled",
                    "rejected",
                    "expired",
                ]
                .contains(&e.status.as_str())
                || !e.filled_quantity.is_finite()
                || e.filled_quantity < 0.0
                || e.filled_quantity > p.quantity
                || !e.average_price.is_finite()
                || e.average_price < 0.0
                || (e.filled_quantity > 0.0 && e.average_price <= 0.0)
                || (e.status == "filled" && e.filled_quantity != p.quantity)
                || (e.status == "partial"
                    && (e.filled_quantity <= 0.0 || e.filled_quantity >= p.quantity))
                || e.observed_at_ms <= 0
                || (e.status == "not_sent" && (e.order_id.is_some() || e.filled_quantity != 0.0))
            {
                return Err("Invalid exact-order evidence".into());
            }
        }
        if self.observed_position_id.as_ref().is_some_and(|id| {
            id.is_empty() || id.len() > 30 || !id.bytes().all(|c| c.is_ascii_digit())
        }) || self.lifecycle_observed_at_ms.is_some_and(|t| t <= 0)
            || self.flat_observed_at_ms.is_some_and(|t| t <= 0)
        {
            return Err("Invalid saved position observation".into());
        }
        if let Some(position) = &self.position {
            position.validate()?;
            let evidence = self
                .evidence
                .as_ref()
                .ok_or("Position has no entry evidence")?;
            if !self.attempted
                || self.observed_position_id.as_ref() != Some(&position.position_id)
                || position.side != p.side
                || position.quantity > evidence.filled_quantity
                || self
                    .lifecycle_observed_at_ms
                    .is_none_or(|t| t < evidence.observed_at_ms)
                || !same_price(position.average_price, evidence.average_price)
            {
                return Err("Position does not match the managed fill".into());
            }
        }
        if let Some(exit) = &self.profit_exit {
            let e = &exit.plan;
            if e.entry_plan != *p
                || self.observed_position_id.as_ref() != Some(&e.position_id)
                || e.prepared_at_ms < p.prepared_at_ms
                || e.expires_at_ms - e.prepared_at_ms != 60_000
                || [e.quantity, e.average_price, e.price]
                    .iter()
                    .any(|n| !n.is_finite() || *n <= 0.0)
                || e.quantity > p.quantity
                || (p.side == "long" && e.price <= e.average_price)
                || (p.side == "short" && e.price >= e.average_price)
            {
                return Err("Invalid saved profit exit".into());
            }
            if let Some(evidence) = &exit.evidence {
                let mut checked = exit.clone();
                checked.evidence = None;
                checked.observe(evidence.clone(), evidence.observed_at_ms)?;
            }
        }
        Ok(())
    }

    fn request(&self, kind: &str) -> Value {
        json!({"kind":kind, "provider":"bingx", "plan":self.plan})
    }

    fn observe_order(&mut self, evidence: OrderEvidence, observed: i64) -> Result<(), String> {
        if evidence.observed_at_ms > observed
            || evidence
                .error_message
                .as_ref()
                .is_some_and(|m| m.len() > 2048)
            || observed - evidence.observed_at_ms > 60_000
            || self.evidence.as_ref().is_some_and(|old| {
                evidence.observed_at_ms < old.observed_at_ms
                    || evidence.client_order_id != old.client_order_id
                    || (old.order_id.is_some()
                        && evidence.order_id.is_some()
                        && old.order_id != evidence.order_id)
                    || evidence.filled_quantity < old.filled_quantity
                    || (matches!(
                        old.status.as_str(),
                        "filled" | "cancelled" | "rejected" | "expired"
                    ) && matches!(evidence.status.as_str(), "open" | "partial" | "unknown"))
            })
        {
            return Err("Order evidence is stale or changed identity/fill history".into());
        }
        // A new exact-order read does not refresh a previously observed position.
        self.position = None;
        self.evidence = Some(evidence);
        self.validate()
    }
}

#[derive(Serialize)]
struct Output<'a> {
    state: SavedState<'a>,
    view: WorkspaceView,
    requests: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resume_action: Option<&'static str>,
}

// Serialize changing state only once, as part of the final envelope. Pure
// observations retain the original bytes without a float round trip.
#[derive(Serialize)]
#[serde(untagged)]
enum SavedState<'a> {
    Unchanged(&'a RawValue),
    Updated(State),
}

#[derive(Serialize)]
#[serde(untagged)]
enum WorkspaceView {
    Trading(Value),
    Orders(OpenOrdersView),
}

#[derive(Serialize)]
struct OpenOrdersView {
    title: &'static str,
    fields: [Value; 0],
    confirmation: Option<Value>,
    summary: String,
    message: String,
    actions: [Value; 2],
    details_title: &'static str,
    columns: [&'static str; 6],
    rows: Vec<[String; 6]>,
    details: String,
}

#[derive(Serialize)]
struct Envelope<'a> {
    schema_version: u32,
    status: &'static str,
    result: Option<Output<'a>>,
    error_code: Option<&'static str>,
    error_message: Option<String>,
}

fn evaluate(input: Input<'_>) -> Result<Output<'_>, String> {
    if input.schema_version != 1 || input.plugin_id != PLUGIN_ID || input.host_method != "workspace"
    {
        return Err("Unsupported invocation".into());
    }
    let observed = input.observed_at_ms;
    if observed <= 0 {
        return Err("Invalid observation time".into());
    }
    if matches!(input.action.as_str(), "list_orders" | "open_orders") {
        return observe_orders(input);
    }
    let mut state = match input.state {
        Some(raw) => serde_json::from_str::<State>(raw.get()).map_err(|e| e.to_string())?,
        None => State::new(Settings::default()),
    };
    state.validate()?;
    let mut requests = Vec::new();
    let mut resume_action = None;
    match input.action.as_str() {
        "open" => state.request_recovery(&mut requests),
        "present" => {}
        "inspect" | "connect" | "prepare_entry" | "start" => {
            let mut settings = input.settings.ok_or("Missing settings")?;
            settings.validate()?;
            let switching = settings.symbol != state.settings.symbol;
            if switching {
                if input.action == "connect"
                    || input
                        .execution
                        .as_ref()
                        .is_some_and(|e| e.allow_new_entries)
                {
                    return Err("Stop local entries before changing instrument".into());
                }
                state.select_instrument(settings.clone())?;
                if state
                    .entry
                    .as_ref()
                    .is_some_and(|e| e.attempted && !e.not_sent())
                {
                    settings = state.settings.clone();
                }
            }
            if state
                .entry
                .as_ref()
                .is_some_and(|e| e.attempted && !e.not_sent())
                && (settings.symbol == state.settings.symbol
                    && (settings != state.settings || input.action == "connect"))
            {
                return Err(
                    "Reconcile the managed order before changing its account or settings".into(),
                );
            }
            if state
                .entry
                .as_ref()
                .is_some_and(|e| !e.attempted || e.not_sent())
            {
                state.entry = None;
            }
            if state.settings.symbol != settings.symbol
                || state.settings.detection_length != settings.detection_length
                || state.settings.margin != settings.margin
            {
                let account = state.account.take();
                let retired_before_ms = state.retired_before_ms;
                state = State::new(settings.clone());
                state.account = account;
                state.retired_before_ms = retired_before_ms;
            }
            state.settings = settings;
            if input.action == "connect" {
                requests.push(json!({"kind":"account.connect", "provider":"bingx", "symbol":state.settings.symbol}));
            } else {
                state.request_market(&mut requests);
                resume_action = Some(if input.action == "prepare_entry" {
                    "prepare"
                } else {
                    "present"
                });
                if state.account.is_some() {
                    requests.push(json!({"kind":"account.snapshot.read", "provider":"bingx", "symbol":state.settings.symbol,
                        "account_id":state.account.as_ref().unwrap().account_id}));
                }
            }
        }
        "prepare" => prepare_entry(&mut state, observed)?,
        "cycle" => {
            let execution = input
                .execution
                .as_ref()
                .ok_or("Start has not been authorized")?;
            execution.validate_scope(&state)?;
            if let Some(entry) = state
                .entry
                .as_ref()
                .filter(|e| e.attempted && !e.not_sent())
            {
                let mut read = entry.request("order.snapshot.read");
                read["scope"] = json!("lifecycle");
                if let Some(exit) = entry.profit_exit.as_ref().filter(|e| e.attempted) {
                    read["exit_plan"] = json!(exit.plan);
                }
                requests.push(read);
                state.request_market(&mut requests);
                resume_action = Some("exit_ready");
                // Closure does not immediately reuse stale account evidence.
                // The next cycle loads a complete market/account observation.
            } else {
                state.entry = None;
                state.request_market(&mut requests);
                requests.push(json!({"kind":"account.snapshot.read", "provider":"bingx",
                    "symbol":state.settings.symbol, "account_id":execution.account_id}));
                resume_action = Some("cycle_ready");
            }
        }
        "cycle_ready" => {
            let execution = input
                .execution
                .as_ref()
                .ok_or("Start has not been authorized")?;
            execution.validate_scope(&state)?;
            if execution.allow_new_entries && state.entry.is_none() && candidate(&state).is_some() {
                prepare_entry(&mut state, observed)?;
            }
        }
        "cancel_entry" | "validate_cancel" => {
            let cancellation = pending_cancellation(&state, observed).ok_or(
                "Refresh the exact unfilled entry and invalidated line before cancellation",
            )?;
            if input.action == "cancel_entry" {
                requests.push(cancellation);
            }
        }
        "exit_ready" => {
            if let Some(plan) = profit_exit_plan(&state, observed) {
                state.entry.as_mut().unwrap().profit_exit = Some(ManagedExit {
                    plan,
                    attempted: false,
                    evidence: None,
                });
            }
        }
        "place_exit" => {
            let exit = state
                .entry
                .as_mut()
                .and_then(|e| e.profit_exit.as_mut())
                .filter(|e| {
                    !e.attempted
                        && observed >= e.plan.prepared_at_ms
                        && observed <= e.plan.expires_at_ms
                })
                .ok_or("Refresh the opposite-line exit before sending it")?;
            exit.attempted = true;
            requests.push(exit.request());
        }
        "validate_exit" => {
            let entry = state.entry.as_ref().ok_or("No managed entry")?;
            let exit = entry
                .profit_exit
                .as_ref()
                .ok_or("No exit awaiting delivery")?;
            let position = entry
                .position
                .as_ref()
                .ok_or("No matching current position")?;
            if !exit.attempted
                || exit.evidence.is_some()
                || observed < exit.plan.prepared_at_ms
                || observed > exit.plan.expires_at_ms
                || entry
                    .lifecycle_observed_at_ms
                    .is_none_or(|t| t > observed || observed - t > 60_000)
                || position.position_id != exit.plan.position_id
                || position.quantity != exit.plan.quantity
                || !same_price(position.average_price, exit.plan.average_price)
                || profit_candidate(&state, position, observed).is_none_or(|(_, line)| {
                    let scale = 10f64.powi(state.account.as_ref().unwrap().price_precision as i32);
                    let rounded = if position.side == "long" {
                        (line.price * scale).floor() / scale
                    } else {
                        (line.price * scale).ceil() / scale
                    };
                    rounded != exit.plan.price
                })
            {
                return Err("Exit position or opposite line changed; no exit sent".into());
            }
        }
        "exit" => {
            let evidence: OrderEvidence =
                serde_json::from_str(input.snapshot.ok_or("Missing exit evidence")?.get())
                    .map_err(|_| "Invalid exit evidence")?;
            let entry = state.entry.as_mut().ok_or("No managed entry")?;
            entry
                .profit_exit
                .as_mut()
                .ok_or("No managed exit")?
                .observe(evidence, observed)?;
            if entry
                .profit_exit
                .as_ref()
                .unwrap()
                .evidence
                .as_ref()
                .unwrap()
                .order_id
                .is_none()
            {
                // Proven rejection/non-dispatch permits a fresh candidate, never
                // a repeat of an exit with an uncertain provider outcome.
                entry.profit_exit = None;
            }
        }
        "place_entry" => {
            if input
                .settings
                .as_ref()
                .is_some_and(|s| s != &state.settings)
            {
                return Err("Settings changed; prepare an entry again".into());
            }
            let current_price = state.current_price.ok_or("No current market quote")?;
            let entry = state.entry.as_mut().ok_or("Prepare an entry first")?;
            if entry.attempted && !entry.not_sent() {
                return Err("This entry was already attempted; refresh its exact order".into());
            }
            if observed > entry.plan.expires_at_ms || observed < entry.plan.prepared_at_ms {
                return Err("Draft history window expired; prepare a fresh entry".into());
            }
            if (entry.plan.side == "long" && current_price <= entry.plan.price)
                || (entry.plan.side == "short" && current_price >= entry.plan.price)
            {
                return Err("Price already passed this entry; prepare a fresh entry".into());
            }
            entry.attempted = true;
            entry.evidence = None;
            requests.push(entry.request("order.entry.place"));
        }
        "refresh_order" => {
            let switching = input
                .settings
                .as_ref()
                .is_some_and(|s| s.symbol != state.settings.symbol);
            if switching {
                if input
                    .execution
                    .as_ref()
                    .is_some_and(|e| e.allow_new_entries)
                {
                    return Err("Stop local entries before changing instrument".into());
                }
                let settings = input.settings.ok_or("Missing settings")?;
                settings.validate()?;
                state.select_instrument(settings)?;
                state.request_market(&mut requests);
                if let Some(account) = &state.account {
                    requests.push(json!({"kind":"account.snapshot.read", "provider":"bingx",
                        "symbol":state.settings.symbol, "account_id":account.account_id}));
                }
                resume_action = Some("present");
            } else {
                if input
                    .settings
                    .as_ref()
                    .is_some_and(|s| s != &state.settings)
                {
                    return Err("Keep the managed order's settings while refreshing it".into());
                }
                let entry = state
                    .entry
                    .as_ref()
                    .filter(|e| e.attempted)
                    .ok_or("No attempted entry")?;
                let mut read = entry.request("order.snapshot.read");
                read["scope"] = json!("lifecycle");
                if let Some(exit) = entry.profit_exit.as_ref().filter(|e| e.attempted) {
                    read["exit_plan"] = json!(exit.plan);
                }
                requests.push(read);
                state.request_market(&mut requests);
                resume_action = Some("exit_ready");
            }
        }
        "validate_entry" => {
            let entry = state
                .entry
                .as_mut()
                .filter(|e| e.attempted)
                .ok_or("No entry awaiting delivery")?;
            if let Some(snapshot) = input.snapshot {
                let quote: Snapshot =
                    serde_json::from_str(snapshot.get()).map_err(|_| "Invalid final quote")?;
                let p = &entry.plan;
                if quote.symbol != p.symbol
                    || quote.timeframe != "5m"
                    || observed < p.prepared_at_ms
                    || observed > p.expires_at_ms
                    || !quote.current_price.is_finite()
                    || quote.current_price <= 0.0
                    || (p.side == "long" && quote.current_price <= p.price)
                    || (p.side == "short" && quote.current_price >= p.price)
                {
                    return Err("Entry expired or price passed its line; no order sent".into());
                }
                let span = duration("5m")?;
                let current_open = observed - observed % span;
                let prepared_open = p.prepared_at_ms - p.prepared_at_ms % span;
                let start = quote.history_start_ms.ok_or("Missing history start")?;
                let end = quote.history_end_ms.ok_or("Missing history end")?;
                let current = quote
                    .current_candle
                    .as_ref()
                    .ok_or("Missing live candle; no order sent")?;
                if quote.candles.is_empty()
                    || quote.candles.len() > 48
                    || start > prepared_open
                    || end != current_open - span
                    || start > end
                    || quote.candles.last().unwrap().0 > end
                    || quote.candles.iter().any(|bar| bar.0 % span != 0)
                    || quote
                        .candles
                        .windows(2)
                        .any(|bars| bars[1].0 - bars[0].0 != span)
                    || current.0 != current_open
                    || current.4 != quote.current_price
                {
                    return Err(
                        "Market history does not cover the waiting period; no order sent".into(),
                    );
                }
                let first = quote.candles[0].0;
                if first != start && entry.validation != Some((observed, first - span)) {
                    return Err("Market validation skipped history or changed observation".into());
                }
                for bar in quote.candles.iter().chain(quote.current_candle.iter()) {
                    bar.validate()?;
                    if bar.0 > observed
                        || (bar.0 >= p.first_known
                            && ((p.side == "long" && bar.3 <= p.line_price)
                                || (p.side == "short" && bar.2 >= p.line_price)))
                    {
                        return Err("Entry line was already touched; no order sent".into());
                    }
                }
                let through = quote.candles.last().unwrap().0;
                if quote.batch_complete == Some(true) {
                    if through != end {
                        return Err("Final validation did not reach current market evidence".into());
                    }
                    entry.validation = None;
                } else {
                    entry.validation = Some((observed, through));
                }
            } else {
                requests.push(json!({"kind":"market.candles.read", "provider":"bingx",
                    "symbol":entry.plan.symbol, "timeframe":"5m", "limit":600}));
            }
        }
        "restore_entry" => {
            let history: Value =
                serde_json::from_str(input.snapshot.ok_or("Missing durable history")?.get())
                    .map_err(|_| "Invalid durable history")?;
            let account = &state
                .account
                .as_ref()
                .ok_or("Connect the account before recovery")?
                .account_id;
            if history["account_id"].as_str() != Some(account.as_str())
                || history["batch_complete"].as_bool().is_none()
            {
                return Err("Durable history belongs to another account or is incomplete".into());
            }
            let operations = history["operations"]
                .as_array()
                .filter(|ops| ops.len() <= 4)
                .ok_or("Invalid durable history batch")?;
            for operation in operations {
                if operation["account_binding_id"].as_str() != Some(account.as_str())
                    || operation["provider_id"] != "bingx"
                {
                    return Err("Durable operation belongs to another account or provider".into());
                }
                if operation["effect_kind"] == "position.exit.place" {
                    if operation["state"] == "terminal_failure"
                        && matches!(
                            operation["last_error_code"].as_str(),
                            Some("exit_not_sent" | "provider_rejected")
                        )
                    {
                        continue;
                    }
                    let plan: ExitPlan = serde_json::from_str(
                        operation["canonical_payload_json"]
                            .as_str()
                            .ok_or("Missing durable exit payload")?,
                    )
                    .map_err(|_| "Invalid durable exit")?;
                    if let Some(entry) = state.entry.as_mut().filter(|e| e.plan == plan.entry_plan)
                    {
                        entry.observed_position_id = Some(plan.position_id.clone());
                        entry.profit_exit = Some(ManagedExit {
                            plan,
                            attempted: true,
                            evidence: None,
                        });
                    }
                    continue;
                }
                if operation["effect_kind"] != "order.entry.place"
                    || (operation["state"] == "terminal_failure"
                        && matches!(
                            operation["last_error_code"].as_str(),
                            Some("entry_not_sent" | "provider_rejected")
                        )
                        && operation["receipt"].is_null())
                {
                    continue;
                }
                let plan: EntryPlan = serde_json::from_str(
                    operation["canonical_payload_json"]
                        .as_str()
                        .ok_or("Missing durable entry payload")?,
                )
                .map_err(|_| "Invalid durable entry")?;
                if plan.account_id != *account {
                    return Err("Durable entry belongs to another account".into());
                }
                if plan.prepared_at_ms <= state.retired_before_ms {
                    continue;
                }
                if state.settings.symbol != plan.symbol {
                    continue;
                }
                state.entry = Some(ManagedEntry {
                    plan,
                    attempted: true,
                    evidence: None,
                    validation: None,
                    observed_position_id: None,
                    lifecycle_observed_at_ms: None,
                    flat_observed_at_ms: None,
                    position: None,
                    profit_exit: None,
                });
            }
            state.validate()?;
        }
        "order" => {
            let evidence: OrderEvidence =
                serde_json::from_str(input.snapshot.ok_or("Missing order evidence")?.get())
                    .map_err(|_| "Invalid order evidence")?;
            let entry = state
                .entry
                .as_mut()
                .filter(|e| e.attempted)
                .ok_or("No managed order")?;
            entry.observe_order(evidence, observed)?;
        }
        "lifecycle" => {
            let snapshot: LifecycleSnapshot =
                serde_json::from_str(input.snapshot.ok_or("Missing lifecycle evidence")?.get())
                    .map_err(|_| "Invalid lifecycle evidence")?;
            let entry = state
                .entry
                .as_mut()
                .filter(|e| e.attempted)
                .ok_or("No attempted entry")?;
            if snapshot.account_id != entry.plan.account_id
                || snapshot.symbol != entry.plan.symbol
                || snapshot.observed_at_ms > observed
                || observed - snapshot.observed_at_ms > 60_000
                || snapshot.entry.observed_at_ms > snapshot.observed_at_ms
                || snapshot.observed_at_ms - snapshot.entry.observed_at_ms > 60_000
                || entry
                    .lifecycle_observed_at_ms
                    .is_some_and(|t| t > snapshot.observed_at_ms)
                || snapshot.positions.len() > 64
                || snapshot.orders.len() > 64
            {
                return Err("Lifecycle evidence is stale or belongs to another scope".into());
            }
            entry.observe_order(snapshot.entry, observed)?;
            if entry.profit_exit.as_ref().is_some_and(|e| !e.attempted) {
                entry.profit_exit = None;
            }
            match (entry.profit_exit.as_mut(), snapshot.exit) {
                (Some(exit), Some(evidence)) => exit.observe(evidence, observed)?,
                (None, None) => {}
                _ => return Err("Lifecycle omitted or added managed exit evidence".into()),
            }
            let mut position_ids = std::collections::BTreeSet::new();
            for position in &snapshot.positions {
                position.validate()?;
                if !position_ids.insert(&position.position_id) {
                    return Err("Duplicate current position evidence".into());
                }
            }
            let evidence = entry.evidence.as_ref().unwrap();
            if let [position] = snapshot.positions.as_slice() {
                if evidence.filled_quantity > 0.0
                    && position.side == entry.plan.side
                    && position.quantity <= evidence.filled_quantity
                    && same_price(position.average_price, evidence.average_price)
                    && entry
                        .observed_position_id
                        .as_ref()
                        .is_none_or(|id| id == &position.position_id)
                {
                    // Observation is not authority to manage a foreign position.
                    // A flat read alone is insufficient if a fill was never seen open.
                    entry.observed_position_id = Some(position.position_id.clone());
                    entry.position = Some(position.clone());
                }
            }
            let terminal = matches!(
                evidence.status.as_str(),
                "filled" | "cancelled" | "rejected" | "expired"
            );
            let flat = terminal && snapshot.positions.is_empty() && snapshot.orders.is_empty();
            // If a stop or manual close happened between polls, one flat read
            // alone can race the fill. Require a second fresh flat snapshot
            // before rearming. A position observed by this lifecycle has
            // already established the fill-to-position link, so one flat
            // snapshot remains sufficient for its closure.
            let closed = flat
                && (evidence.filled_quantity == 0.0
                    || entry.observed_position_id.is_some()
                    || entry
                        .flat_observed_at_ms
                        .is_some_and(|previous| previous < snapshot.observed_at_ms));
            let exit_terminal = entry.profit_exit.as_ref().is_none_or(|exit| {
                !exit.attempted
                    || exit.evidence.as_ref().is_some_and(|e| {
                        ["filled", "cancelled", "expired", "rejected"].contains(&e.status.as_str())
                    })
            });
            entry.lifecycle_observed_at_ms = Some(snapshot.observed_at_ms);
            entry.flat_observed_at_ms = if flat && !closed {
                Some(snapshot.observed_at_ms)
            } else {
                None
            };
            if closed && exit_terminal {
                let plan = entry.plan.clone();
                if evidence.filled_quantity > 0.0 {
                    for frame in &mut state.frames {
                        if frame.timeframe == plan.timeframe {
                            let levels = if plan.side == "long" {
                                &mut frame.sell
                            } else {
                                &mut frame.buy
                            };
                            for level in levels {
                                if level.origin == plan.origin
                                    && level.first_known == plan.first_known
                                {
                                    level.consumed = true;
                                }
                            }
                        }
                    }
                }
                state.retired_before_ms = state.retired_before_ms.max(plan.prepared_at_ms);
                state.entry = None;
                state.complete = false;
            }
        }
        "account" => {
            let account: AccountSnapshot =
                serde_json::from_str(input.snapshot.ok_or("Missing account snapshot")?.get())
                    .map_err(|_| "Invalid account snapshot")?;
            account.validate()?;
            if account.symbol != state.settings.symbol
                || account.observed_at_ms > observed
                || observed.saturating_sub(account.observed_at_ms) > 60_000
            {
                return Err(
                    "Account evidence does not match the selected instrument or current time"
                        .into(),
                );
            }
            if state
                .account
                .as_ref()
                .is_some_and(|old| old.account_id != account.account_id)
            {
                state.retired_before_ms = 0;
            }
            state.account = Some(account);
            state.request_recovery(&mut requests);
        }
        "market" => {
            let snapshot: Snapshot =
                serde_json::from_str(input.snapshot.ok_or("Missing market snapshot")?.get())
                    .map_err(|_| "Invalid market snapshot")?;
            if snapshot.symbol != state.settings.symbol {
                return Err("Market snapshot belongs to another instrument".into());
            }
            let tf = snapshot.timeframe.as_str();
            let span = duration(tf)?;
            let candles = snapshot.candles;
            if candles.is_empty() || candles.len() > 600 {
                return Err("Expected 1..600 closed candles".into());
            }
            for bar in &candles {
                bar.validate()?;
                if bar.0 % span != 0 || bar.0 + span > observed {
                    return Err("Unclosed or misaligned candle".into());
                }
            }
            if candles.windows(2).any(|b| b[1].0 - b[0].0 != span) {
                return Err("Candle history is not contiguous".into());
            }
            let price = snapshot.current_price;
            if !price.is_finite() || price <= 0.0 {
                return Err("Invalid current price".into());
            }
            if let Some(current) = &snapshot.current_candle {
                current.validate()?;
                if current.0 % span != 0
                    || current.0 > observed
                    || current.0 + span <= observed
                    || current.4 != price
                {
                    return Err("Invalid current-candle quote".into());
                }
            }
            let index = match state.frames.iter().position(|f| f.timeframe == tf) {
                Some(index) => index,
                None => {
                    state.frames.push(Frame {
                        timeframe: tf.into(),
                        ..Frame::default()
                    });
                    state.frames.len() - 1
                }
            };
            let frame = &mut state.frames[index];
            let history_end = snapshot.history_end_ms.unwrap_or(candles.last().unwrap().0);
            if history_end < candles.last().unwrap().0 || history_end + span > observed {
                return Err("Invalid history end".into());
            }
            if frame
                .last_open
                .is_some_and(|last| candles[0].0 > last + span)
            {
                // After a long offline interval the provider's bounded window
                // no longer overlaps. Rebuild from its complete contiguous
                // history instead of inventing the missing candles.
                *frame = Frame {
                    timeframe: tf.into(),
                    ..Frame::default()
                };
            }
            if frame.formation_start.is_none() {
                frame.formation_start = Some(history_end.saturating_sub(500 * span));
            }
            if let Some(last) = frame.last_open {
                if history_end < last {
                    return Err("Stale market snapshot".into());
                }
                if let Some(next) = candles.iter().find(|c| c.0 > last) {
                    if next.0 != last + span {
                        return Err("Missing candles since previous inspection".into());
                    }
                }
                for saved in &frame.bars {
                    if candles.iter().any(|c| c.0 == saved.0 && c != saved) {
                        return Err("Previously closed candle changed".into());
                    }
                }
            }
            // Only a complete window with a live candle proves the line is
            // current. Partial streaming or failed reads cannot inherit this.
            frame.checked_at_ms = None;
            for bar in &candles {
                if frame.last_open.is_some_and(|last| bar.0 <= last) {
                    continue;
                }
                // JackV Present draws levels over the last 501 bars while
                // swings and ATR warm up from the entire supplied history.
                frame.process(
                    bar.clone(),
                    &state.settings,
                    span,
                    bar.0 >= frame.formation_start.unwrap(),
                );
            }
            if snapshot.batch_complete.unwrap_or(true)
                && candles.last().unwrap().0 == history_end
                && history_end == observed - observed % span - span
                && snapshot.current_candle.is_some()
                && state.observed_at.is_none_or(|last| observed >= last)
            {
                frame.checked_at_ms = Some(observed);
            }
            // Older captured snapshots may replay history, but must not
            // replace a newer quote or invalidate levels formed since it.
            if state.observed_at.is_none_or(|last| observed >= last) {
                state.current_price = Some(price);
                state.observed_at = Some(observed);
                for frame in &mut state.frames {
                    for level in &mut frame.buy {
                        level.touched |= price >= level.price;
                        if let Some(current) = &snapshot.current_candle {
                            level.touched |=
                                current.0 >= level.first_known && current.2 >= level.price;
                        }
                    }
                    for level in &mut frame.sell {
                        level.touched |= price <= level.price;
                        if let Some(current) = &snapshot.current_candle {
                            level.touched |=
                                current.0 >= level.first_known && current.3 <= level.price;
                        }
                    }
                }
            }
            state.complete =
                tf == "5m" && state.frames.len() == 6 && snapshot.batch_complete.unwrap_or(true);
        }
        _ => return Err("Unsupported action".into()),
    }
    let autonomous = input
        .execution
        .as_ref()
        .is_some_and(|e| e.allow_new_entries);
    let mut presentation = view_for_execution(
        &state,
        observed,
        autonomous,
        !matches!(
            input.action.as_str(),
            "market" | "lifecycle" | "exit_ready" | "place_exit" | "validate_exit" | "exit"
        ),
    );
    if let Some(account) = &state.account {
        if input.execution.is_some() && !autonomous {
            let message = presentation["message"].as_str().unwrap().to_owned();
            presentation["message"] = json!(format!("{message}\nNew entries are stopped; reducing exits may continue under the unexpired host grant."));
        }
        presentation["schedule"] = json!({"action":"cycle", "interval_seconds":60,
            "account_id":account.account_id, "symbol":state.settings.symbol,
            "max_margin":state.settings.entry_margin, "max_stop_percent":state.settings.stop_percent});
        let actions = presentation["actions"].as_array_mut().unwrap();
        if actions.len() >= 4 {
            if let Some(index) = actions
                .iter()
                .position(|a| a["id"] == "connect" || a["id"] == "inspect")
            {
                actions.remove(index);
            }
        }
        if autonomous {
            actions.push(json!({"id":"stop", "label":"Stop new entries", "host":"workspace.stop"}));
        } else {
            actions.push(
                json!({"id":"start", "label":"Start local cycles", "host":"workspace.start"}),
            );
        }
    }
    Ok(Output {
        view: WorkspaceView::Trading(presentation),
        state: if matches!(input.action.as_str(), "open" | "present" | "validate_exit")
            && input.state.is_some()
        {
            // Read-only presentation preserves opaque saved bytes instead of
            // spending another budget serializing every retained swing.
            SavedState::Unchanged(input.state.unwrap())
        } else {
            SavedState::Updated(state)
        },
        requests,
        resume_action,
    })
}

fn prepare_entry(state: &mut State, observed: i64) -> Result<(), String> {
    if observed <= state.retired_before_ms {
        return Err("Use a fresh observation after the retired entry".into());
    }
    if state.entry.as_ref().is_some_and(|e| e.attempted) {
        return Err("An entry is already managed; refresh its exact order instead".into());
    }
    if !state.complete
        || state
            .observed_at
            .is_none_or(|t| t > observed || observed - t > 60_000)
    {
        return Err("Refresh all six timeframes before preparing an entry".into());
    }
    let (tf, side, level) = candidate(state).ok_or("No untouched entry line ahead of price")?;
    let (price, quantity, stop_price, margin, leverage) =
        sizing(state, side, level.price, observed)?;
    state.entry = Some(ManagedEntry {
        plan: EntryPlan {
            account_id: state.account.as_ref().unwrap().account_id.clone(),
            symbol: state.settings.symbol.clone(),
            side: side.into(),
            timeframe: tf.into(),
            origin: level.origin,
            first_known: level.first_known,
            line_price: level.price,
            price,
            quantity,
            stop_price,
            margin,
            leverage: leverage as u32,
            prepared_at_ms: observed,
            expires_at_ms: observed + ENTRY_HISTORY_WINDOW,
        },
        attempted: false,
        evidence: None,
        validation: None,
        observed_position_id: None,
        lifecycle_observed_at_ms: None,
        flat_observed_at_ms: None,
        position: None,
        profit_exit: None,
    });
    Ok(())
}

fn observe_orders(input: Input<'_>) -> Result<Output<'_>, String> {
    // Observation preserves strategy bytes without parsing candle floats or
    // running its reducer. The host validates the full workspace before reads.
    #[derive(Deserialize)]
    struct Scope {
        version: u32,
        settings: Settings,
        account: Option<AccountSnapshot>,
    }
    let state = input.state.ok_or("Connect your account first")?;
    let scope: Scope = serde_json::from_str(state.get()).map_err(|e| e.to_string())?;
    scope.settings.validate()?;
    let account = scope.account.ok_or("Connect your account first")?;
    account.validate()?;
    if scope.version != 1 || account.symbol != scope.settings.symbol {
        return Err("Invalid order observation scope".into());
    }
    if input
        .settings
        .as_ref()
        .is_some_and(|s| s != &scope.settings)
    {
        return Err("Refresh zones before reading orders for changed settings".into());
    }
    let mut requests = Vec::new();
    let mut rows = Vec::new();
    let mut observed_at = None;
    if input.action == "list_orders" {
        requests.push(json!({"kind":"order.snapshot.read", "scope":"open",
            "provider":"bingx", "account_id":account.account_id, "symbol":scope.settings.symbol}));
    } else {
        let snapshot: OpenOrdersSnapshot =
            serde_json::from_str(input.snapshot.ok_or("Missing open-order snapshot")?.get())
                .map_err(|_| "Invalid open-order snapshot")?;
        if snapshot.account_id != account.account_id
            || snapshot.symbol != scope.settings.symbol
            || snapshot.observed_at_ms <= 0
            || snapshot.observed_at_ms > input.observed_at_ms
            || input.observed_at_ms - snapshot.observed_at_ms > 60_000
            || snapshot.orders.len() > 64
        {
            return Err("Open orders belong to another account, instrument or observation".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        for order in &snapshot.orders {
            let token = |s: &str| {
                !s.is_empty()
                    && s.len() <= 32
                    && s.bytes().all(|c| c.is_ascii_uppercase() || c == b'_')
            };
            let mut numbers = [0.0; 4];
            for (index, n) in [
                &order.price,
                &order.stop_price,
                &order.quantity,
                &order.filled_quantity,
            ]
            .iter()
            .enumerate()
            {
                numbers[index] = n
                    .parse::<f64>()
                    .map_err(|_| "Invalid open-order evidence")?;
                if n.len() > 64 || !numbers[index].is_finite() || numbers[index] < 0.0 {
                    return Err("Invalid open-order evidence".into());
                }
            }
            if order.order_id.is_empty()
                || order.order_id.len() > 30
                || order.order_id.starts_with('0')
                || !order.order_id.bytes().all(|c| c.is_ascii_digit())
                || !ids.insert(&order.order_id)
                || !["BUY", "SELL"].contains(&order.side.as_str())
                || !["BOTH", "LONG", "SHORT"].contains(&order.position_side.as_str())
                || !token(&order.r#type)
                || !token(&order.status)
                || numbers[3] > numbers[2]
            {
                return Err("Invalid open-order evidence".into());
            }
            rows.push([
                order.order_id.clone(),
                format!("{} / {}", order.side, order.position_side),
                order.r#type.clone(),
                order.status.clone(),
                format!("{} / {}", order.price, order.stop_price),
                format!("{} / {}", order.filled_quantity, order.quantity),
            ]);
        }
        observed_at = Some(snapshot.observed_at_ms);
    }
    let view = OpenOrdersView {
        title: "Jack Ventura", fields: [], confirmation: None,
        summary: format!("{} / {}", account.account_label, scope.settings.symbol),
        message: match observed_at {
            Some(_) => format!("{} open orders returned for {}. Expand Open orders for exact IDs and provider status.", rows.len(), scope.settings.symbol),
            None => "Reading current open orders...".into()
        },
        actions: [json!({"id":"list_orders", "label":"Refresh open orders"}), json!({"id":"open", "label":"Back to trading"})],
        details_title: "Open orders",
        columns: ["Order ID", "Side / position", "Type", "Status", "Price / trigger", "Filled / quantity"],
        rows,
        details: format!("Observation: {}. Orders are not adopted or cancelled. An empty list does not prove there is no position. No protection or PnL is inferred.",
            observed_at.map(|t| age(input.observed_at_ms, t)).unwrap_or_else(|| "not yet".into()))};
    Ok(Output {
        state: SavedState::Unchanged(state),
        view: WorkspaceView::Orders(view),
        requests,
        resume_action: None,
    })
}

fn age(now: i64, then: i64) -> String {
    let minutes = now.saturating_sub(then).max(0) / 60_000;
    if minutes >= 1440 {
        format!("{}d {}h ago", minutes / 1440, minutes % 1440 / 60)
    } else if minutes >= 60 {
        format!("{}h {}m ago", minutes / 60, minutes % 60)
    } else {
        format!("{minutes}m ago")
    }
}

// Preview only: round quantity down to the user's margin, and round the
// protective price toward entry so a price tick cannot enlarge that budget.
fn sizing(
    state: &State,
    side: &str,
    line: f64,
    now: i64,
) -> Result<(f64, f64, f64, f64, f64), String> {
    let account = state
        .account
        .as_ref()
        .ok_or("Connect your BingX account to preview quantity and stop")?;
    if account.symbol != state.settings.symbol
        || account.observed_at_ms > now
        || now.saturating_sub(account.observed_at_ms) > 60_000
    {
        return Err("Refresh account data for this instrument to preview quantity and stop".into());
    }
    if state.settings.entry_margin > account.available_margin {
        return Err("The chosen margin exceeds available futures margin".into());
    }
    let leverage = if side == "long" {
        account.long_leverage
    } else {
        account.short_leverage
    } as f64;
    let price_scale = 10f64.powi(account.price_precision as i32);
    let quantity_scale = 10f64.powi(account.quantity_precision as i32);
    let entry = if side == "long" {
        (line * price_scale).floor()
    } else {
        (line * price_scale).ceil()
    } / price_scale;
    let notional = state.settings.entry_margin * leverage;
    if !entry.is_finite() || entry <= 0.0 || !notional.is_finite() {
        return Err("Chosen margin or price cannot be represented by this contract".into());
    }
    let quantity = (notional / entry * quantity_scale).floor() / quantity_scale;
    let actual_notional = quantity * entry;
    let actual_margin = actual_notional / leverage;
    if !quantity.is_finite()
        || quantity <= 0.0
        || quantity < account.min_quantity
        || actual_notional < account.min_notional
        || actual_margin > state.settings.entry_margin
    {
        return Err(
            "Chosen margin is below the exchange minimum after rounding; it was not increased"
                .into(),
        );
    }
    let loss_budget = actual_margin * state.settings.stop_percent / 100.0;
    let delta = loss_budget / quantity;
    let stop = if side == "long" {
        ((entry - delta) * price_scale).ceil()
    } else {
        ((entry + delta) * price_scale).floor()
    } / price_scale;
    if !stop.is_finite()
        || stop <= 0.0
        || (side == "long" && stop >= entry)
        || (side == "short" && stop <= entry)
    {
        return Err(
            "The chosen stop is smaller than a price tick or outside the contract price range"
                .into(),
        );
    }
    Ok((entry, quantity, stop, actual_margin, leverage))
}

fn preview(state: &State, side: &str, line: f64, now: i64) -> Result<String, String> {
    let (entry, quantity, stop, actual_margin, leverage) = sizing(state, side, line, now)?;
    Ok(format!("Preview: {side} {} quantity at {}; {}x exchange leverage, estimated margin {:.6} USDT, notional {:.6} USDT, stop {} ({}% of margin). No order or protection has been placed; fees, slippage and liquidation are not guaranteed by this estimate.",
        quantity, entry, leverage, actual_margin, quantity * entry, stop, state.settings.stop_percent))
}

fn same_price(a: f64, b: f64) -> bool {
    (a - b).abs() <= a.abs().max(b.abs()) * 1e-9
}

fn available_lines(state: &State) -> impl Iterator<Item = (&str, &str, &Level)> {
    TIMEFRAMES
        .iter()
        .flat_map(|(tf, _)| {
            state
                .frames
                .iter()
                .filter(move |f| f.timeframe == *tf)
                .flat_map(move |f| {
                    f.buy
                        .iter()
                        .map(move |l| (*tf, "short", l))
                        .chain(f.sell.iter().map(move |l| (*tf, "long", l)))
                })
        })
        .filter(|(_, _, l)| !l.touched && !l.consumed)
}

fn candidate(state: &State) -> Option<(&str, &str, &Level)> {
    let price = state.current_price?;
    available_lines(state)
        .filter(|(_, side, l)| {
            (*side == "short" && l.price > price) || (*side == "long" && l.price < price)
        })
        .min_by(|(atf, _, a), (btf, _, b)| {
            (a.price - price)
                .abs()
                .total_cmp(&(b.price - price).abs())
                .then_with(|| duration(btf).unwrap().cmp(&duration(atf).unwrap()))
                .then(a.first_known.cmp(&b.first_known))
                .then(a.origin.cmp(&b.origin))
        })
}

fn position_stop(state: &State, position: &PositionObservation) -> Option<(f64, f64, f64)> {
    let plan = &state.entry.as_ref()?.plan;
    let account = state.account.as_ref()?;
    if account.symbol != plan.symbol {
        return None;
    }
    // Preserve the entry's rounded risk ratio, not a later settings value or
    // its pre-fill quote. Remaining exposure scales the nominal loss budget.
    let delta = position.average_price * (plan.price - plan.stop_price).abs() / plan.price;
    let scale = 10f64.powi(account.price_precision as i32);
    let stop = if plan.side == "long" {
        ((position.average_price - delta) * scale).ceil() / scale
    } else {
        ((position.average_price + delta) * scale).floor() / scale
    };
    let margin = position.quantity * position.average_price / plan.leverage as f64;
    let loss = (position.average_price - stop).abs() * position.quantity;
    if [stop, margin, loss]
        .iter()
        .any(|v| !v.is_finite() || *v <= 0.0)
        || (plan.side == "long" && stop >= position.average_price)
        || (plan.side == "short" && stop <= position.average_price)
    {
        return None;
    }
    Some((stop, margin, loss))
}

fn profit_candidate<'a>(
    state: &'a State,
    position: &PositionObservation,
    now: i64,
) -> Option<(&'a str, &'a Level)> {
    let price = state.current_price?;
    if !state.complete
        || state
            .observed_at
            .is_none_or(|t| t > now || now - t > 60_000)
        || state.frames.len() != TIMEFRAMES.len()
        || state
            .frames
            .iter()
            .any(|f| f.checked_at_ms.is_none_or(|t| t > now || now - t > 60_000))
    {
        return None;
    }
    available_lines(state)
        .filter(|(_, side, level)| {
            level.first_known <= now
                && if position.side == "long" {
                    *side == "short" && level.price > price && level.price > position.average_price
                } else {
                    *side == "long" && level.price < price && level.price < position.average_price
                }
        })
        .min_by(|(atf, _, a), (btf, _, b)| {
            (a.price - price)
                .abs()
                .total_cmp(&(b.price - price).abs())
                .then_with(|| duration(btf).unwrap().cmp(&duration(atf).unwrap()))
                .then(a.first_known.cmp(&b.first_known))
                .then(a.origin.cmp(&b.origin))
        })
        .map(|(tf, _, level)| (tf, level))
}

fn profit_exit_plan(state: &State, now: i64) -> Option<ExitPlan> {
    let entry = state.entry.as_ref()?;
    if entry.profit_exit.is_some()
        || entry
            .lifecycle_observed_at_ms
            .is_none_or(|t| t > now || now - t > 60_000)
    {
        return None;
    }
    let position = entry.position.as_ref()?;
    let (_, line) = profit_candidate(state, position, now)?;
    let account = state.account.as_ref()?;
    let scale = 10f64.powi(account.price_precision as i32);
    let price = if position.side == "long" {
        (line.price * scale).floor() / scale
    } else {
        (line.price * scale).ceil() / scale
    };
    if price <= 0.0
        || (position.side == "long"
            && (price <= position.average_price || price <= state.current_price?))
        || (position.side == "short"
            && (price >= position.average_price || price >= state.current_price?))
    {
        return None;
    }
    Some(ExitPlan {
        entry_plan: entry.plan.clone(),
        position_id: position.position_id.clone(),
        quantity: position.quantity,
        average_price: position.average_price,
        price,
        prepared_at_ms: now,
        expires_at_ms: now.checked_add(60_000)?,
    })
}

fn position_status(state: &State, entry: &ManagedEntry, now: i64) -> String {
    let Some(position) = &entry.position else {
        return if entry
            .evidence
            .as_ref()
            .is_some_and(|e| e.filled_quantity > 0.0)
        {
            "No matching current position observation. Refresh its lifecycle; a fill receipt alone cannot size an exit or authorize reentry.".into()
        } else {
            String::new()
        };
    };
    let observed = entry.lifecycle_observed_at_ms.unwrap();
    if observed > now || now - observed > 60_000 {
        return format!("Saved position {} is stale. Refresh its lifecycle before using a stop or profit candidate; no exit is authorized.", position.position_id);
    }
    let mut message = format!(
        "Observed position {}: {} quantity at average {} ({}).",
        position.position_id,
        position.quantity,
        position.average_price,
        age(now, observed)
    );
    match position_stop(state, position) {
        Some((stop, margin, loss)) => message.push_str(&format!(
            "\nFill-based stop estimate: {stop}; remaining margin {margin:.6} USDT at {}x entry leverage, nominal loss {loss:.6} USDT before fees/slippage. This is not confirmation of exchange-side protection.", entry.plan.leverage)),
        None => message.push_str("\nA fill-based stop cannot be represented by this contract; exchange-side protection is unverified."),
    }
    if let Some(exit) = &entry.profit_exit {
        message.push_str(&format!(
            "\n{} profit exit: {} for {} quantity. {}",
            if exit.attempted { "Fixed" } else { "Prepared" },
            exit.plan.price,
            exit.plan.quantity,
            exit.evidence
                .as_ref()
                .map(|e| format!(
                    "Order {} / {} / filled {}. Position closure still requires a lifecycle read.",
                    e.order_id.as_deref().unwrap_or("unverified"),
                    e.status,
                    e.filled_quantity
                ))
                .unwrap_or_else(|| if exit.attempted {
                    "Provider outcome unverified; refresh the exact exit, do not resubmit.".into()
                } else {
                    "Not sent. Explicit confirmation or authorized cycle is required.".into()
                })
        ));
        return message;
    }
    match profit_candidate(state, position, now) {
        Some((tf, level)) => message.push_str(&format!(
            "\nOpposite-line exit candidate: {} on {} for the remaining {} quantity. Observation only; no profit exit is placed or fixed yet.", level.price, tf.to_uppercase(), position.quantity)),
        None => message.push_str("\nNo current eligible opposite-line exit candidate. A target is not invented; profit exit and position protection remain unverified."),
    }
    message
}

#[derive(PartialEq)]
enum PendingLine {
    Unknown,
    Filled,
    Terminal,
    Stale,
    Incomplete,
    Missing,
    Moved,
    Touched,
    Valid,
}

fn pending_line(state: &State, entry: &ManagedEntry, now: i64) -> PendingLine {
    let Some(evidence) = &entry.evidence else {
        return PendingLine::Unknown;
    };
    if evidence.filled_quantity > 0.0 {
        return PendingLine::Filled;
    }
    if matches!(
        evidence.status.as_str(),
        "cancelled" | "rejected" | "expired" | "not_sent"
    ) {
        return PendingLine::Terminal;
    }
    if evidence.status != "open"
        || now < evidence.observed_at_ms
        || now - evidence.observed_at_ms > 60_000
    {
        return PendingLine::Stale;
    }
    if !state.complete
        || !TIMEFRAMES.iter().all(|(tf, _)| {
            state.frames.iter().any(|f| {
                f.timeframe == *tf
                    && f.checked_at_ms
                        .is_some_and(|t| t <= now && now - t <= 60_000)
            })
        })
    {
        return PendingLine::Incomplete;
    }
    let plan = &entry.plan;
    let frame = state
        .frames
        .iter()
        .find(|f| f.timeframe == plan.timeframe)
        .unwrap();
    let levels = if plan.side == "long" {
        &frame.sell
    } else {
        &frame.buy
    };
    let Some(line) = levels
        .iter()
        .find(|l| l.origin == plan.origin && l.first_known == plan.first_known)
    else {
        return PendingLine::Missing;
    };
    if line.price != plan.line_price {
        return PendingLine::Moved;
    }
    if line.touched {
        return PendingLine::Touched;
    }
    PendingLine::Valid
}

fn pending_cancellation(state: &State, now: i64) -> Option<Value> {
    let entry = state.entry.as_ref().filter(|e| e.attempted)?;
    if !matches!(
        pending_line(state, entry, now),
        PendingLine::Missing | PendingLine::Moved | PendingLine::Touched
    ) {
        return None;
    }
    let order_id = entry.evidence.as_ref()?.order_id.as_ref()?;
    Some(
        json!({"kind":"order.entry.cancel", "provider":"bingx", "plan":entry.plan, "order_id":order_id}),
    )
}

fn pending_line_status(state: &State, entry: &ManagedEntry, now: i64) -> &'static str {
    match pending_line(state, entry, now) {
        PendingLine::Unknown => "Pending line: exact order status is unknown; no replacement is authorized.",
        PendingLine::Filled => "Entry has fills. Reconcile the actual position before any replacement; line changes do not authorize a second entry.",
        PendingLine::Terminal => "No pending entry in the last order observation. Reconcile flat exposure before any re-entry.",
        PendingLine::Stale => "Pending line: refresh the exact order before deciding what to do; no replacement is authorized.",
        PendingLine::Incomplete => "Pending line: refresh order and zones; complete current market evidence is not available.",
        PendingLine::Missing => "Original line is no longer present. Cancel this exact unfilled entry; replacement waits for confirmed cancellation and flat exposure.",
        PendingLine::Moved => "Original line moved. Cancel this exact unfilled entry; replacement waits for confirmed cancellation and flat exposure.",
        PendingLine::Touched => "Price reached the original line. Cancellation rechecks the exact order for a fill; market evidence alone cannot authorize replacement.",
        PendingLine::Valid => "Original line is unchanged and untouched. Keep the existing entry; a nearer alternative does not replace it.",
    }
}

#[cfg(test)]
fn view(state: &State, now: i64) -> Value {
    view_for_execution(state, now, false, true)
}

fn view_for_execution(state: &State, now: i64, autonomous: bool, render_lines: bool) -> Value {
    let mut lines = Vec::new();
    // Candle processing and full presentation have separate bounded invocations.
    // The existing resume action renders after all provider evidence is saved.
    if render_lines && (state.entry.is_none() || state.complete) {
        for (tf, _) in TIMEFRAMES {
            if let Some(frame) = state.frames.iter().find(|f| f.timeframe == tf) {
                for (side, levels) in [("short", &frame.buy), ("long", &frame.sell)] {
                    for level in levels {
                        lines.push((tf, side, level));
                    }
                }
            }
        }
    }
    let candidate = if state.entry.is_none() {
        candidate(state)
    } else {
        None
    };
    let mut message = if let Some(entry) = &state.entry {
        let p = &entry.plan;
        if !entry.attempted {
            format!("Prepared {} limit: {} quantity at {} on {}, margin {:.6} USDT at {}x, requested stop {}. {} No order has been sent. {}",
                p.side, p.quantity, p.price, p.timeframe.to_uppercase(), p.margin, p.leverage, p.stop_price,
                if now > p.expires_at_ms {"History window expired: prepare again."} else {"Market and leverage are rechecked before sending."},
                if autonomous {"The next authorized cycle may send this entry after fresh validation."} else {"Explicit confirmation is required for this single entry."})
        } else if let Some(e) = &entry.evidence {
            if e.status == "not_sent" {
                format!(
                    "{}: entry was not sent. No exchange order or stop was created. {}",
                    p.symbol,
                    if autonomous {
                        "The next cycle recalculates from fresh zones and account data."
                    } else {
                        "Recalculate the entry from current zones and account data; a new plan requires your confirmation."
                    }
                )
            } else if e.order_id.is_none() && e.status == "unknown" {
                format!("{}: no provider order has been confirmed for this attempt. Refresh its exact status; no automatic resubmission. Position protection and PnL are unverified.", p.symbol)
            } else if entry.position.is_some() {
                format!(
                    "{} / entry order {} / {} / filled {} of {} / observed {}.",
                    p.symbol,
                    e.order_id.as_deref().unwrap_or("unknown"),
                    e.status,
                    e.filled_quantity,
                    p.quantity,
                    age(now, e.observed_at_ms)
                )
            } else {
                format!("{}: {} limit at {} / order {} / {} / filled {} at {} / observed {}. Stop {} was requested, but position protection and PnL are not verified. {}",
                p.symbol, p.side, p.price, e.order_id.as_deref().unwrap_or("unknown"), e.status,
                e.filled_quantity, e.average_price, age(now,e.observed_at_ms), p.stop_price,
                if autonomous {"Cycles reconcile this entry and any reducing exit before reentry."} else {"New entries are not enabled. Reducing exits use explicit confirmation or an unexpired cycle grant."})
            }
        } else {
            "Entry outcome unknown. Refresh the exact order; do not submit a second entry. No position protection or PnL is verified.".into()
        }
    } else if state
        .observed_at
        .is_some_and(|t| now.saturating_sub(t) > 300_000)
    {
        "Saved observation. Refresh zones before using the entry candidate. No order has been sent."
            .into()
    } else if !state.complete {
        "Refresh the selected instrument to load all six timeframes. No order has been sent.".into()
    } else if let Some((tf, side, level)) = candidate {
        let mut message = format!(
            "Entry candidate: {side} at {} on {}. Observation only; no order has been sent.",
            level.price,
            tf.to_uppercase()
        );
        match preview(state, side, level.price, now) {
            Ok(p) => {
                message.push('\n');
                message.push_str(&p);
            }
            Err(reason) => {
                message.push('\n');
                message.push_str(&reason);
            }
        }
        message
    } else {
        "No untouched entry line is currently ahead of price. Existing lines remain visible. No order has been sent.".into()
    };
    if let Some(reason) = state
        .entry
        .as_ref()
        .and_then(|e| e.evidence.as_ref())
        .and_then(|e| e.error_message.as_ref())
    {
        message.push('\n');
        message.push_str(reason);
    }
    if !state.parked.is_empty() {
        let symbols = state
            .parked
            .iter()
            .map(|p| p.settings.symbol.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        message.push_str(&format!("\nSaved managed instruments: {symbols}. Their exchange orders remain live; only the selected instrument is observed by this workspace."));
    }
    if let Some(entry) = state
        .entry
        .as_ref()
        .filter(|e| e.attempted && !e.not_sent())
    {
        if entry.position.is_none() {
            message.push('\n');
            message.push_str(pending_line_status(state, entry, now));
        }
        let position = position_status(state, entry, now);
        if !position.is_empty() {
            message.push('\n');
            message.push_str(&position);
        }
    } else if state.entry.is_none() && state.retired_before_ms > 0 {
        message.push_str(if autonomous {
            "\nPrevious entry retired after a terminal order and complete flat evidence. The next cycle refreshes market and account data for a distinct entry."
        } else {
            "\nPrevious entry retired after a terminal order and complete flat evidence. Refresh market and account data for a distinct next entry; no automatic submission is enabled."
        });
    }
    let rows: Vec<_> = lines
        .iter()
        .map(|(tf, side, level)| {
            json!([
                tf.to_uppercase(),
                if *side == "short" {
                    "Buyside / short"
                } else {
                    "Sellside / long"
                },
                level.price.to_string(),
                if level.consumed {
                    "Consumed by previous entry"
                } else if level.touched {
                    "Touched"
                } else {
                    "Untouched"
                },
                age(now, level.first_known)
            ])
        })
        .collect();
    if autonomous {
        message.push_str("\nLocal cycles are enabled under host authority. New entries use fresh market/account evidence; no eligible line means waiting. Invalidated unfilled entries may be cancelled before replacement. Reducing exits use matching position evidence and a profitable opposite line. Stop disables entry and cancellation, not reducing exits. Exchange-side stop protection remains unverified.");
    }
    let summary = state
        .account
        .as_ref()
        .map(|a| {
            format!(
                "{} / account data read {}. {}",
                a.account_label,
                age(now, a.observed_at_ms),
                if autonomous {
                    "Local cycles: app must remain open and online."
                } else {
                    "Local single-entry workspace; new cycles are not enabled."
                }
            )
        })
        .unwrap_or_else(|| {
            "BingX LIVE / account not connected. Observation works without keys.".into()
        });
    let mut actions = vec![
        json!({"id":"connect", "label":if state.account.is_some() {"Reconnect account"} else {"Connect account"}, "host":"bingx.account.connect"}),
        json!({"id":"inspect", "label":"Refresh zones / preview"}),
    ];
    let mut confirmation = Value::Null;
    if let Some(entry) = &state.entry {
        if entry.attempted {
            actions = if entry.not_sent() {
                vec![json!({"id":"prepare_entry", "label":"Recalculate entry"})]
            } else {
                vec![json!({"id":"refresh_order", "label":"Refresh order or switch instrument"})]
            };
            if let Some(cancel) = pending_cancellation(state, now) {
                actions.push(json!({"id":"cancel_entry", "label":"Cancel invalidated entry", "host":"bingx.order.submit"}));
                confirmation = cancel;
            } else if let Some(exit) = entry
                .profit_exit
                .as_ref()
                .filter(|e| !e.attempted && now <= e.plan.expires_at_ms)
            {
                actions.push(json!({"id":"place_exit", "label":"Place opposite-line exit", "host":"bingx.order.submit"}));
                confirmation = exit.request();
            }
        } else {
            actions.push(json!({"id":"prepare_entry", "label":"Prepare one entry"}));
            if now <= entry.plan.expires_at_ms {
                actions.push(json!({"id":"place_entry", "label":"Place this entry", "host":"bingx.order.submit"}));
                confirmation = entry.request("order.entry.place");
            }
        }
    } else if state.account.is_some() {
        actions.push(json!({"id":"prepare_entry", "label":"Prepare one entry"}));
    }
    if state.account.is_some() {
        if actions.len() == 4 {
            actions.retain(|a| a["id"] != "inspect");
        }
        actions.push(json!({"id":"list_orders", "label":"Refresh open orders"}));
    }
    json!({"title":"Jack Ventura", "message":message, "summary":summary, "confirmation":confirmation,
        "fields":[
            {"id":"symbol", "label":"Instrument", "type":"choice", "value":state.settings.symbol,
                "source":{"kind":"market.instruments.read", "provider":"bingx"}},
            {"id":"entry_margin", "label":"Margin per entry (USDT, before leverage)", "type":"number", "value":state.settings.entry_margin},
            {"id":"stop_percent", "label":"Stop loss (% of entry margin)", "type":"number", "value":state.settings.stop_percent},
            {"id":"detection_length", "label":"Detection length", "type":"integer", "value":state.settings.detection_length, "advanced":true},
            {"id":"margin", "label":"Cluster margin", "type":"number", "value":state.settings.margin, "advanced":true}
        ],
        "actions":actions,
        "columns":["Timeframe", "Line / direction", "Price", "Status", "First known"], "rows":rows,
        "details":format!("{} / last price: {} / observed {}.",
            state.settings.symbol,
            state.current_price.map(|p| p.to_string()).unwrap_or_else(|| "not available".into()),
            state.observed_at.map(|t| age(now, t)).unwrap_or_else(|| "not yet".into()))})
}

pub fn evaluate_json(raw: &[u8]) -> Vec<u8> {
    let result = if raw.len() > INPUT_LIMIT {
        Err("Input exceeds the bounded ABI".into())
    } else {
        serde_json::from_slice::<Input>(raw)
            .map_err(|e| e.to_string())
            .and_then(evaluate)
    };
    let envelope = match result {
        Ok(result) => Envelope {
            schema_version: 1,
            status: "executed",
            result: Some(result),
            error_code: None,
            error_message: None,
        },
        Err(error) => Envelope {
            schema_version: 1,
            status: "rejected",
            result: None,
            error_code: Some("invalid_args"),
            error_message: Some(error),
        },
    };
    serde_json::to_vec(&envelope).unwrap_or_default()
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn hivra_alloc_v1(len: u32) -> u32 {
    if len == 0 || len as usize > INPUT_LIMIT {
        return 0;
    }
    let bytes = vec![0u8; len as usize].into_boxed_slice();
    Box::into_raw(bytes) as *mut u8 as u32
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub unsafe extern "C" fn hivra_dealloc_v1(ptr: u32, len: u32) {
    if ptr != 0 && len != 0 {
        drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
            ptr as *mut u8,
            len as usize,
        )));
    }
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub unsafe extern "C" fn hivra_evaluate_v1(ptr: u32, len: u32) -> u64 {
    if ptr == 0 || len == 0 || len as usize > INPUT_LIMIT {
        return 0;
    }
    let bytes = evaluate_json(std::slice::from_raw_parts(ptr as *const u8, len as usize));
    if bytes.len() > 128 * 1024 {
        return 0;
    }
    let output = bytes.into_boxed_slice();
    let output_len = output.len() as u32;
    let output_ptr = Box::into_raw(output) as *mut u8 as u32;
    ((output_ptr as u64) << 32) | output_len as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previous_swings_import_once_without_changing_strategy_values() {
        let previous = json!({"direction":-1,"time":1200000,"price":0.008688});
        let swing: Swing = serde_json::from_value(previous).unwrap();
        let compact = serde_json::to_value(&swing).unwrap();
        assert_eq!(compact, json!([-1, 1200000, 0.008688]));
        let restored: Swing = serde_json::from_value(compact).unwrap();
        assert_eq!(
            (restored.direction, restored.time, restored.price),
            (swing.direction, swing.time, swing.price)
        );
        for invalid in [
            json!([-1, 12]),
            json!([-1, 12, 1.0, 2.0]),
            json!({"direction":1,"time":12}),
            json!({"direction":1,"time":12,"price":1.0,"other":true}),
        ] {
            assert!(serde_json::from_value::<Swing>(invalid).is_err());
        }
    }

    #[test]
    fn lost_state_recovers_exact_exit_from_durable_history_without_effects() {
        let mut state = observed_fill("long", 0.5);
        fresh_exit_frames(&mut state);
        state.frames[1].buy.push(exit_level(105.0));
        let ready = invoke(
            "exit_ready",
            serde_json::to_value(state).unwrap(),
            Value::Null,
        );
        let saved = ready["result"]["state"].clone();
        let entry_plan = saved["entry"]["plan"].clone();
        let exit_plan = saved["entry"]["profit_exit"]["plan"].clone();
        let mut lost = saved.clone();
        lost["entry"] = Value::Null;
        let operations = json!([
            {"account_binding_id":"a".repeat(64),"provider_id":"bingx", "effect_kind":"order.entry.place",
                "state":"succeeded","canonical_payload_json":entry_plan.to_string()},
            {"account_binding_id":"a".repeat(64),"provider_id":"bingx", "effect_kind":"position.exit.place",
                "state":"unresolved","canonical_payload_json":exit_plan.to_string()},
        ]);
        let recovered = invoke(
            "restore_entry",
            lost,
            json!({"account_id":"a".repeat(64),
            "operations":operations,"batch_complete":true}),
        );
        assert_eq!(recovered["status"], "executed", "{recovered}");
        assert_eq!(
            recovered["result"]["state"]["entry"]["profit_exit"]["plan"],
            exit_plan
        );
        assert_eq!(
            recovered["result"]["state"]["entry"]["profit_exit"]["attempted"],
            true
        );
        assert!(recovered["result"]["requests"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(recovered["result"]["view"]["confirmation"].is_null());
    }

    #[test]
    fn profit_exit_freezes_one_plan_and_unknown_outcome_never_resubmits() {
        let mut state = observed_fill("long", 0.5);
        fresh_exit_frames(&mut state);
        state.frames[1].buy.push(exit_level(105.0));
        let ready = invoke(
            "exit_ready",
            serde_json::to_value(state).unwrap(),
            Value::Null,
        );
        let confirmation = &ready["result"]["view"]["confirmation"];
        assert_eq!(confirmation["kind"], "position.exit.place");
        assert_eq!(confirmation["plan"]["price"], 105.0);
        let sent = invoke_at(
            "place_exit",
            ready["result"]["state"].clone(),
            Value::Null,
            100_000_001,
        );
        assert_eq!(sent["result"]["requests"][0], *confirmation);
        let saved = sent["result"]["state"].clone();
        assert_eq!(
            invoke_at("validate_exit", saved.clone(), Value::Null, 100_000_001)["status"],
            "executed"
        );
        let reopened = invoke_at("present", saved.clone(), Value::Null, 100_000_002);
        assert_eq!(reopened["result"]["state"], saved);
        assert!(reopened["result"]["view"]["confirmation"].is_null());
        assert!(reopened["result"]["view"]["message"]
            .as_str()
            .unwrap()
            .contains("Provider outcome unverified"));
        assert_eq!(
            invoke("place_exit", saved.clone(), Value::Null)["status"],
            "rejected"
        );
        let cycle = cycle_invoke("cycle", saved, false);
        assert_eq!(
            cycle["result"]["requests"][0]["exit_plan"],
            confirmation["plan"]
        );
        assert_eq!(cycle["result"]["requests"].as_array().unwrap().len(), 7);
    }

    #[test]
    fn exit_closure_requires_its_terminal_evidence_and_flat_lifecycle() {
        let mut state = observed_fill("long", 1.0);
        fresh_exit_frames(&mut state);
        state.frames[1].buy.push(exit_level(105.0));
        let ready = invoke(
            "exit_ready",
            serde_json::to_value(state).unwrap(),
            Value::Null,
        );
        let sent = invoke("place_exit", ready["result"]["state"].clone(), Value::Null);
        let saved = sent["result"]["state"].clone();
        let qty = saved["entry"]["profit_exit"]["plan"]["quantity"]
            .as_f64()
            .unwrap();
        let mut evidence = json!({"account_id":"a".repeat(64),"symbol":"BTC-USDT", "client_order_id":"c".repeat(40),
            "order_id":"999", "status":"open", "filled_quantity":0.0,"average_price":0.0,"observed_at_ms":100_000_001});
        let observed = invoke_at("exit", saved.clone(), evidence.clone(), 100_000_001);
        assert_eq!(observed["status"], "executed", "{observed}");
        let mut snapshot = lifecycle_snapshot(&saved, json!([]), json!([]), 100_000_001);
        assert_eq!(
            invoke_at("lifecycle", saved.clone(), snapshot.clone(), 100_000_001)["status"],
            "rejected"
        );
        snapshot["exit"] = evidence.clone();
        let unresolved = invoke_at("lifecycle", saved.clone(), snapshot.clone(), 100_000_001);
        assert!(unresolved["result"]["state"]["entry"].is_object());
        evidence["status"] = json!("filled");
        evidence["filled_quantity"] = json!(qty);
        evidence["average_price"] = json!(105.0);
        snapshot["exit"] = evidence;
        let closed = invoke_at("lifecycle", saved, snapshot, 100_000_001);
        assert_eq!(closed["status"], "executed", "{closed}");
        assert!(closed["result"]["state"]["entry"].is_null());
    }

    #[test]
    fn exit_rejection_can_refresh_but_changed_position_cannot_dispatch() {
        let mut state = observed_fill("short", 0.5);
        fresh_exit_frames(&mut state);
        state.frames[1].sell.push(exit_level(94.0));
        let ready = invoke(
            "exit_ready",
            serde_json::to_value(state).unwrap(),
            Value::Null,
        );
        let sent = invoke("place_exit", ready["result"]["state"].clone(), Value::Null);
        let saved = sent["result"]["state"].clone();
        let mut changed = saved.clone();
        changed["entry"]["position"]["quantity"] = json!(0.1);
        assert_eq!(
            invoke("validate_exit", changed, Value::Null)["status"],
            "rejected"
        );
        let evidence = json!({"account_id":"a".repeat(64),"symbol":"BTC-USDT", "client_order_id":"c".repeat(40),
            "order_id":null,"status":"not_sent","filled_quantity":0.0,"average_price":0.0,"observed_at_ms":100_000_000});
        let rejected = invoke("exit", saved, evidence);
        assert_eq!(rejected["status"], "executed", "{rejected}");
        assert!(rejected["result"]["state"]["entry"]["profit_exit"].is_null());
        let fresh = invoke(
            "exit_ready",
            rejected["result"]["state"].clone(),
            Value::Null,
        );
        assert_eq!(
            fresh["result"]["view"]["confirmation"]["plan"]["price"],
            94.0
        );
    }

    fn settings() -> Settings {
        Settings {
            detection_length: 3,
            ..Settings::default()
        }
    }

    fn account() -> AccountSnapshot {
        AccountSnapshot {
            symbol: "BTC-USDT".into(),
            endpoint: "LIVE".into(),
            account_id: "a".repeat(64),
            account_label: "BingX LIVE / account ending 1234".into(),
            available_margin: 30.0,
            long_leverage: 50,
            short_leverage: 20,
            price_precision: 2,
            quantity_precision: 3,
            min_quantity: 0.001,
            min_notional: 5.0,
            observed_at_ms: 100_000_000,
        }
    }

    fn entry_state() -> State {
        let mut state = State::new(settings());
        state.account = Some(account());
        state.current_price = Some(100.0);
        state.observed_at = Some(100_000_000);
        state.complete = true;
        state.frames.push(Frame {
            timeframe: "5m".into(),
            sell: vec![Level {
                origin: 10,
                first_known: 20,
                price: 96.5,
                top: 98.0,
                bottom: 95.0,
                touched: false,
                consumed: false,
            }],
            ..Frame::default()
        });
        state
    }

    fn lifecycle_state(status: &str, filled: f64) -> Value {
        let prepared = invoke(
            "prepare",
            serde_json::to_value(entry_state()).unwrap(),
            Value::Null,
        );
        let placed = invoke(
            "place_entry",
            prepared["result"]["state"].clone(),
            Value::Null,
        );
        let mut state = placed["result"]["state"].clone();
        let qty = state["entry"]["plan"]["quantity"].as_f64().unwrap();
        state["entry"]["evidence"] = json!({
            "account_id":"a".repeat(64), "symbol":"BTC-USDT", "client_order_id":"b".repeat(40),
            "order_id":"123", "status":status, "filled_quantity":qty * filled,
            "average_price": if filled > 0.0 {96.5} else {0.0}, "observed_at_ms":100_000_000,
        });
        state
    }

    fn lifecycle_snapshot(state: &Value, positions: Value, orders: Value, now: i64) -> Value {
        let mut order = state["entry"]["evidence"].clone();
        order["observed_at_ms"] = json!(now);
        json!({"account_id":"a".repeat(64), "symbol":"BTC-USDT", "entry":order,
            "positions":positions, "orders":orders, "observed_at_ms":now})
    }

    fn select_symbol(state: Value, symbol: &str, allow_new_entries: bool) -> Value {
        let mut selected = settings();
        selected.symbol = symbol.into();
        let raw = json!({"schema_version":1,"plugin_id":PLUGIN_ID,"host_method":"workspace",
            "action":"inspect","state":state,"settings":selected,"observed_at_ms":100_000_001,
            "execution":{"account_id":"a".repeat(64),"symbol":"BTC-USDT",
                "allow_new_entries":allow_new_entries}});
        serde_json::from_slice(&evaluate_json(&serde_json::to_vec(&raw).unwrap())).unwrap()
    }

    #[test]
    fn managed_order_refresh_can_switch_instrument_without_touching_order() {
        let active = lifecycle_state("filled", 1.0);
        let mut selected = settings();
        selected.symbol = "DASH-USDT".into();
        let raw = json!({"schema_version":1,"plugin_id":PLUGIN_ID,"host_method":"workspace",
            "action":"refresh_order","state":active,"settings":selected,
            "observed_at_ms":100_000_001,
            "execution":{"account_id":"a".repeat(64),"symbol":"BTC-USDT",
                "allow_new_entries":false}});
        let switched: Value =
            serde_json::from_slice(&evaluate_json(&serde_json::to_vec(&raw).unwrap())).unwrap();
        assert_eq!(switched["status"], "executed", "{switched}");
        assert_eq!(
            switched["result"]["state"]["settings"]["symbol"],
            "DASH-USDT"
        );
        assert_eq!(
            switched["result"]["state"]["parked"][0]["settings"]["symbol"],
            "BTC-USDT"
        );
        assert_eq!(switched["result"]["resume_action"], "present");
        let requests = switched["result"]["requests"].as_array().unwrap();
        assert!(requests
            .iter()
            .any(|r| r["kind"] == "account.snapshot.read" && r["symbol"] == "DASH-USDT"));
        assert!(requests.iter().all(|r| r["kind"] != "order.snapshot.read"
            && r["kind"] != "order.entry.place"
            && r["kind"] != "order.cancel"));
    }

    #[test]
    fn returning_to_retired_instrument_does_not_resurrect_its_old_order() {
        let mut retired = lifecycle_state("filled", 1.0);
        let old_plan = retired["entry"]["plan"].clone();
        let retired_at = old_plan["prepared_at_ms"].as_i64().unwrap() + 1;
        retired["entry"] = Value::Null;
        retired["retired_before_ms"] = json!(retired_at);
        let switched = select_symbol(retired, "DASH-USDT", false);
        assert_eq!(switched["status"], "executed", "{switched}");
        let saved = &switched["result"]["state"]["parked"][0];
        assert!(saved["entry"].is_null());
        assert_eq!(saved["retired_before_ms"], retired_at);
        let reopened = invoke("open", switched["result"]["state"].clone(), Value::Null);
        assert_eq!(reopened["status"], "executed", "{reopened}");
        let returned = select_symbol(reopened["result"]["state"].clone(), "BTC-USDT", false);
        assert_eq!(returned["status"], "executed", "{returned}");
        assert_eq!(returned["result"]["state"]["retired_before_ms"], retired_at);
        let recovered = invoke(
            "restore_entry",
            returned["result"]["state"].clone(),
            json!({"account_id":"a".repeat(64),"batch_complete":true,
                "operations":[{"account_binding_id":"a".repeat(64),
                    "provider_id":"bingx","effect_kind":"order.entry.place",
                    "state":"succeeded","canonical_payload_json":old_plan.to_string()}]}),
        );
        assert_eq!(recovered["status"], "executed", "{recovered}");
        assert!(recovered["result"]["state"]["entry"].is_null());
        assert_eq!(
            recovered["result"]["state"]["retired_before_ms"],
            retired_at
        );
    }

    #[test]
    fn switching_instruments_preserves_managed_entry_and_ignores_foreign_journal() {
        let active = lifecycle_state("filled", 1.0);
        assert_eq!(
            select_symbol(active.clone(), "DASH-USDT", true)["status"],
            "rejected"
        );
        let switched = select_symbol(active.clone(), "DASH-USDT", false);
        assert_eq!(switched["status"], "executed", "{switched}");
        let dash = switched["result"]["state"].clone();
        assert_eq!(dash["settings"]["symbol"], "DASH-USDT");
        assert!(dash["entry"].is_null());
        assert_eq!(dash["parked"][0]["entry"]["plan"], active["entry"]["plan"]);
        assert!(switched["result"]["requests"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["kind"] != "order.entry.place"));
        let restarted = invoke("open", dash, Value::Null);
        assert_eq!(restarted["status"], "executed", "{restarted}");
        assert_eq!(
            restarted["result"]["requests"][0]["kind"],
            "account.snapshot.read"
        );
        assert_eq!(restarted["result"]["requests"][0]["symbol"], "DASH-USDT");
        let mut dash_account = account();
        dash_account.symbol = "DASH-USDT".into();
        dash_account.observed_at_ms = 100_000_001;
        let connected = invoke_at(
            "account",
            restarted["result"]["state"].clone(),
            serde_json::to_value(dash_account).unwrap(),
            100_000_001,
        );
        assert_eq!(connected["status"], "executed", "{connected}");
        let dash = connected["result"]["state"].clone();
        let old_journal = invoke(
            "restore_entry",
            dash.clone(),
            json!({
                "account_id":"a".repeat(64), "batch_complete":true,
                "operations":[{"account_binding_id":"a".repeat(64),"provider_id":"bingx",
                    "effect_kind":"order.entry.place","state":"succeeded",
                    "canonical_payload_json":active["entry"]["plan"].to_string()}]
            }),
        );
        assert_eq!(old_journal["status"], "executed", "{old_journal}");
        assert!(old_journal["result"]["state"]["entry"].is_null());
        assert_eq!(
            old_journal["result"]["state"]["settings"]["symbol"],
            "DASH-USDT"
        );
        let returned = select_symbol(dash, "BTC-USDT", false);
        assert_eq!(returned["status"], "executed", "{returned}");
        assert_eq!(
            returned["result"]["state"]["entry"]["plan"],
            active["entry"]["plan"]
        );
        assert_eq!(
            returned["result"]["state"]["entry"]["evidence"],
            active["entry"]["evidence"]
        );
        assert!(returned["result"]["state"]["parked"].is_null());
        let resumed = invoke(
            "refresh_order",
            returned["result"]["state"].clone(),
            Value::Null,
        );
        assert_eq!(resumed["status"], "executed", "{resumed}");
        assert_eq!(resumed["result"]["requests"][0]["scope"], "lifecycle");
        assert_eq!(
            resumed["result"]["requests"][0]["plan"]["symbol"],
            "BTC-USDT"
        );
    }

    fn cycle_invoke(action: &str, state: Value, allowed: bool) -> Value {
        let raw = json!({
            "schema_version":1, "plugin_id":PLUGIN_ID, "host_method":"workspace",
            "action":action, "state":state, "observed_at_ms":100_000_001,
            "execution":{"account_id":"a".repeat(64), "symbol":"BTC-USDT", "allow_new_entries":allowed}
        });
        serde_json::from_slice(&evaluate_json(&serde_json::to_vec(&raw).unwrap())).unwrap()
    }

    #[test]
    fn cycle_prepares_only_with_authority_and_a_real_candidate() {
        let initial = serde_json::to_value(entry_state()).unwrap();
        let stopped = cycle_invoke("cycle_ready", initial.clone(), false);
        assert!(stopped["result"]["state"]["entry"].is_null());
        let ready = cycle_invoke("cycle_ready", initial.clone(), true);
        assert_eq!(ready["result"]["state"]["entry"]["attempted"], false);
        assert!(ready["result"]["view"]["confirmation"].is_object());
        assert!(ready["result"]["requests"].as_array().unwrap().is_empty());
        let mut no_line = initial.clone();
        no_line["frames"] = json!([]);
        let waiting = cycle_invoke("cycle_ready", no_line, true);
        assert!(waiting["result"]["state"]["entry"].is_null());
        let missing = invoke("cycle_ready", initial.clone(), Value::Null);
        assert_eq!(missing["status"], "rejected");
        let mut foreign = initial;
        foreign["account"]["account_id"] = json!("b".repeat(64));
        assert_eq!(
            cycle_invoke("cycle_ready", foreign, true)["status"],
            "rejected"
        );
    }

    #[test]
    fn cycles_reconcile_before_rearming_and_start_does_not_submit() {
        let managed = lifecycle_state("partial", 0.5);
        let reconciled = cycle_invoke("cycle", managed.clone(), true);
        let reads = reconciled["result"]["requests"].as_array().unwrap();
        assert_eq!(reads.len(), 7);
        assert_eq!(reads[0]["scope"], "lifecycle");
        assert!(reads.iter().all(|r| r["kind"] != "order.entry.place"));
        assert_eq!(reconciled["result"]["resume_action"], "exit_ready");
        let retained = cycle_invoke("cycle_ready", managed, true);
        assert_eq!(
            retained["result"]["state"]["entry"]["evidence"]["status"],
            "partial"
        );
        let initial = serde_json::to_value(entry_state()).unwrap();
        let next = cycle_invoke("cycle", initial.clone(), true);
        assert_eq!(next["result"]["resume_action"], "cycle_ready");
        let raw = json!({"schema_version":1,"plugin_id":PLUGIN_ID,"host_method":"workspace",
            "action":"start", "state":initial, "settings":settings(), "observed_at_ms":100_000_001});
        let start: Value =
            serde_json::from_slice(&evaluate_json(&serde_json::to_vec(&raw).unwrap())).unwrap();
        assert!(start["result"]["state"]["entry"].is_null());
        assert!(start["result"]["requests"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["kind"] != "order.entry.place"));
    }

    #[test]
    fn lifecycle_keeps_unknown_or_partial_exposure() {
        for (status, filled) in [("partial", 0.5), ("open", 0.0), ("unknown", 0.0)] {
            let state = lifecycle_state(status, filled);
            let output = invoke(
                "lifecycle",
                state.clone(),
                lifecycle_snapshot(&state, json!([]), json!([]), 100_000_000),
            );
            assert_eq!(output["status"], "executed", "{output}");
            assert!(!output["result"]["state"]["entry"].is_null(), "{status}");
            assert!(output["result"]["requests"].as_array().unwrap().is_empty());
        }
    }

    #[test]
    fn fill_closed_between_polls_requires_two_fresh_flat_snapshots_before_rearm() {
        let state = lifecycle_state("filled", 1.0);
        let first = invoke(
            "lifecycle",
            state.clone(),
            lifecycle_snapshot(&state, json!([]), json!([]), 100_000_000),
        );
        assert_eq!(first["status"], "executed", "{first}");
        let waiting = first["result"]["state"].clone();
        assert!(!waiting["entry"].is_null());
        assert_eq!(waiting["entry"]["observed_position_id"], Value::Null);
        assert_eq!(waiting["entry"]["flat_observed_at_ms"], 100_000_000);

        let second = invoke_at(
            "lifecycle",
            waiting.clone(),
            lifecycle_snapshot(&waiting, json!([]), json!([]), 100_000_001),
            100_000_001,
        );
        assert_eq!(second["status"], "executed", "{second}");
        let retired = &second["result"]["state"];
        assert!(retired["entry"].is_null());
        assert_eq!(retired["retired_before_ms"], 100_000_000);
        assert_eq!(retired["frames"][0]["sell"][0]["consumed"], true);
    }

    #[test]
    fn observed_position_closure_survives_restart_and_retires_history_without_pnl() {
        for terminal in ["filled", "cancelled", "expired"] {
            let mut state = lifecycle_state("filled", 1.0);
            state["entry"]["evidence"]["status"] = json!(terminal);
            let quantity = state["entry"]["plan"]["quantity"].clone();
            let position = json!({"position_id":"456", "side":"long", "quantity":quantity, "average_price":96.5});
            let open = invoke(
                "lifecycle",
                state.clone(),
                lifecycle_snapshot(&state, json!([position]), json!([]), 100_000_000),
            );
            assert_eq!(open["status"], "executed", "{open}");
            let reopened: Value =
                serde_json::from_slice(&serde_json::to_vec(&open["result"]["state"]).unwrap())
                    .unwrap();
            assert_eq!(reopened["entry"]["observed_position_id"], "456");
            let remains = invoke_at(
                "lifecycle",
                reopened.clone(),
                lifecycle_snapshot(
                    &reopened,
                    json!([]),
                    json!([{"order_id":"789", "side":"SELL", "position_side":"LONG", "type":"STOP_MARKET", "status":"NEW", "price":"0", "stop_price":"96", "quantity":"1", "filled_quantity":"0"}]),
                    100_000_001,
                ),
                100_000_001,
            );
            assert!(!remains["result"]["state"]["entry"].is_null());
            let closed = invoke_at(
                "lifecycle",
                reopened.clone(),
                lifecycle_snapshot(&reopened, json!([]), json!([]), 100_000_001),
                100_000_001,
            );
            assert_eq!(closed["status"], "executed", "{closed}");
            let retired = closed["result"]["state"].clone();
            assert!(retired["entry"].is_null());
            assert_eq!(retired["retired_before_ms"], 100_000_000);
            assert_eq!(retired["frames"][0]["sell"][0]["touched"], false);
            assert_eq!(retired["frames"][0]["sell"][0]["consumed"], true);
            assert_eq!(retired["complete"], false);
            let restored = invoke_at(
                "restore_entry",
                retired.clone(),
                json!({"account_id":"a".repeat(64), "batch_complete":true,
                "operations":[{"account_binding_id":"a".repeat(64), "provider_id":"bingx", "effect_kind":"order.entry.place", "state":"succeeded",
                    "canonical_payload_json":serde_json::to_string(&state["entry"]["plan"]).unwrap()}]}),
                100_000_002,
            );
            assert_eq!(restored["status"], "executed", "{restored}");
            assert!(restored["result"]["state"]["entry"].is_null());
            assert_eq!(
                restored["result"]["state"]["retired_before_ms"],
                retired["retired_before_ms"]
            );
            let loaded: State = serde_json::from_value(retired).unwrap();
            assert!(candidate(&loaded).is_none());
        }
    }

    #[test]
    fn unfilled_terminal_requires_complete_flat_evidence_and_new_entry_identity() {
        let state = lifecycle_state("cancelled", 0.0);
        let closed = invoke(
            "lifecycle",
            state.clone(),
            lifecycle_snapshot(&state, json!([]), json!([]), 100_000_000),
        );
        assert_eq!(closed["status"], "executed", "{closed}");
        let mut fresh = closed["result"]["state"].clone();
        assert!(fresh["entry"].is_null());
        fresh["complete"] = json!(true);
        fresh["account"]["observed_at_ms"] = json!(100_000_001);
        fresh["observed_at"] = json!(100_000_001);
        let same_clock = invoke("prepare", fresh.clone(), Value::Null);
        assert_eq!(same_clock["status"], "rejected");
        let next = invoke_at("prepare", fresh, Value::Null, 100_000_001);
        assert_eq!(next["status"], "executed", "{next}");
        assert_eq!(
            next["result"]["state"]["entry"]["plan"]["prepared_at_ms"],
            100_000_001
        );
        assert_eq!(next["result"]["state"]["entry"]["attempted"], false);
    }

    #[test]
    fn lifecycle_rejects_stale_foreign_incomplete_and_regressing_evidence() {
        let state = lifecycle_state("filled", 1.0);
        let snapshot = lifecycle_snapshot(&state, json!([]), json!([]), 100_000_000);
        let mut mutations = Vec::new();
        let mut foreign = snapshot.clone();
        foreign["account_id"] = json!("f".repeat(64));
        mutations.push(foreign);
        let mut foreign = snapshot.clone();
        foreign["symbol"] = json!("ETH-USDT");
        mutations.push(foreign);
        let mut stale = snapshot.clone();
        stale["observed_at_ms"] = json!(99_939_999);
        mutations.push(stale);
        let mut future = snapshot.clone();
        future["observed_at_ms"] = json!(100_000_001);
        mutations.push(future);
        let mut missing = snapshot.clone();
        missing.as_object_mut().unwrap().remove("orders");
        mutations.push(missing);
        let mut regression = snapshot.clone();
        regression["entry"]["status"] = json!("open");
        mutations.push(regression);
        for evidence in mutations {
            let result = invoke("lifecycle", state.clone(), evidence);
            assert_eq!(result["status"], "rejected", "{result}");
        }
    }

    fn observed_fill(side: &str, fraction: f64) -> State {
        let mut state = lifecycle_state("partial", fraction);
        if fraction == 1.0 {
            state["entry"]["evidence"]["status"] = json!("filled");
        }
        let average = if side == "long" { 96.4 } else { 96.6 };
        state["entry"]["plan"]["side"] = json!(side);
        if side == "short" {
            state["entry"]["plan"]["stop_price"] = json!(96.88);
        }
        state["entry"]["evidence"]["average_price"] = json!(average);
        let snapshot = lifecycle_snapshot(
            &state,
            json!([{
                "position_id":"456", "side":side,
                "quantity":state["entry"]["evidence"]["filled_quantity"],
                "average_price":average,
            }]),
            json!([]),
            100_000_000,
        );
        let output = invoke("lifecycle", state, snapshot);
        assert_eq!(output["status"], "executed", "{output}");
        assert!(output["result"]["requests"].as_array().unwrap().is_empty());
        serde_json::from_value(output["result"]["state"].clone()).unwrap()
    }

    fn fresh_exit_frames(state: &mut State) {
        let now = 100_000_000;
        state.complete = true;
        state.current_price = Some(100.0);
        state.observed_at = Some(now);
        state.frames = TIMEFRAMES
            .iter()
            .map(|(tf, span)| {
                let time = now - now % span - span;
                Frame {
                    timeframe: (*tf).into(),
                    checked_at_ms: Some(now),
                    last_open: Some(time),
                    bars: vec![Candle(time, 100.0, 101.0, 99.0, 100.0)],
                    ..Frame::default()
                }
            })
            .collect();
    }

    fn exit_level(price: f64) -> Level {
        Level {
            origin: 10,
            first_known: 20,
            price,
            top: price + 1.0,
            bottom: price - 1.0,
            touched: false,
            consumed: false,
        }
    }

    #[test]
    fn fill_stop_uses_actual_average_and_scales_only_remaining_exposure() {
        for side in ["long", "short"] {
            let half = observed_fill(side, 0.5);
            let full = observed_fill(side, 1.0);
            let position = half.entry.as_ref().unwrap().position.as_ref().unwrap();
            let (stop, margin, loss) = position_stop(&half, position).unwrap();
            let full_position = full.entry.as_ref().unwrap().position.as_ref().unwrap();
            let (full_stop, full_margin, full_loss) = position_stop(&full, full_position).unwrap();
            assert_eq!(stop, full_stop);
            assert!((margin * 2.0 - full_margin).abs() < 1e-12);
            assert!((loss * 2.0 - full_loss).abs() < 1e-12);
            assert!(loss <= margin * half.settings.stop_percent / 100.0);
            assert!((stop - if side == "long" { 96.03 } else { 96.98 }).abs() < 1e-9);
            assert_ne!(stop, half.entry.as_ref().unwrap().plan.stop_price);
            let mut reduced = position.clone();
            reduced.quantity /= 2.0;
            let (reduced_stop, reduced_margin, reduced_loss) =
                position_stop(&half, &reduced).unwrap();
            assert_eq!(reduced_stop, stop);
            assert!((reduced_margin * 2.0 - margin).abs() < 1e-12);
            assert!((reduced_loss * 2.0 - loss).abs() < 1e-12);
            half.validate().unwrap();
        }
    }

    #[test]
    fn profit_candidate_uses_only_profitable_untouched_opposite_lines_and_same_ties() {
        let mut state = observed_fill("long", 1.0);
        fresh_exit_frames(&mut state);
        state.frames[1].buy.push(exit_level(105.0));
        state.frames[2].buy.push(exit_level(105.0));
        state.frames[5].buy.push(exit_level(110.0));
        state.frames[0].buy.push(exit_level(96.0));
        state.frames[0].sell.push(exit_level(100.5));
        let position = state
            .entry
            .as_ref()
            .unwrap()
            .position
            .as_ref()
            .unwrap()
            .clone();
        let candidate = profit_candidate(&state, &position, 100_000_000).unwrap();
        assert_eq!((candidate.0, candidate.1.price), ("4h", 105.0));
        state.frames[1].buy[0].touched = true;
        assert_eq!(
            profit_candidate(&state, &position, 100_000_000).unwrap().0,
            "1h"
        );
        state.frames[2].buy[0].consumed = true;
        assert_eq!(
            profit_candidate(&state, &position, 100_000_000)
                .unwrap()
                .1
                .price,
            110.0
        );
        state.frames[5].buy[0].first_known = 100_000_001;
        assert!(profit_candidate(&state, &position, 100_000_000).is_none());

        let mut short = observed_fill("short", 1.0);
        fresh_exit_frames(&mut short);
        short.current_price = Some(95.0);
        short.frames[1].sell.push(exit_level(94.0));
        short.frames[2].sell.push(exit_level(93.0));
        short.frames[0].sell.push(exit_level(97.0));
        short.frames[0].buy.push(exit_level(94.5));
        let position = short.entry.as_ref().unwrap().position.as_ref().unwrap();
        assert_eq!(
            profit_candidate(&short, position, 100_000_000)
                .unwrap()
                .1
                .price,
            94.0
        );
    }

    #[test]
    fn position_preview_survives_reopen_without_claiming_protection_or_dispatch() {
        let mut state = observed_fill("long", 0.5);
        fresh_exit_frames(&mut state);
        state.frames[1].buy.push(exit_level(105.0));
        let saved = serde_json::to_value(&state).unwrap();
        let result = invoke("present", saved.clone(), Value::Null);
        assert_eq!(result["status"], "executed", "{result}");
        assert_eq!(result["result"]["state"], saved);
        assert!(result["result"]["requests"].as_array().unwrap().is_empty());
        let message = result["result"]["view"]["message"].as_str().unwrap();
        assert!(message.contains("average 96.4"));
        assert!(message.contains("Fill-based stop estimate: 96.03"));
        assert!(message.contains("105 on 4H"));
        assert!(message.contains("no profit exit is placed or fixed yet"));
        assert!(message.contains("not confirmation of exchange-side protection"));
        assert!(result["result"]["view"]["confirmation"].is_null());
        let stale = invoke_at("present", saved, Value::Null, 100_060_001);
        let message = stale["result"]["view"]["message"].as_str().unwrap();
        assert!(message.contains("Saved position 456 is stale"));
        assert!(!message.contains("Opposite-line exit candidate:"));
    }

    #[test]
    fn partial_or_stale_market_cannot_create_a_profit_candidate() {
        let mut state = observed_fill("long", 1.0);
        fresh_exit_frames(&mut state);
        state.frames[1].buy.push(exit_level(105.0));
        let position = state
            .entry
            .as_ref()
            .unwrap()
            .position
            .as_ref()
            .unwrap()
            .clone();
        state.complete = false;
        assert!(profit_candidate(&state, &position, 100_000_000).is_none());
        state.complete = true;
        state.frames[5].checked_at_ms = None;
        assert!(profit_candidate(&state, &position, 100_000_000).is_none());
        state.frames[5].checked_at_ms = Some(99_939_999);
        assert!(profit_candidate(&state, &position, 100_000_000).is_none());
    }

    #[test]
    fn conflicting_positions_are_not_sized_and_flat_read_does_not_hide_a_fill() {
        let state = lifecycle_state("partial", 0.5);
        let quantity = state["entry"]["evidence"]["filled_quantity"]
            .as_f64()
            .unwrap();
        let position =
            json!({"position_id":"456", "side":"long", "quantity":quantity, "average_price":96.5});
        let changed = |key: &str, value: Value| {
            let mut changed = position.clone();
            changed[key] = value;
            changed
        };
        for positions in [
            json!([]),
            json!([changed("quantity", json!(quantity * 2.0))]),
            json!([changed("side", json!("short"))]),
            json!([changed("average_price", json!(97.0))]),
            json!([position.clone(), changed("position_id", json!("789"))]),
        ] {
            let output = invoke(
                "lifecycle",
                state.clone(),
                lifecycle_snapshot(&state, positions, json!([]), 100_000_000),
            );
            assert_eq!(output["status"], "executed", "{output}");
            assert!(!output["result"]["state"]["entry"].is_null());
            assert!(output["result"]["state"]["entry"]["position"].is_null());
            assert!(output["result"]["state"]["entry"]["observed_position_id"].is_null());
            assert!(output["result"]["requests"].as_array().unwrap().is_empty());
        }
        let duplicate = invoke(
            "lifecycle",
            state.clone(),
            lifecycle_snapshot(
                &state,
                json!([position.clone(), position]),
                json!([]),
                100_000_000,
            ),
        );
        assert_eq!(duplicate["status"], "rejected");
    }

    #[test]
    fn exact_order_refresh_invalidates_saved_position_and_corrupt_state_is_rejected() {
        let state = observed_fill("long", 0.5);
        let value = serde_json::to_value(&state).unwrap();
        let refreshed = invoke("order", value.clone(), value["entry"]["evidence"].clone());
        assert_eq!(refreshed["status"], "executed", "{refreshed}");
        assert!(refreshed["result"]["state"]["entry"]["position"].is_null());
        for key in ["side", "position_id", "quantity", "average_price"] {
            let mut damaged = value.clone();
            damaged["entry"]["position"][key] = match key {
                "side" => json!("short"),
                "position_id" => json!("789"),
                "quantity" => json!(999.0),
                _ => json!(97.0),
            };
            assert_eq!(invoke("open", damaged, Value::Null)["status"], "rejected");
        }
    }

    #[test]
    fn prepared_entry_uses_the_same_candidate_and_sizing_as_preview() {
        let state = entry_state();
        let prepared = invoke(
            "prepare",
            serde_json::to_value(state.clone()).unwrap(),
            Value::Null,
        );
        assert_eq!(prepared["status"], "executed", "{prepared}");
        let plan = &prepared["result"]["state"]["entry"]["plan"];
        assert_eq!(plan["price"], 96.5);
        assert_eq!(plan["quantity"], 0.518);
        assert_eq!(plan["stop_price"], 96.12);
        assert_eq!(plan["side"], "long");
        assert_eq!(plan["first_known"], 20);
        assert_eq!(plan["expires_at_ms"], 100_000_000 + ENTRY_HISTORY_WINDOW);
        assert!(prepared["result"]["requests"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(prepared["result"]["view"]["confirmation"]["plan"], *plan);
        let placed = invoke(
            "place_entry",
            prepared["result"]["state"].clone(),
            Value::Null,
        );
        assert_eq!(placed["status"], "executed", "{placed}");
        assert_eq!(placed["result"]["requests"][0]["kind"], "order.entry.place");
        assert_eq!(placed["result"]["requests"][0]["plan"], *plan);
        let resumed = placed["result"]["state"].clone();
        assert_eq!(
            invoke("place_entry", resumed.clone(), Value::Null)["status"],
            "rejected"
        );
        assert_eq!(
            invoke("prepare", resumed.clone(), Value::Null)["status"],
            "rejected"
        );
        let reopened = invoke("open", resumed.clone(), Value::Null);
        assert_eq!(reopened["result"]["state"], resumed);
        assert!(reopened["result"]["requests"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(
            invoke("refresh_order", resumed, Value::Null)["result"]["requests"][0]["kind"],
            "order.snapshot.read"
        );
    }

    fn managed_pending_state(now: i64) -> State {
        let prepared = invoke(
            "prepare",
            serde_json::to_value(entry_state()).unwrap(),
            Value::Null,
        );
        let placed = invoke(
            "place_entry",
            prepared["result"]["state"].clone(),
            Value::Null,
        );
        let mut state: State = serde_json::from_value(placed["result"]["state"].clone()).unwrap();
        let line = state.frames[0].sell[0].clone();
        state.observed_at = Some(now);
        state.frames = TIMEFRAMES
            .iter()
            .map(|(tf, span)| {
                let last = now - now % span - span;
                Frame {
                    timeframe: (*tf).into(),
                    checked_at_ms: Some(now),
                    last_open: Some(last),
                    bars: vec![Candle(last, 100., 101., 99., 100.)],
                    sell: if *tf == "5m" {
                        vec![line.clone()]
                    } else {
                        vec![]
                    },
                    ..Frame::default()
                }
            })
            .collect();
        state.entry.as_mut().unwrap().evidence = Some(OrderEvidence {
            account_id: "a".repeat(64),
            symbol: "BTC-USDT".into(),
            client_order_id: "c".repeat(40),
            order_id: Some("2105918659918786560".into()),
            status: "open".into(),
            filled_quantity: 0.,
            average_price: 0.,
            observed_at_ms: now,
            error_message: None,
        });
        state.validate().unwrap();
        state
    }

    #[test]
    fn pending_line_checks_original_identity_not_nearest_candidate_and_never_requests_effects() {
        let now = 1_800_000_123_456;
        let mut state = managed_pending_state(now);
        let entry = state.entry.clone().unwrap();
        assert!(pending_line_status(&state, &entry, now).contains("unchanged"));
        state.frames[2].sell.push(Level {
            price: 99.,
            origin: 30,
            first_known: 40,
            top: 99.5,
            bottom: 98.5,
            touched: false,
            consumed: false,
        });
        assert_eq!(candidate(&state).unwrap().0, "1h");
        assert!(pending_line_status(&state, &entry, now).contains("unchanged"));
        state.frames[5].sell[0].price = 96.;
        assert!(pending_line_status(&state, &entry, now).contains("moved"));
        state.frames[5].sell[0].price = entry.plan.line_price;
        state.frames[5].sell[0].touched = true;
        assert!(pending_line(&state, &entry, now) == PendingLine::Touched);
        state.frames[5].sell[0].touched = false;
        state.frames[5].sell[0].first_known += 1;
        assert!(pending_line_status(&state, &entry, now).contains("no longer present"));
        let reopened = invoke_at(
            "open",
            serde_json::to_value(state).unwrap(),
            Value::Null,
            now,
        );
        assert_eq!(reopened["status"], "executed", "{reopened}");
        assert!(reopened["result"]["requests"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(
            reopened["result"]["state"]["entry"]["plan"],
            serde_json::to_value(entry.plan).unwrap()
        );
        assert_eq!(
            reopened["result"]["view"]["confirmation"]["kind"],
            "order.entry.cancel"
        );
        assert_eq!(
            reopened["result"]["view"]["confirmation"]["order_id"],
            entry.evidence.unwrap().order_id.unwrap()
        );
    }

    #[test]
    fn invalidation_cancels_only_exact_unfilled_entries_and_a_fill_race_retains_the_trade() {
        let now = 1_800_000_123_456;
        let mut state = managed_pending_state(now);
        assert!(pending_cancellation(&state, now).is_none());
        state.frames[5].sell[0].price -= 0.5;
        let initial = serde_json::to_value(&state).unwrap();
        let cancelled = invoke_at("cancel_entry", initial.clone(), Value::Null, now);
        assert_eq!(cancelled["status"], "executed", "{cancelled}");
        let request = &cancelled["result"]["requests"][0];
        assert_eq!(request["kind"], "order.entry.cancel");
        assert_eq!(request["plan"], initial["entry"]["plan"]);
        assert_eq!(
            request["order_id"],
            initial["entry"]["evidence"]["order_id"]
        );
        assert_eq!(cancelled["result"]["state"], initial);
        let checked = invoke_at("validate_cancel", initial.clone(), Value::Null, now);
        assert!(checked["result"]["requests"].as_array().unwrap().is_empty());
        for fault in ["partial", "unknown", "stale", "incomplete", "missing_id"] {
            let mut changed = state.clone();
            match fault {
                "partial" => {
                    let e = changed.entry.as_mut().unwrap().evidence.as_mut().unwrap();
                    e.status = "partial".into();
                    e.filled_quantity = 0.1;
                    e.average_price = 96.4;
                }
                "unknown" => {
                    changed
                        .entry
                        .as_mut()
                        .unwrap()
                        .evidence
                        .as_mut()
                        .unwrap()
                        .status = "unknown".into()
                }
                "stale" => {
                    changed
                        .entry
                        .as_mut()
                        .unwrap()
                        .evidence
                        .as_mut()
                        .unwrap()
                        .observed_at_ms -= 60_001
                }
                "incomplete" => changed.complete = false,
                _ => {
                    changed
                        .entry
                        .as_mut()
                        .unwrap()
                        .evidence
                        .as_mut()
                        .unwrap()
                        .order_id = None
                }
            }
            assert!(pending_cancellation(&changed, now).is_none(), "{fault}");
            let result = invoke_at(
                "cancel_entry",
                serde_json::to_value(changed).unwrap(),
                Value::Null,
                now,
            );
            assert_eq!(result["status"], "rejected", "{fault}: {result}");
        }
        let mut race = lifecycle_snapshot(&initial, json!([]), json!([]), now);
        race["entry"]["status"] = json!("partial");
        race["entry"]["filled_quantity"] = json!(0.1);
        race["entry"]["average_price"] = json!(96.4);
        let retained = invoke_at("lifecycle", initial.clone(), race, now);
        assert_eq!(retained["status"], "executed", "{retained}");
        assert!(!retained["result"]["state"]["entry"].is_null());
        assert!(retained["result"]["view"]["confirmation"].is_null());
        let mut closed = lifecycle_snapshot(&initial, json!([]), json!([]), now);
        closed["entry"]["status"] = json!("cancelled");
        let retired = invoke_at("lifecycle", initial, closed, now);
        assert!(retired["result"]["state"]["entry"].is_null());
        assert!(retired["result"]["requests"].as_array().unwrap().is_empty());
    }

    #[test]
    fn fills_unknown_terminal_and_stale_evidence_cannot_authorize_pending_replacement() {
        let now = 1_800_000_123_456;
        let mut state = managed_pending_state(now);
        let mut entry = state.entry.clone().unwrap();
        for status in ["partial", "filled", "cancelled", "expired"] {
            let e = entry.evidence.as_mut().unwrap();
            e.status = status.into();
            e.filled_quantity = 0.1;
            e.average_price = 96.4;
            assert!(pending_line_status(&state, &entry, now).contains("actual position"));
        }
        let e = entry.evidence.as_mut().unwrap();
        e.filled_quantity = 0.;
        e.average_price = 0.;
        e.status = "cancelled".into();
        assert!(pending_line_status(&state, &entry, now).contains("No pending entry"));
        entry.evidence.as_mut().unwrap().status = "unknown".into();
        assert!(pending_line_status(&state, &entry, now).contains("refresh the exact order"));
        entry.evidence.as_mut().unwrap().status = "open".into();
        assert!(
            pending_line_status(&state, &entry, now + 60_001).contains("refresh the exact order")
        );
        state.complete = false;
        assert!(pending_line_status(&state, &entry, now).contains("not available"));
        state.complete = true;
        state.frames[0].checked_at_ms = None;
        assert!(pending_line_status(&state, &entry, now).contains("not available"));
        entry.evidence = None;
        assert!(pending_line_status(&state, &entry, now).contains("unknown"));
    }

    #[test]
    fn pending_refresh_streams_all_frames_without_changing_plan_and_recovers_after_interruption() {
        let now = 1_800_000_123_456;
        let state = managed_pending_state(now);
        let plan = serde_json::to_value(&state.entry.as_ref().unwrap().plan).unwrap();
        let out = invoke_at(
            "refresh_order",
            serde_json::to_value(state).unwrap(),
            Value::Null,
            now,
        );
        assert_eq!(out["status"], "executed", "{out}");
        let requests = out["result"]["requests"].as_array().unwrap();
        assert_eq!(requests.len(), 7);
        assert_eq!(requests[0]["kind"], "order.snapshot.read");
        assert_eq!(requests[0]["plan"], plan);
        assert_eq!(out["result"]["resume_action"], "exit_ready");
        for ((tf, _), r) in TIMEFRAMES.iter().zip(&requests[1..]) {
            assert_eq!(r["kind"], "market.candles.read");
            assert_eq!(r["timeframe"], *tf);
            assert_eq!(r["symbol"], "BTC-USDT");
        }
        let mut state = out["result"]["state"].clone();
        for (tf, span) in TIMEFRAMES {
            let end = now - now % span - span;
            let quote = json!({"symbol":"BTC-USDT", "timeframe":tf,
                "candles":[[end,100.,101.,99.,100.]], "history_end_ms":end,
                "current_price":100., "current_candle":[end+span,100.,101.,99.,100.],
                "batch_complete":false});
            let partial = invoke_at("market", state, quote.clone(), now);
            assert_eq!(partial["status"], "executed", "{partial}");
            state = partial["result"]["state"].clone();
            assert!(!partial["result"]["view"]["message"]
                .as_str()
                .unwrap()
                .contains("unchanged and untouched"));
            assert!(partial["result"]["view"]["rows"]
                .as_array()
                .unwrap()
                .is_empty());
            // A restart during a read must retain the plan, not mark partial evidence current.
            let reopen = invoke_at("open", state.clone(), Value::Null, now);
            assert_eq!(reopen["result"]["state"], state);
            let mut complete_quote = quote;
            complete_quote["batch_complete"] = json!(true);
            let complete = invoke_at("market", state, complete_quote, now);
            assert_eq!(complete["status"], "executed", "{complete}");
            state = complete["result"]["state"].clone();
            assert_eq!(state["entry"]["plan"], plan);
            assert!(complete["result"]["view"]["rows"]
                .as_array()
                .unwrap()
                .is_empty());
            assert!(complete["result"]["requests"]
                .as_array()
                .unwrap()
                .is_empty());
        }
        let presented = invoke_at("present", state.clone(), Value::Null, now);
        assert_eq!(presented["result"]["state"], state);
        assert!(presented["result"]["requests"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(presented["result"]["resume_action"].is_null());
        let reopen = invoke_at("open", state.clone(), Value::Null, now);
        assert_eq!(reopen["result"]["state"], state);
        assert!(reopen["result"]["view"]["message"]
            .as_str()
            .unwrap()
            .contains("unchanged and untouched"));
        assert_eq!(
            reopen["result"]["view"]["rows"].as_array().unwrap().len(),
            1
        );
        let stale = invoke_at("open", state.clone(), Value::Null, now + 60_001);
        assert!(!stale["result"]["view"]["message"]
            .as_str()
            .unwrap()
            .contains("unchanged and untouched"));
        state["settings"]["entry_margin"] = json!(2.);
        let mut raw = json!({"schema_version":1, "plugin_id":PLUGIN_ID, "host_method":"workspace",
            "action":"refresh_order", "observed_at_ms":now, "state":state});
        raw["settings"] = json!(settings());
        let changed: Value =
            serde_json::from_slice(&evaluate_json(&serde_json::to_vec(&raw).unwrap())).unwrap();
        assert_eq!(changed["status"], "rejected");
    }

    #[test]
    fn pending_line_requires_live_completed_history_and_rejects_inconsistent_saved_observation() {
        let now = 1_800_000_123_456;
        let state = managed_pending_state(now);
        let span = duration("5m").unwrap();
        let end = now - now % span - span;
        for (history_end, current) in [
            (end, Value::Null),
            (end + span, json!([end + span, 100., 101., 99., 100.])),
        ] {
            let out = invoke_at(
                "market",
                serde_json::to_value(&state).unwrap(),
                json!({
                    "symbol":"BTC-USDT", "timeframe":"5m", "candles":[[end,100.,101.,99.,100.]],
                    "current_price":100., "history_end_ms":history_end, "batch_complete":true,
                    "current_candle":current
                }),
                now,
            );
            if out["status"] == "executed" {
                assert!(out["result"]["state"]["frames"][5]["checked_at_ms"].is_null());
                assert!(!out["result"]["view"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("unchanged and untouched"));
            } else {
                assert_eq!(out["status"], "rejected");
            }
        }
        let mut invalid = serde_json::to_value(state).unwrap();
        invalid["frames"][0]["checked_at_ms"] = json!(now + 1);
        assert_eq!(
            invoke_at("open", invalid, Value::Null, now)["status"],
            "rejected"
        );
    }

    #[test]
    fn only_confirmed_non_dispatch_allows_explicit_retry_of_the_same_plan() {
        let prepared = invoke(
            "prepare",
            serde_json::to_value(entry_state()).unwrap(),
            Value::Null,
        );
        let placed = invoke(
            "place_entry",
            prepared["result"]["state"].clone(),
            Value::Null,
        );
        let state = placed["result"]["state"].clone();
        let requested = invoke("validate_entry", state.clone(), Value::Null);
        assert_eq!(requested["status"], "executed", "{requested}");
        assert_eq!(requested["result"]["state"], state);
        assert_eq!(
            requested["result"]["requests"],
            json!([
                {"kind":"market.candles.read", "provider":"bingx",
                    "symbol":"BTC-USDT", "timeframe":"5m", "limit":600}
            ])
        );
        let evidence = json!({"account_id":"a".repeat(64),"symbol":"BTC-USDT",
            "client_order_id":"c".repeat(40),"order_id":null,"status":"not_sent",
            "filled_quantity":0.,"average_price":0.,"observed_at_ms":100_000_000});
        let stopped = invoke("order", state.clone(), evidence.clone());
        assert_eq!(stopped["status"], "executed", "{stopped}");
        let reopened = invoke("open", stopped["result"]["state"].clone(), Value::Null);
        assert!(reopened["result"]["requests"]
            .as_array()
            .unwrap()
            .is_empty());
        let retry = invoke(
            "place_entry",
            reopened["result"]["state"].clone(),
            Value::Null,
        );
        assert_eq!(retry["status"], "executed", "{retry}");
        assert_eq!(
            retry["result"]["requests"][0],
            placed["result"]["requests"][0]
        );
        assert!(retry["result"]["state"]["entry"]["evidence"].is_null());
        let mut unknown = evidence.clone();
        unknown["status"] = json!("unknown");
        let unresolved = invoke("order", state.clone(), unknown);
        assert_eq!(
            invoke(
                "place_entry",
                unresolved["result"]["state"].clone(),
                Value::Null
            )["status"],
            "rejected"
        );
        let mut invalid = evidence;
        invalid["order_id"] = json!("123");
        assert_eq!(invoke("order", state, invalid)["status"], "rejected");
    }

    #[test]
    fn fresh_preparation_replaces_only_a_confirmed_unsent_entry() {
        let prepared = invoke(
            "prepare",
            serde_json::to_value(entry_state()).unwrap(),
            Value::Null,
        );
        let placed = invoke(
            "place_entry",
            prepared["result"]["state"].clone(),
            Value::Null,
        );
        for status in [
            "not_sent",
            "unknown",
            "open",
            "partial",
            "filled",
            "cancelled",
            "rejected",
            "expired",
        ] {
            let dispatched = !["not_sent", "unknown"].contains(&status);
            let quantity = placed["result"]["state"]["entry"]["plan"]["quantity"]
                .as_f64()
                .unwrap();
            let filled = match status {
                "filled" => quantity,
                "partial" => quantity / 2.0,
                _ => 0.0,
            };
            let evidence = json!({"account_id":"a".repeat(64),"symbol":"BTC-USDT",
                "client_order_id":"c".repeat(40),"order_id":if dispatched {Some("123")} else {None},
                "status":status,"filled_quantity":filled,"average_price":if filled > 0.0 {96.5} else {0.0},"observed_at_ms":100_000_000});
            let stopped = invoke("order", placed["result"]["state"].clone(), evidence);
            assert_eq!(stopped["status"], "executed", "{stopped}");
            let state = stopped["result"]["state"].clone();
            let actions = stopped["result"]["view"]["actions"].as_array().unwrap();
            assert_eq!(
                actions.iter().any(|a| a["id"] == "prepare_entry"),
                status == "not_sent"
            );
            assert!(stopped["result"]["view"]["confirmation"].is_null());
            let raw = json!({"schema_version":1,"plugin_id":PLUGIN_ID,"host_method":"workspace",
                "action":"prepare_entry","settings":settings(),"state":state,"observed_at_ms":100_000_001});
            let refreshed: Value =
                serde_json::from_slice(&evaluate_json(&serde_json::to_vec(&raw).unwrap())).unwrap();
            if status != "not_sent" {
                assert!(
                    refreshed["result"]["state"]["entry"]["attempted"] == true
                        || refreshed["status"] == "rejected"
                );
                assert_eq!(invoke("prepare", state, Value::Null)["status"], "rejected");
                continue;
            }
            assert_eq!(refreshed["status"], "executed", "{refreshed}");
            assert_eq!(refreshed["result"]["resume_action"], "prepare");
            assert!(refreshed["result"]["state"]["entry"].is_null());
            let requests = refreshed["result"]["requests"].as_array().unwrap();
            assert_eq!(requests.len(), 7);
            assert!(requests.iter().all(|r| r["kind"] != "order.entry.place"));
            let reopened = invoke("open", refreshed["result"]["state"].clone(), Value::Null);
            assert!(reopened["result"].get("resume_action").is_none());
            assert!(reopened["result"]["state"]["entry"].is_null());
            assert_eq!(
                invoke("prepare", reopened["result"]["state"].clone(), Value::Null)["status"],
                "rejected"
            );
            // Six fresh market frames and a fresh account snapshot are needed
            // before the existing prepare action can admit a replacement.
            let mut fresh = reopened["result"]["state"].clone();
            fresh["complete"] = json!(true);
            let replacement = invoke("prepare", fresh, Value::Null);
            assert_eq!(replacement["status"], "executed", "{replacement}");
            assert!(replacement["result"].get("resume_action").is_none());
            assert_eq!(replacement["result"]["state"]["entry"]["attempted"], false);
            assert!(replacement["result"]["requests"]
                .as_array()
                .unwrap()
                .is_empty());
        }
    }

    #[test]
    fn final_quote_is_checked_by_wasm_before_delivery() {
        let prepared = invoke(
            "prepare",
            serde_json::to_value(entry_state()).unwrap(),
            Value::Null,
        );
        let placed = invoke(
            "place_entry",
            prepared["result"]["state"].clone(),
            Value::Null,
        );
        let state = placed["result"]["state"].clone();
        let quote = json!({"symbol":"BTC-USDT","timeframe":"5m","candles":[[99_600_000,100.,101.,99.,100.]],
            "history_start_ms":99_600_000,"history_end_ms":99_600_000,"batch_complete":true,
            "current_price":100.,"current_candle":[99_900_000,100.,101.,99.,100.]});
        let good = invoke("validate_entry", state.clone(), quote.clone());
        assert_eq!(good["status"], "executed", "{good}");
        assert_eq!(good["result"]["state"], state);
        assert!(good["result"]["requests"].as_array().unwrap().is_empty());
        let mut swept = quote;
        swept["current_candle"][3] = json!(96.0);
        assert_eq!(invoke("validate_entry", state, swept)["status"], "rejected");
    }

    #[test]
    fn user_waiting_preserves_the_plan_but_requires_complete_unswept_history() {
        let prepared = invoke(
            "prepare",
            serde_json::to_value(entry_state()).unwrap(),
            Value::Null,
        );
        let plan = prepared["result"]["state"]["entry"]["plan"].clone();
        let now = 100_900_000;
        let call = |action: &str, state: Value, snapshot: Value| -> Value {
            let raw = json!({"schema_version":1,"plugin_id":PLUGIN_ID,"host_method":"workspace",
                "action":action,"state":state,"snapshot":snapshot,"observed_at_ms":now});
            serde_json::from_slice(&evaluate_json(&serde_json::to_vec(&raw).unwrap())).unwrap()
        };
        let placed = call(
            "place_entry",
            prepared["result"]["state"].clone(),
            Value::Null,
        );
        assert_eq!(placed["status"], "executed", "{placed}");
        assert_eq!(placed["result"]["requests"][0]["plan"], plan);
        let state = placed["result"]["state"].clone();
        let mut quote = json!({"symbol":"BTC-USDT","timeframe":"5m",
            "history_start_ms":99_900_000,"history_end_ms":100_500_000,"batch_complete":true,
            "candles":(99_900_000..100_800_000).step_by(300_000)
                .map(|t| json!([t,100.,101.,99.,100.])).collect::<Vec<_>>(),
            "current_price":100.,"current_candle":[100_800_000,100.,101.,99.,100.]});
        let good = call("validate_entry", state.clone(), quote.clone());
        assert_eq!(good["status"], "executed", "{good}");
        assert_eq!(good["result"]["state"], state);
        let mut incomplete = quote.clone();
        incomplete["candles"].as_array_mut().unwrap().remove(0);
        assert_eq!(
            call("validate_entry", state.clone(), incomplete)["status"],
            "rejected"
        );
        let mut gap = quote.clone();
        gap["candles"].as_array_mut().unwrap().remove(1);
        assert_eq!(
            call("validate_entry", state.clone(), gap)["status"],
            "rejected"
        );
        let mut first = quote.clone();
        first["candles"] = json!([quote["candles"][0], quote["candles"][1]]);
        first["batch_complete"] = json!(false);
        let progress = call("validate_entry", state.clone(), first);
        assert_eq!(progress["status"], "executed", "{progress}");
        let mut last = quote.clone();
        last["candles"] = json!([quote["candles"][2]]);
        assert_eq!(
            call("validate_entry", state.clone(), last.clone())["status"],
            "rejected"
        );
        let finished = call("validate_entry", progress["result"]["state"].clone(), last);
        assert_eq!(finished["status"], "executed", "{finished}");
        assert_eq!(finished["result"]["state"], state);
        quote["candles"][1][3] = json!(96.0);
        assert_eq!(call("validate_entry", state, quote)["status"], "rejected");
    }

    #[test]
    fn fractional_plan_remains_exact_across_json_reopen_and_confirmation() {
        let prepared = invoke(
            "prepare",
            serde_json::to_value(entry_state()).unwrap(),
            Value::Null,
        );
        let mut state = prepared["result"]["state"].clone();
        state["entry"]["plan"]["margin"] = json!(0.9999106133333333);
        let plan = state["entry"]["plan"].clone();
        for _ in 0..10 {
            state = invoke("open", state, Value::Null)["result"]["state"].clone();
            assert_eq!(state["entry"]["plan"], plan);
        }
        let placed = invoke("place_entry", state, Value::Null);
        assert_eq!(placed["status"], "executed", "{placed}");
        assert_eq!(placed["result"]["requests"][0]["plan"], plan);
    }

    #[test]
    fn entry_never_uses_touched_stale_or_changed_inputs() {
        let mut state = entry_state();
        state.frames[0].sell[0].touched = true;
        assert_eq!(
            invoke(
                "prepare",
                serde_json::to_value(&state).unwrap(),
                Value::Null
            )["status"],
            "rejected"
        );
        state = entry_state();
        state.observed_at = Some(99_000_000);
        assert_eq!(
            invoke(
                "prepare",
                serde_json::to_value(&state).unwrap(),
                Value::Null
            )["status"],
            "rejected"
        );
        let prepared = invoke(
            "prepare",
            serde_json::to_value(entry_state()).unwrap(),
            Value::Null,
        );
        let raw = json!({"schema_version":1,"plugin_id":PLUGIN_ID,"host_method":"workspace",
            "action":"place_entry","state":prepared["result"]["state"],"observed_at_ms":100_000_001 + ENTRY_HISTORY_WINDOW});
        let out: Value =
            serde_json::from_slice(&evaluate_json(&serde_json::to_vec(&raw).unwrap())).unwrap();
        assert_eq!(out["status"], "rejected");
        let mut raw = raw;
        raw["observed_at_ms"] = json!(100_000_000);
        let mut settings = settings();
        settings.entry_margin = 2.0;
        raw["settings"] = serde_json::to_value(settings).unwrap();
        let out: Value =
            serde_json::from_slice(&evaluate_json(&serde_json::to_vec(&raw).unwrap())).unwrap();
        assert_eq!(out["status"], "rejected");
    }

    #[test]
    fn live_candle_sweep_marks_a_known_line_without_backdating_formation() {
        let mut state = entry_state();
        state.frames[0].sell[0].first_known = 99_900_000;
        let out = invoke(
            "market",
            serde_json::to_value(state).unwrap(),
            json!({
            "symbol":"BTC-USDT","timeframe":"5m","candles":[[99_600_000,100.,101.,99.,100.]],
            "current_price":100.,"current_candle":[99_900_000,100.,101.,96.,100.]}),
        );
        assert_eq!(out["status"], "executed", "{out}");
        assert_eq!(
            out["result"]["state"]["frames"][0]["sell"][0]["touched"],
            true
        );
        let mut frame = Frame {
            buy: vec![Level {
                origin: 10,
                first_known: 600_000,
                price: 105.,
                top: 106.,
                bottom: 104.,
                touched: false,
                consumed: false,
            }],
            ..Frame::default()
        };
        frame.process(
            Candle(300_000, 100., 106., 99., 100.),
            &settings(),
            300_000,
            false,
        );
        assert!(!frame.buy[0].touched);
        frame.process(
            Candle(600_000, 100., 106., 99., 100.),
            &settings(),
            300_000,
            false,
        );
        assert!(frame.buy[0].touched);
    }

    #[test]
    fn provider_observation_is_distinct_from_entry_acceptance_and_survives_restart() {
        let prepared = invoke(
            "prepare",
            serde_json::to_value(entry_state()).unwrap(),
            Value::Null,
        );
        let placed = invoke(
            "place_entry",
            prepared["result"]["state"].clone(),
            Value::Null,
        );
        let state = placed["result"]["state"].clone();
        let mut evidence = json!({"account_id":"a".repeat(64),"symbol":"BTC-USDT",
            "client_order_id":"c".repeat(40),"order_id":"2103610529511862272","status":"partial",
            "filled_quantity":0.1,"average_price":96.4,"observed_at_ms":100_000_000});
        let partial = invoke("order", state.clone(), evidence.clone());
        assert_eq!(partial["status"], "executed", "{partial}");
        assert!(partial["result"]["view"]["message"]
            .as_str()
            .unwrap()
            .contains("not verified"));
        assert_eq!(
            invoke("open", partial["result"]["state"].clone(), Value::Null)["result"]["state"],
            partial["result"]["state"]
        );
        evidence["filled_quantity"] = json!(0.0);
        assert_eq!(
            invoke(
                "order",
                partial["result"]["state"].clone(),
                evidence.clone()
            )["status"],
            "rejected"
        );
        evidence["filled_quantity"] = json!(0.1);
        evidence["account_id"] = json!("b".repeat(64));
        assert_eq!(invoke("order", state, evidence)["status"], "rejected");
        let restored = invoke(
            "restore_entry",
            serde_json::to_value(entry_state()).unwrap(),
            json!({"account_id":"a".repeat(64), "batch_complete":true, "operations":[{
                "account_binding_id":"a".repeat(64), "provider_id":"bingx", "effect_kind":"order.entry.place",
                "state":"succeeded", "canonical_payload_json":serde_json::to_string(&prepared["result"]["state"]["entry"]["plan"]).unwrap()
            }]}),
        );
        assert_eq!(restored["result"]["state"]["entry"]["attempted"], true);
        assert!(restored["result"]["requests"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn recovery_selects_durable_entries_across_batches_without_inventing_observations() {
        let prepared = invoke(
            "prepare",
            serde_json::to_value(entry_state()).unwrap(),
            Value::Null,
        );
        let plan = prepared["result"]["state"]["entry"]["plan"].clone();
        let mut state = serde_json::to_value(entry_state()).unwrap();
        let opened = invoke("open", state.clone(), Value::Null);
        assert_eq!(opened["result"]["requests"][0]["scope"], "durable");
        let operation = json!({
            "account_binding_id":"a".repeat(64), "provider_id":"bingx",
            "effect_kind":"order.entry.place", "state":"succeeded",
            "canonical_payload_json":serde_json::to_string(&plan).unwrap(),
            "receipt":{"provider_receipt_id":"123"}
        });
        for complete in [false, true] {
            let mut not_sent = operation.clone();
            not_sent["state"] = json!("terminal_failure");
            not_sent["last_error_code"] = json!("entry_not_sent");
            not_sent["receipt"] = Value::Null;
            not_sent["canonical_payload_json"] = json!("not an entry plan");
            let mut rejected = not_sent.clone();
            rejected["last_error_code"] = json!("provider_rejected");
            let mut other_effect = operation.clone();
            other_effect["effect_kind"] = json!("another.effect");
            let out = invoke(
                "restore_entry",
                state,
                json!({
                    "account_id":"a".repeat(64),"batch_complete":complete,
                    "operations":[operation,not_sent,rejected,other_effect]
                }),
            );
            assert_eq!(out["status"], "executed", "{out}");
            state = out["result"]["state"].clone();
            assert_eq!(state["entry"]["plan"], plan);
            assert_eq!(state["entry"]["attempted"], true);
            assert!(state["entry"]["evidence"].is_null());
            assert!(out["result"]["requests"].as_array().unwrap().is_empty());
        }
        let reopened = invoke("open", state.clone(), Value::Null);
        assert_eq!(reopened["result"]["state"], state);
        assert!(reopened["result"]["requests"]
            .as_array()
            .unwrap()
            .is_empty());
        for fault in ["account", "provider", "payload", "size", "incomplete"] {
            let mut bad_op = operation.clone();
            let mut snapshot = json!({"account_id":"a".repeat(64),"batch_complete":true});
            match fault {
                "account" => bad_op["account_binding_id"] = json!("b".repeat(64)),
                "provider" => bad_op["provider_id"] = json!("foreign"),
                "payload" => bad_op["canonical_payload_json"] = json!("{}"),
                "incomplete" => snapshot["batch_complete"] = Value::Null,
                _ => (),
            }
            snapshot["operations"] = json!(vec![bad_op; if fault == "size" { 5 } else { 1 }]);
            assert_eq!(
                invoke("restore_entry", state.clone(), snapshot)["status"],
                "rejected",
                "{fault}"
            );
        }
    }

    #[test]
    fn package_renders_and_retains_confirmed_non_dispatch_reason() {
        let prepared = invoke(
            "prepare",
            serde_json::to_value(entry_state()).unwrap(),
            Value::Null,
        );
        let placed = invoke(
            "place_entry",
            prepared["result"]["state"].clone(),
            Value::Null,
        );
        let reason = "Final evidence rejected: entry line was already touched";
        let out = invoke(
            "order",
            placed["result"]["state"].clone(),
            json!({
                "account_id":"a".repeat(64),"symbol":"BTC-USDT",
                "client_order_id":"c".repeat(40),"order_id":null,"status":"not_sent",
                "filled_quantity":0.,"average_price":0.,"observed_at_ms":100_000_000,
                "error_message":reason
            }),
        );
        assert_eq!(out["status"], "executed", "{out}");
        assert!(out["result"]["view"]["message"]
            .as_str()
            .unwrap()
            .contains(reason));
        let reopened = invoke("open", out["result"]["state"].clone(), Value::Null);
        assert_eq!(reopened["result"]["state"], out["result"]["state"]);
        assert!(reopened["result"]["view"]["message"]
            .as_str()
            .unwrap()
            .contains(reason));
    }

    #[test]
    fn preview_uses_side_leverage_and_stop_as_percentage_of_margin() {
        let mut state = State::new(settings());
        state.account = Some(account());
        let long = preview(&state, "long", 100.0, 100_000_000).unwrap();
        assert!(long.contains("long 0.5 quantity at 100; 50x"), "{long}");
        assert!(
            long.contains("estimated margin 1.000000 USDT, notional 50.000000 USDT, stop 99.6"),
            "{long}"
        );
        let short = preview(&state, "short", 100.0, 100_000_000).unwrap();
        assert!(short.contains("short 0.2 quantity at 100; 20x"), "{short}");
        assert!(
            short.contains("notional 20.000000 USDT, stop 101"),
            "{short}"
        );
        state.settings.stop_percent = 0.1;
        assert!(preview(&state, "long", 100.0, 100_000_000).is_err());
    }

    #[test]
    fn preview_never_increases_budget_and_does_not_use_stale_or_other_symbol_evidence() {
        let mut state = State::new(settings());
        state.account = Some(account());
        let rounded = preview(&state, "long", 100.019, 100_000_000).unwrap();
        assert!(
            rounded.contains("long 0.499 quantity at 100.01"),
            "{rounded}"
        );
        assert!(rounded.contains("estimated margin 0.998100"), "{rounded}");
        assert!(preview(&state, "long", 100.0, 100_060_001).is_err());
        assert!(preview(&state, "long", 100.0, 99_999_999).is_err());
        state.settings.symbol = "XRP-USDT".into();
        assert!(preview(&state, "long", 100.0, 100_000_000).is_err());
        state.settings.symbol = "BTC-USDT".into();
        state.settings.entry_margin = 0.01;
        assert!(preview(&state, "long", 100.0, 100_000_000)
            .unwrap_err()
            .contains("not increased"));
        state.settings.entry_margin = 31.0;
        assert!(preview(&state, "long", 100.0, 100_000_000)
            .unwrap_err()
            .contains("available"));
    }

    #[test]
    fn budget_changes_preserve_zone_history_and_existing_observer_state_loads() {
        let mut state = State::new(settings());
        let mut frame = Frame {
            timeframe: "5m".into(),
            ..Frame::default()
        };
        for bar in repeated_swings() {
            frame.process(bar, &settings(), 300_000, true);
        }
        state.frames.push(frame);
        state.account = Some(account());
        let old_frames = serde_json::to_value(&state.frames).unwrap();
        let mut next = state.settings.clone();
        next.entry_margin = 2.0;
        next.stop_percent = 10.0;
        let input = json!({"schema_version":1,"plugin_id":PLUGIN_ID,"host_method":"workspace",
            "action":"inspect","state":state,"settings":next,"observed_at_ms":100_000_000});
        let out: Value =
            serde_json::from_slice(&evaluate_json(&serde_json::to_vec(&input).unwrap())).unwrap();
        assert_eq!(out["status"], "executed");
        assert_eq!(out["result"]["state"]["frames"], old_frames);
        assert_eq!(out["result"]["requests"].as_array().unwrap().len(), 7);
        assert_eq!(
            out["result"]["requests"][6]["kind"],
            "account.snapshot.read"
        );
        let mut old = serde_json::to_value(State::new(settings())).unwrap();
        old.as_object_mut().unwrap().remove("account");
        old["settings"]
            .as_object_mut()
            .unwrap()
            .remove("entry_margin");
        old["settings"]
            .as_object_mut()
            .unwrap()
            .remove("stop_percent");
        assert_eq!(invoke("open", old, Value::Null)["status"], "executed");
    }

    #[test]
    fn account_admission_rejects_wrong_instrument_endpoint_and_secret_fields() {
        let state = serde_json::to_value(State::new(settings())).unwrap();
        let good = serde_json::to_value(account()).unwrap();
        assert_eq!(
            invoke("account", state.clone(), good.clone())["status"],
            "executed"
        );
        for (field, value) in [
            ("symbol", json!("XRP-USDT")),
            ("endpoint", json!("TEST")),
            ("observed_at_ms", json!(99_900_000)),
            ("long_leverage", json!(0)),
            ("api_key", json!("secret")),
        ] {
            let mut bad = good.clone();
            bad[field] = value;
            assert_eq!(
                invoke("account", state.clone(), bad)["status"],
                "rejected",
                "{field}"
            );
        }
    }

    fn repeated_swings() -> Vec<Candle> {
        // Three separated equal highs and lows, confirmed one bar later.
        let prices = [
            100., 101., 102., 103., 105., 102., 101., 99., 97., 100., 101., 103., 105., 102., 101.,
            99., 97., 100., 101., 103., 105., 102., 101., 99., 97., 100.,
        ];
        prices
            .iter()
            .enumerate()
            .map(|(i, p)| Candle(i as i64 * 300_000, *p, *p + 0.5, *p - 0.5, *p))
            .collect()
    }

    #[test]
    fn three_alternating_swings_form_the_pine_line_at_confirmation() {
        let bars = repeated_swings();
        let mut frame = Frame::default();
        for bar in &bars[..21] {
            frame.process(bar.clone(), &settings(), 300_000, true);
        }
        assert!(frame.buy.is_empty());
        frame.process(bars[21].clone(), &settings(), 300_000, true);
        assert_eq!(frame.buy.len(), 1);
        assert_eq!(frame.buy[0].price, 105.5);
        assert_eq!(frame.buy[0].origin, 4 * 300_000);
        assert_eq!(frame.buy[0].first_known, 22 * 300_000);
        assert!(!frame.buy[0].touched);
        for bar in &bars[22..] {
            frame.process(bar.clone(), &settings(), 300_000, true);
        }
        assert_eq!(frame.sell[0].price, 96.5);
    }

    #[test]
    fn cluster_line_uses_last_match_not_center_or_latest_pivot() {
        let mut frame = Frame {
            swings: vec![
                Swing {
                    direction: 1,
                    time: 30,
                    price: 106.,
                },
                Swing {
                    direction: -1,
                    time: 20,
                    price: 90.,
                },
                Swing {
                    direction: 1,
                    time: 10,
                    price: 105.,
                },
                Swing {
                    direction: 1,
                    time: 5,
                    price: 104.,
                },
            ],
            ..Frame::default()
        };
        frame.cluster(1, 106., 3., 40);
        assert_eq!(frame.buy[0].price, 104.);
        assert_eq!(frame.buy[0].top, 108.);
        assert_eq!(frame.buy[0].bottom, 102.);
    }

    #[test]
    fn restart_state_preserves_swings_atr_and_known_times() {
        let bars = repeated_swings();
        let mut full = Frame::default();
        let mut resumed = Frame::default();
        for bar in &bars {
            full.process(bar.clone(), &settings(), 300_000, true);
        }
        for bar in &bars[..18] {
            resumed.process(bar.clone(), &settings(), 300_000, true);
        }
        resumed = serde_json::from_slice(&serde_json::to_vec(&resumed).unwrap()).unwrap();
        for bar in &bars[18..] {
            resumed.process(bar.clone(), &settings(), 300_000, true);
        }
        assert_eq!(
            serde_json::to_value(full).unwrap(),
            serde_json::to_value(resumed).unwrap()
        );
    }

    fn invoke(action: &str, state: Value, snapshot: Value) -> Value {
        invoke_at(action, state, snapshot, 100_000_000)
    }

    fn invoke_at(action: &str, state: Value, snapshot: Value, now: i64) -> Value {
        let raw = json!({"schema_version":1,"plugin_id":PLUGIN_ID,"host_method":"workspace",
            "action":action,"state":state,"snapshot":snapshot,"observed_at_ms":now});
        serde_json::from_slice(&evaluate_json(&serde_json::to_vec(&raw).unwrap())).unwrap()
    }

    #[test]
    fn open_orders_are_transient_observations_not_managed_entries() {
        let state = serde_json::to_value(entry_state()).unwrap();
        let request = invoke("list_orders", state.clone(), Value::Null);
        assert_eq!(request["status"], "executed");
        assert_eq!(request["result"]["state"], state);
        assert_eq!(
            request["result"]["requests"],
            json!([{
                "kind":"order.snapshot.read", "scope":"open", "provider":"bingx",
                "account_id":"a".repeat(64), "symbol":"BTC-USDT"
            }])
        );
        let snapshot = json!({"account_id":"a".repeat(64), "symbol":"BTC-USDT",
        "observed_at_ms":100_000_000, "orders":[{
            "order_id":"2103610529511862272", "side":"BUY", "position_side":"LONG",
                "type":"LIMIT", "status":"PARTIALLY_FILLED", "price":"96.5",
                "stop_price":"0", "quantity":"0.518", "filled_quantity":"0.1"
        }]});
        let listed = invoke("open_orders", state.clone(), snapshot.clone());
        assert_eq!(listed["status"], "executed");
        assert_eq!(listed["result"]["state"], state);
        assert_eq!(listed["result"]["requests"], json!([]));
        assert_eq!(listed["result"]["view"]["details_title"], "Open orders");
        assert_eq!(
            listed["result"]["view"]["rows"][0][0],
            "2103610529511862272"
        );
        assert_eq!(listed["result"]["view"]["rows"][0][3], "PARTIALLY_FILLED");
        let reopened = invoke("open", listed["result"]["state"].clone(), Value::Null);
        assert_eq!(reopened["result"]["state"], state);
        assert!(reopened["result"]["view"]["details_title"].is_null());
        assert_eq!(
            reopened["result"]["requests"],
            json!([
                {"kind":"order.snapshot.read", "scope":"durable", "provider":"bingx", "account_id":"a".repeat(64)}
            ])
        );
        let mut full = snapshot.clone();
        full["orders"] = json!((0..64)
            .map(|i| {
                let mut row = snapshot["orders"][0].clone();
                row["order_id"] = json!((2103610529511862272_u64 + i).to_string());
                row
            })
            .collect::<Vec<_>>());
        let bounded = invoke("open_orders", state.clone(), full);
        assert_eq!(bounded["status"], "executed");
        assert_eq!(bounded["result"]["state"], state);
        assert_eq!(
            bounded["result"]["view"]["rows"].as_array().unwrap().len(),
            64
        );
        assert!(
            serde_json::to_vec(&bounded["result"]["view"])
                .unwrap()
                .len()
                <= 16 * 1024
        );
        assert!(bounded["result"]["view"]["confirmation"].is_null());
        // Construct an explicitly empty observation, not a missing/failed read.
        let mut empty_snapshot = snapshot.clone();
        empty_snapshot["orders"] = json!([]);
        let empty = invoke("open_orders", state, empty_snapshot);
        assert_eq!(empty["status"], "executed");
        assert_eq!(empty["result"]["view"]["rows"], json!([]));
    }

    #[test]
    fn open_orders_reject_scope_age_duplicates_and_malformed_rows() {
        let state = serde_json::to_value(entry_state()).unwrap();
        let snapshot = json!({"account_id":"a".repeat(64), "symbol":"BTC-USDT",
        "observed_at_ms":100_000_000, "orders":[{
            "order_id":"2103610529511862272", "side":"BUY", "position_side":"LONG",
                "type":"LIMIT", "status":"NEW", "price":"96.5",
                "stop_price":"0", "quantity":"0.518", "filled_quantity":"0"
        }]});
        for kind in [
            "account",
            "symbol",
            "future",
            "stale",
            "duplicate",
            "quantity",
            "extra",
        ] {
            let mut bad = snapshot.clone();
            match kind {
                "account" => bad["account_id"] = json!("b".repeat(64)),
                "symbol" => bad["symbol"] = json!("VET-USDT"),
                "future" => bad["observed_at_ms"] = json!(100_000_001),
                "stale" => bad["observed_at_ms"] = json!(99_939_999),
                "duplicate" => bad["orders"] = json!([bad["orders"][0], bad["orders"][0]]),
                "quantity" => bad["orders"][0]["filled_quantity"] = json!("1"),
                _ => bad["orders"][0]["secret_key"] = json!("not allowed"),
            }
            assert_eq!(
                invoke("open_orders", state.clone(), bad)["status"],
                "rejected",
                "{kind}"
            );
        }
        assert_eq!(
            invoke("list_orders", Value::Null, Value::Null)["status"],
            "rejected"
        );
    }

    #[test]
    fn wrong_symbol_unclosed_gap_and_mutated_candles_are_rejected() {
        let bars = repeated_swings();
        let snapshot =
            json!({"symbol":"BTC-USDT","timeframe":"5m","candles":bars,"current_price":100.});
        let accepted = invoke("market", Value::Null, snapshot.clone());
        assert_eq!(accepted["status"], "executed");
        let state = accepted["result"]["state"].clone();
        for kind in ["symbol", "future", "gap", "mutation"] {
            let mut bad = snapshot.clone();
            match kind {
                "symbol" => bad["symbol"] = json!("XRP-USDT"),
                "future" => bad["candles"][25][0] = json!(100_200_000),
                "gap" => {
                    bad["candles"].as_array_mut().unwrap().remove(12);
                }
                _ => bad["candles"][25][4] = json!(100.1),
            }
            assert_eq!(
                invoke("market", state.clone(), bad)["status"],
                "rejected",
                "{kind}"
            );
        }
        let replay = invoke("market", state.clone(), snapshot);
        assert_eq!(replay["result"]["state"], state);
    }

    #[test]
    fn touched_levels_stay_visible_and_cannot_be_entry_candidates() {
        let mut state = State::new(settings());
        let mut frame = Frame {
            timeframe: "5m".into(),
            ..Frame::default()
        };
        for bar in repeated_swings() {
            frame.process(bar, &settings(), 300_000, true);
        }
        state.frames.push(frame);
        state.current_price = Some(100.);
        state.complete = true;
        assert!(view(&state, 100_000_000)["message"]
            .as_str()
            .unwrap()
            .contains("Entry candidate"));
        for frame in &mut state.frames {
            for level in frame.buy.iter_mut().chain(&mut frame.sell) {
                level.touched = true;
            }
        }
        let result = view(&state, 100_000_000);
        assert!(!result["rows"].as_array().unwrap().is_empty());
        assert!(result["message"]
            .as_str()
            .unwrap()
            .contains("No untouched entry"));
    }

    #[test]
    fn inspect_requests_only_selected_symbol_and_six_timeframes() {
        let raw = json!({"schema_version":1,"plugin_id":PLUGIN_ID,"host_method":"workspace",
            "action":"inspect","state":null,"observed_at_ms":100_000_000,
            "settings":{"symbol":"XRP-USDT","detection_length":7,"margin":6.9}});
        let first = evaluate_json(&serde_json::to_vec(&raw).unwrap());
        assert_eq!(first, evaluate_json(&serde_json::to_vec(&raw).unwrap()));
        let out: Value = serde_json::from_slice(&first).unwrap();
        assert_eq!(out["result"]["requests"].as_array().unwrap().len(), 6);
        let field = &out["result"]["view"]["fields"][0];
        assert_eq!(field["type"], "choice");
        assert_eq!(field["value"], "XRP-USDT");
        assert_eq!(field["source"]["kind"], "market.instruments.read");
        let mut long = raw.clone();
        long["settings"]["symbol"] = json!("NCSIRUSSELL20002USD-USDT");
        let long_result: Value =
            serde_json::from_slice(&evaluate_json(&serde_json::to_vec(&long).unwrap())).unwrap();
        assert_eq!(long_result["status"], "executed");
        long["settings"]["symbol"] = json!(format!("{}-USDT", "A".repeat(61)));
        let invalid: Value =
            serde_json::from_slice(&evaluate_json(&serde_json::to_vec(&long).unwrap())).unwrap();
        assert_ne!(invalid["status"], "executed");
        for request in out["result"]["requests"].as_array().unwrap() {
            assert_eq!(request["symbol"], "XRP-USDT");
            assert_eq!(request["kind"], "market.candles.read");
        }
    }

    #[test]
    fn streaming_and_overlapping_reinspection_preserve_exact_state() {
        let bars: Vec<_> = (0..180)
            .map(|i| {
                let price = 100. + 5. * (i as f64 * std::f64::consts::PI / 12.).sin();
                Candle(i * 300_000, price, price + 0.5, price - 0.5, price)
            })
            .collect();
        let full = invoke(
            "market",
            Value::Null,
            json!({"symbol":"BTC-USDT","timeframe":"5m","candles":bars,"current_price":100.}),
        );
        assert_eq!(full["status"], "executed");
        let expected = full["result"]["state"].clone();
        let mut state = Value::Null;
        for batch_size in [12, 24, 48] {
            state = Value::Null;
            for _ in 0..2 {
                for (index, batch) in bars.chunks(batch_size).enumerate() {
                    let out = invoke(
                        "market",
                        state,
                        json!({"symbol":"BTC-USDT","timeframe":"5m","candles":batch,
                            "current_price":100.,"history_end_ms":bars.last().unwrap().0,
                            "batch_complete":(index+1)*batch_size>=bars.len()}),
                    );
                    assert_eq!(out["status"], "executed", "{out}");
                    state = out["result"]["state"].clone();
                }
                assert_eq!(state, expected);
            }
        }
        let mut next = bars.last().unwrap().clone();
        next.0 += 300_000;
        let continued = invoke(
            "market",
            state,
            json!({"symbol":"BTC-USDT","timeframe":"5m","candles":[next],"current_price":100.}),
        );
        let mut extended = bars;
        extended.push(next);
        let rebuilt = invoke(
            "market",
            Value::Null,
            json!({"symbol":"BTC-USDT","timeframe":"5m","candles":extended,"current_price":100.}),
        );
        // The retained Present window starts at initial admission, not at
        // every refresh; only the formation boundary differs here.
        let mut expected = rebuilt["result"]["state"].clone();
        expected["frames"][0]["formation_start"] =
            continued["result"]["state"]["frames"][0]["formation_start"].clone();
        assert_eq!(continued["result"]["state"], expected);
    }

    #[test]
    fn changed_symbol_discards_old_lines_and_partial_data_is_not_a_candidate() {
        let initial = invoke(
            "market",
            Value::Null,
            json!({"symbol":"BTC-USDT","timeframe":"5m","candles":repeated_swings(),"current_price":100.}),
        );
        let initial = invoke("present", initial["result"]["state"].clone(), Value::Null);
        assert!(!initial["result"]["view"]["rows"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(!initial["result"]["view"]["message"]
            .as_str()
            .unwrap()
            .contains("Entry candidate"));
        let raw = json!({"schema_version":1,"plugin_id":PLUGIN_ID,"host_method":"workspace",
            "action":"inspect","state":initial["result"]["state"],"observed_at_ms":100_000_000,
            "settings":{"symbol":"XRP-USDT","detection_length":7,"margin":6.9}});
        let out: Value =
            serde_json::from_slice(&evaluate_json(&serde_json::to_vec(&raw).unwrap())).unwrap();
        assert!(out["result"]["state"]["frames"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(out["result"]["view"]["rows"].as_array().unwrap().is_empty());
        assert_eq!(out["result"]["state"]["settings"]["symbol"], "XRP-USDT");
    }

    #[test]
    fn older_quote_cannot_touch_a_more_recently_observed_line() {
        let snapshot = json!({"symbol":"BTC-USDT","timeframe":"5m",
            "candles":repeated_swings(),"current_price":100.});
        let accepted = invoke(
            "market",
            serde_json::to_value(State::new(settings())).unwrap(),
            snapshot.clone(),
        );
        let state = accepted["result"]["state"].clone();
        assert_eq!(state["frames"][0]["buy"][0]["touched"], false);
        let raw = json!({"schema_version":1,"plugin_id":PLUGIN_ID,"host_method":"workspace",
            "action":"market","state":state,"observed_at_ms":90_000_000,
            "snapshot":{"current_price":106.,"symbol":"BTC-USDT","timeframe":"5m",
                "candles":snapshot["candles"]}});
        let replay: Value =
            serde_json::from_slice(&evaluate_json(&serde_json::to_vec(&raw).unwrap())).unwrap();
        assert_eq!(replay["status"], "executed");
        assert_eq!(replay["result"]["state"], state);
    }

    #[test]
    fn offline_gap_rebuilds_from_available_history_without_phantom_candles() {
        let initial = invoke(
            "market",
            Value::Null,
            json!({"symbol":"BTC-USDT","timeframe":"5m","candles":repeated_swings(),"current_price":100.}),
        );
        let bars: Vec<_> = repeated_swings()
            .into_iter()
            .map(|mut c| {
                c.0 += 36_000_000;
                c
            })
            .collect();
        let snapshot =
            json!({"symbol":"BTC-USDT","timeframe":"5m","candles":bars,"current_price":100.});
        let resumed = invoke(
            "market",
            initial["result"]["state"].clone(),
            snapshot.clone(),
        );
        let fresh = invoke("market", Value::Null, snapshot);
        assert_eq!(resumed["status"], "executed");
        assert_eq!(resumed["result"]["state"], fresh["result"]["state"]);
    }
}
