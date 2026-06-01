use num_bigint::BigInt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const PLUGIN_ID: &str = "hivra.contract.bingx-futures-trading.v1";
const CONTRACT_KIND: &str = "bingx_futures_order_intent";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrderSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrderType {
    Market,
    Limit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryMode {
    Direct,
    ZonePending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ZoneSide {
    Buyside,
    Sellside,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZonePriceRule {
    ZoneLow,
    ZoneMid,
    ZoneHigh,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BingxIntentInput {
    pub schema_version: u32,
    pub plugin_id: String,
    pub peer_hex: String,
    pub client_order_id: String,
    pub symbol: String,
    pub side: String,
    pub order_type: String,
    pub quantity_decimal: String,
    pub limit_price_decimal: Option<String>,
    pub time_in_force: Option<String>,
    pub entry_mode: Option<String>,
    pub zone_side: Option<String>,
    pub zone_low_decimal: Option<String>,
    pub zone_high_decimal: Option<String>,
    pub zone_price_rule: Option<String>,
    pub manual_entry_price_decimal: Option<String>,
    pub trigger_price_decimal: Option<String>,
    pub stop_loss_decimal: Option<String>,
    pub take_profit_decimal: Option<String>,
    pub created_at_utc: String,
    pub strategy_tag: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CanonicalIntent {
    schema_version: u32,
    plugin_id: String,
    contract_kind: String,
    peer_hex: String,
    client_order_id: String,
    symbol: String,
    side: OrderSide,
    order_type: OrderType,
    quantity_decimal: String,
    limit_price_decimal: Option<String>,
    time_in_force: Option<String>,
    entry_mode: EntryMode,
    zone_side: Option<ZoneSide>,
    zone_low_decimal: Option<String>,
    zone_high_decimal: Option<String>,
    zone_price_rule: Option<ZonePriceRule>,
    trigger_price_decimal: Option<String>,
    stop_loss_decimal: Option<String>,
    take_profit_decimal: Option<String>,
    created_at_utc: String,
    strategy_tag: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BingxIntentOutput {
    pub side: OrderSide,
    pub order_type: OrderType,
    pub entry_mode: EntryMode,
    pub limit_price_decimal: Option<String>,
    pub zone_side: Option<ZoneSide>,
    pub zone_price_rule: Option<ZonePriceRule>,
    pub trigger_price_decimal: Option<String>,
    pub stop_loss_decimal: Option<String>,
    pub take_profit_decimal: Option<String>,
    pub canonical_json: String,
    pub intent_hash_hex: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BingxContractError(pub String);

impl std::fmt::Display for BingxContractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for BingxContractError {}

#[no_mangle]
pub extern "C" fn hivra_plugin_abi_version() -> u32 {
    1
}

#[no_mangle]
pub extern "C" fn hivra_plugin_contract_id() -> u32 {
    1
}

#[no_mangle]
pub extern "C" fn hivra_bingx_parse_side_code(raw_side_code: u32) -> u32 {
    match raw_side_code {
        0 => 0,
        1 => 1,
        _ => 255,
    }
}

pub fn evaluate_from_json(raw_json: &str) -> Result<BingxIntentOutput, BingxContractError> {
    let input: BingxIntentInput = serde_json::from_str(raw_json)
        .map_err(|error| BingxContractError(format!("invalid_json: {error}")))?;
    evaluate(input)
}

pub fn evaluate(input: BingxIntentInput) -> Result<BingxIntentOutput, BingxContractError> {
    validate_common(&input)?;

    let normalized_peer = input.peer_hex.trim().to_lowercase();
    let normalized_client_order_id = input.client_order_id.trim().to_string();
    let normalized_symbol = input.symbol.trim().to_uppercase();
    let parsed_side = parse_side(&input.side)?;
    let parsed_order_type = parse_order_type(&input.order_type)?;
    let parsed_entry_mode = parse_entry_mode(input.entry_mode.as_deref())?;

    let normalized_quantity = normalize_decimal(&input.quantity_decimal, "quantity_decimal", 8)?;

    let mut normalized_limit_price: Option<String> = None;
    let normalized_tif = if parsed_order_type == OrderType::Market {
        if is_provided(input.limit_price_decimal.as_deref()) {
            return Err(BingxContractError(
                "limit_price_decimal is not allowed for market orders".to_string(),
            ));
        }
        if is_provided(input.time_in_force.as_deref()) {
            return Err(BingxContractError(
                "time_in_force is not allowed for market orders".to_string(),
            ));
        }
        None
    } else {
        if parsed_entry_mode == EntryMode::Direct {
            let limit = input.limit_price_decimal.as_deref().ok_or_else(|| {
                BingxContractError("limit_price_decimal is required for limit orders".to_string())
            })?;
            normalized_limit_price = Some(normalize_decimal(limit, "limit_price_decimal", 8)?);
        }
        Some(normalize_tif(input.time_in_force.as_deref())?)
    };

    let mut parsed_zone_side: Option<ZoneSide> = None;
    let mut normalized_zone_low: Option<String> = None;
    let mut normalized_zone_high: Option<String> = None;
    let mut parsed_zone_price_rule: Option<ZonePriceRule> = None;
    let mut normalized_trigger_price: Option<String> = None;
    let mut normalized_stop_loss: Option<String> = None;
    let mut normalized_take_profit: Option<String> = None;

    if parsed_entry_mode == EntryMode::ZonePending {
        if parsed_order_type != OrderType::Limit {
            return Err(BingxContractError(
                "entry_mode=zone_pending requires order_type=limit".to_string(),
            ));
        }

        let zone_side = parse_zone_side(input.zone_side.as_deref())?;
        if parsed_side == OrderSide::Buy && zone_side != ZoneSide::Buyside {
            return Err(BingxContractError(
                "buy orders require zone_side=buyside in zone_pending mode".to_string(),
            ));
        }
        if parsed_side == OrderSide::Sell && zone_side != ZoneSide::Sellside {
            return Err(BingxContractError(
                "sell orders require zone_side=sellside in zone_pending mode".to_string(),
            ));
        }
        parsed_zone_side = Some(zone_side);

        let zone_low = normalize_decimal(
            require_value(input.zone_low_decimal.as_deref(), "zone_low_decimal")?,
            "zone_low_decimal",
            8,
        )?;
        let zone_high = normalize_decimal(
            require_value(input.zone_high_decimal.as_deref(), "zone_high_decimal")?,
            "zone_high_decimal",
            8,
        )?;
        let low_scaled = to_scaled_int(&zone_low, 8)?;
        let high_scaled = to_scaled_int(&zone_high, 8)?;
        if low_scaled >= high_scaled {
            return Err(BingxContractError(
                "zone_low_decimal must be less than zone_high_decimal".to_string(),
            ));
        }
        normalized_zone_low = Some(zone_low.clone());
        normalized_zone_high = Some(zone_high.clone());

        let zone_price_rule = parse_zone_price_rule(input.zone_price_rule.as_deref())?;
        parsed_zone_price_rule = Some(zone_price_rule);

        let derived_entry_price = match zone_price_rule {
            ZonePriceRule::ZoneLow => zone_low,
            ZonePriceRule::ZoneHigh => zone_high,
            ZonePriceRule::ZoneMid => {
                let mid_scaled = (low_scaled + high_scaled) / BigInt::from(2u8);
                from_scaled_int(&mid_scaled, 8)
            }
            ZonePriceRule::Manual => {
                let manual = normalize_decimal(
                    require_value(
                        input.manual_entry_price_decimal.as_deref(),
                        "manual_entry_price_decimal",
                    )?,
                    "manual_entry_price_decimal",
                    8,
                )?;
                let manual_scaled = to_scaled_int(&manual, 8)?;
                if manual_scaled < low_scaled || manual_scaled > high_scaled {
                    return Err(BingxContractError(
                        "manual_entry_price_decimal must stay inside [zone_low_decimal, zone_high_decimal]".to_string(),
                    ));
                }
                manual
            }
        };
        normalized_limit_price = Some(derived_entry_price.clone());

        if is_provided(input.limit_price_decimal.as_deref()) {
            let provided = normalize_decimal(
                input.limit_price_decimal.as_deref().unwrap_or(""),
                "limit_price_decimal",
                8,
            )?;
            if provided != derived_entry_price {
                return Err(BingxContractError(
                    "limit_price_decimal must match derived zone entry price".to_string(),
                ));
            }
        }

        normalized_trigger_price = normalize_optional_decimal(
            input.trigger_price_decimal.as_deref(),
            "trigger_price_decimal",
        )?;
        normalized_stop_loss =
            normalize_optional_decimal(input.stop_loss_decimal.as_deref(), "stop_loss_decimal")?;
        normalized_take_profit = normalize_optional_decimal(
            input.take_profit_decimal.as_deref(),
            "take_profit_decimal",
        )?;

        let entry_scaled = to_scaled_int(normalized_limit_price.as_deref().unwrap_or(""), 8)?;
        if parsed_side == OrderSide::Buy {
            if let Some(stop_loss) = normalized_stop_loss.as_deref() {
                if to_scaled_int(stop_loss, 8)? >= entry_scaled {
                    return Err(BingxContractError(
                        "stop_loss_decimal must be below entry price for buy side".to_string(),
                    ));
                }
            }
            if let Some(take_profit) = normalized_take_profit.as_deref() {
                if to_scaled_int(take_profit, 8)? <= entry_scaled {
                    return Err(BingxContractError(
                        "take_profit_decimal must be above entry price for buy side".to_string(),
                    ));
                }
            }
        } else {
            if let Some(stop_loss) = normalized_stop_loss.as_deref() {
                if to_scaled_int(stop_loss, 8)? <= entry_scaled {
                    return Err(BingxContractError(
                        "stop_loss_decimal must be above entry price for sell side".to_string(),
                    ));
                }
            }
            if let Some(take_profit) = normalized_take_profit.as_deref() {
                if to_scaled_int(take_profit, 8)? >= entry_scaled {
                    return Err(BingxContractError(
                        "take_profit_decimal must be below entry price for sell side".to_string(),
                    ));
                }
            }
        }
    } else if is_provided(input.zone_side.as_deref())
        || is_provided(input.zone_low_decimal.as_deref())
        || is_provided(input.zone_high_decimal.as_deref())
        || is_provided(input.zone_price_rule.as_deref())
        || is_provided(input.manual_entry_price_decimal.as_deref())
        || is_provided(input.trigger_price_decimal.as_deref())
        || is_provided(input.stop_loss_decimal.as_deref())
        || is_provided(input.take_profit_decimal.as_deref())
    {
        return Err(BingxContractError(
            "zone_* parameters require entry_mode=zone_pending".to_string(),
        ));
    }

    let normalized_created_at_utc = input.created_at_utc.trim().to_string();
    if !is_valid_iso_utc(&normalized_created_at_utc) {
        return Err(BingxContractError(
            "created_at_utc must be ISO-8601 UTC instant".to_string(),
        ));
    }
    let normalized_strategy_tag = normalize_optional_trimmed(input.strategy_tag.as_deref());
    if let Some(strategy_tag) = normalized_strategy_tag.as_deref() {
        if strategy_tag.len() > 64 {
            return Err(BingxContractError(
                "strategy_tag must be <= 64 chars".to_string(),
            ));
        }
    }

    let canonical = CanonicalIntent {
        schema_version: 1,
        plugin_id: PLUGIN_ID.to_string(),
        contract_kind: CONTRACT_KIND.to_string(),
        peer_hex: normalized_peer,
        client_order_id: normalized_client_order_id,
        symbol: normalized_symbol,
        side: parsed_side,
        order_type: parsed_order_type,
        quantity_decimal: normalized_quantity,
        limit_price_decimal: normalized_limit_price.clone(),
        time_in_force: normalized_tif,
        entry_mode: parsed_entry_mode,
        zone_side: parsed_zone_side,
        zone_low_decimal: normalized_zone_low,
        zone_high_decimal: normalized_zone_high,
        zone_price_rule: parsed_zone_price_rule,
        trigger_price_decimal: normalized_trigger_price.clone(),
        stop_loss_decimal: normalized_stop_loss.clone(),
        take_profit_decimal: normalized_take_profit.clone(),
        created_at_utc: normalized_created_at_utc,
        strategy_tag: normalized_strategy_tag,
    };
    let canonical_json = serde_json::to_string(&canonical)
        .map_err(|error| BingxContractError(format!("canonical_serialize_failed: {error}")))?;
    let intent_hash_hex = sha256_hex(canonical_json.as_bytes());

    Ok(BingxIntentOutput {
        side: canonical.side,
        order_type: canonical.order_type,
        entry_mode: canonical.entry_mode,
        limit_price_decimal: canonical.limit_price_decimal,
        zone_side: canonical.zone_side,
        zone_price_rule: canonical.zone_price_rule,
        trigger_price_decimal: normalized_trigger_price,
        stop_loss_decimal: normalized_stop_loss,
        take_profit_decimal: normalized_take_profit,
        canonical_json,
        intent_hash_hex,
    })
}

fn validate_common(input: &BingxIntentInput) -> Result<(), BingxContractError> {
    if input.schema_version != 1 {
        return Err(BingxContractError(
            "invalid_schema_version: expected 1".to_string(),
        ));
    }
    if input.plugin_id.trim() != PLUGIN_ID {
        return Err(BingxContractError(
            "invalid_plugin_id: unsupported plugin id".to_string(),
        ));
    }
    let peer = input.peer_hex.trim();
    if !is_hex64(peer) {
        return Err(BingxContractError(
            "peer_hex must be a 64-char lowercase hex".to_string(),
        ));
    }

    let order_id = input.client_order_id.trim();
    if order_id.is_empty() {
        return Err(BingxContractError(
            "client_order_id is required".to_string(),
        ));
    }
    if order_id.len() > 128 {
        return Err(BingxContractError(
            "client_order_id must be <= 128 chars".to_string(),
        ));
    }

    let symbol = input.symbol.trim().to_uppercase();
    if !is_valid_symbol(&symbol) {
        return Err(BingxContractError("symbol format is invalid".to_string()));
    }
    Ok(())
}

fn parse_side(value: &str) -> Result<OrderSide, BingxContractError> {
    match value.trim().to_lowercase().as_str() {
        "buy" => Ok(OrderSide::Buy),
        "sell" => Ok(OrderSide::Sell),
        _ => Err(BingxContractError("side must be buy or sell".to_string())),
    }
}

fn parse_order_type(value: &str) -> Result<OrderType, BingxContractError> {
    match value.trim().to_lowercase().as_str() {
        "market" => Ok(OrderType::Market),
        "limit" => Ok(OrderType::Limit),
        _ => Err(BingxContractError(
            "order_type must be market or limit".to_string(),
        )),
    }
}

fn parse_entry_mode(value: Option<&str>) -> Result<EntryMode, BingxContractError> {
    match value.unwrap_or("direct").trim().to_lowercase().as_str() {
        "" | "direct" => Ok(EntryMode::Direct),
        "zone_pending" => Ok(EntryMode::ZonePending),
        _ => Err(BingxContractError(
            "entry_mode must be direct or zone_pending".to_string(),
        )),
    }
}

fn parse_zone_side(value: Option<&str>) -> Result<ZoneSide, BingxContractError> {
    match value.unwrap_or("").trim().to_lowercase().as_str() {
        "buyside" => Ok(ZoneSide::Buyside),
        "sellside" => Ok(ZoneSide::Sellside),
        _ => Err(BingxContractError(
            "zone_side must be buyside or sellside for zone_pending".to_string(),
        )),
    }
}

fn parse_zone_price_rule(value: Option<&str>) -> Result<ZonePriceRule, BingxContractError> {
    match value.unwrap_or("zone_mid").trim().to_lowercase().as_str() {
        "zone_low" => Ok(ZonePriceRule::ZoneLow),
        "zone_mid" => Ok(ZonePriceRule::ZoneMid),
        "zone_high" => Ok(ZonePriceRule::ZoneHigh),
        "manual" => Ok(ZonePriceRule::Manual),
        _ => Err(BingxContractError(
            "zone_price_rule must be zone_low, zone_mid, zone_high, or manual".to_string(),
        )),
    }
}

fn normalize_tif(value: Option<&str>) -> Result<String, BingxContractError> {
    let normalized = value.unwrap_or("GTC").trim().to_uppercase();
    if !matches!(normalized.as_str(), "GTC" | "IOC" | "FOK") {
        return Err(BingxContractError(
            "time_in_force must be GTC, IOC, or FOK".to_string(),
        ));
    }
    Ok(normalized)
}

fn normalize_decimal(
    raw_value: &str,
    field: &str,
    max_scale: usize,
) -> Result<String, BingxContractError> {
    let raw = raw_value.trim();
    if raw.is_empty() {
        return Err(BingxContractError(format!(
            "{field} must be a positive decimal"
        )));
    }
    if !raw.chars().all(|ch| ch.is_ascii_digit() || ch == '.') || raw.matches('.').count() > 1 {
        return Err(BingxContractError(format!(
            "{field} must be a positive decimal"
        )));
    }

    let mut parts = raw.split('.');
    let whole_part = parts.next().unwrap_or_default();
    let frac_part = parts.next().unwrap_or_default();
    if whole_part.is_empty() || !whole_part.chars().all(|c| c.is_ascii_digit()) {
        return Err(BingxContractError(format!(
            "{field} must be a positive decimal"
        )));
    }
    if !frac_part.is_empty() && !frac_part.chars().all(|c| c.is_ascii_digit()) {
        return Err(BingxContractError(format!(
            "{field} must be a positive decimal"
        )));
    }
    if frac_part.len() > max_scale {
        return Err(BingxContractError(format!(
            "{field} precision must be <= {max_scale}"
        )));
    }

    let normalized_whole = whole_part.trim_start_matches('0');
    let normalized_whole = if normalized_whole.is_empty() {
        "0"
    } else {
        normalized_whole
    };
    let normalized_frac = frac_part.trim_end_matches('0');
    let normalized = if normalized_frac.is_empty() {
        normalized_whole.to_string()
    } else {
        format!("{normalized_whole}.{normalized_frac}")
    };
    if normalized == "0" {
        return Err(BingxContractError(format!("{field} must be > 0")));
    }
    Ok(normalized)
}

fn normalize_optional_decimal(
    value: Option<&str>,
    field: &str,
) -> Result<Option<String>, BingxContractError> {
    if !is_provided(value) {
        return Ok(None);
    }
    let normalized = normalize_decimal(value.unwrap_or_default(), field, 8)?;
    Ok(Some(normalized))
}

fn to_scaled_int(normalized: &str, scale: u32) -> Result<BigInt, BingxContractError> {
    let mut parts = normalized.split('.');
    let whole = parts.next().unwrap_or_default();
    let frac = parts.next().unwrap_or_default();
    let base = BigInt::from(10u8).pow(scale);
    let whole_int = BigInt::parse_bytes(whole.as_bytes(), 10)
        .ok_or_else(|| BingxContractError("failed to parse whole decimal part".to_string()))?;
    let padded_frac = format!("{frac:0<width$}", width = scale as usize);
    let frac_int = if padded_frac.is_empty() {
        BigInt::from(0u8)
    } else {
        BigInt::parse_bytes(padded_frac.as_bytes(), 10).ok_or_else(|| {
            BingxContractError("failed to parse fraction decimal part".to_string())
        })?
    };
    Ok(whole_int * base + frac_int)
}

fn from_scaled_int(value: &BigInt, scale: u32) -> String {
    let base = BigInt::from(10u8).pow(scale);
    let whole = value / &base;
    let mut frac = (value % &base).to_string();
    if frac != "0" {
        while frac.len() < scale as usize {
            frac = format!("0{frac}");
        }
        frac = frac.trim_end_matches('0').to_string();
    }
    if frac.is_empty() || frac == "0" {
        whole.to_string()
    } else {
        format!("{whole}.{frac}")
    }
}

fn is_valid_iso_utc(value: &str) -> bool {
    if value.len() < 20 || !value.ends_with('Z') {
        return false;
    }
    value.contains('T')
}

fn is_hex64(value: &str) -> bool {
    value.len() == 64
        && value
            .chars()
            .all(|ch| ch.is_ascii_digit() || ('a'..='f').contains(&ch) || ('A'..='F').contains(&ch))
}

fn is_valid_symbol(value: &str) -> bool {
    if value.len() < 2 || value.len() > 41 {
        return false;
    }
    let separators = ['-', '_', '/'];
    let separator_index = value.char_indices().find_map(|(idx, ch)| {
        if separators.contains(&ch) {
            Some(idx)
        } else {
            None
        }
    });

    match separator_index {
        None => (2..=20).contains(&value.len()) && value.chars().all(is_upper_alnum),
        Some(index) => {
            if value[index + 1..]
                .chars()
                .any(|ch| separators.contains(&ch))
            {
                return false;
            }
            let left = &value[..index];
            let right = &value[index + 1..];
            (2..=20).contains(&left.len())
                && (2..=20).contains(&right.len())
                && left.chars().all(is_upper_alnum)
                && right.chars().all(is_upper_alnum)
        }
    }
}

fn is_upper_alnum(ch: char) -> bool {
    ch.is_ascii_digit() || ch.is_ascii_uppercase()
}

fn is_provided(value: Option<&str>) -> bool {
    value.map(|s| !s.trim().is_empty()).unwrap_or(false)
}

fn require_value<'a>(value: Option<&'a str>, field: &str) -> Result<&'a str, BingxContractError> {
    if !is_provided(value) {
        return Err(BingxContractError(format!(
            "{field} must be a positive decimal"
        )));
    }
    Ok(value.unwrap_or_default())
}

fn normalize_optional_trimmed(value: Option<&str>) -> Option<String> {
    let trimmed = value.unwrap_or_default().trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_input() -> BingxIntentInput {
        BingxIntentInput {
            schema_version: 1,
            plugin_id: PLUGIN_ID.to_string(),
            peer_hex: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            client_order_id: "ord-1".to_string(),
            symbol: "btc-usdt".to_string(),
            side: "buy".to_string(),
            order_type: "limit".to_string(),
            quantity_decimal: "000.0100".to_string(),
            limit_price_decimal: Some("060000.0000".to_string()),
            time_in_force: Some("gtc".to_string()),
            entry_mode: None,
            zone_side: None,
            zone_low_decimal: None,
            zone_high_decimal: None,
            zone_price_rule: None,
            manual_entry_price_decimal: None,
            trigger_price_decimal: None,
            stop_loss_decimal: None,
            take_profit_decimal: None,
            created_at_utc: "2026-04-09T10:00:00Z".to_string(),
            strategy_tag: Some("demo".to_string()),
        }
    }

    #[test]
    fn deterministic_hash_for_identical_inputs() {
        let first = evaluate(sample_input()).expect("first evaluation should pass");
        let second = evaluate(sample_input()).expect("second evaluation should pass");
        assert_eq!(first.canonical_json, second.canonical_json);
        assert_eq!(first.intent_hash_hex, second.intent_hash_hex);
        assert_eq!(first.limit_price_decimal.as_deref(), Some("60000"));
    }

    #[test]
    fn rejects_limit_without_price() {
        let mut input = sample_input();
        input.limit_price_decimal = None;
        let error = evaluate(input).expect_err("missing limit price should reject");
        assert!(error
            .to_string()
            .contains("limit_price_decimal is required for limit orders"));
    }

    #[test]
    fn rejects_market_with_limit_fields() {
        let mut input = sample_input();
        input.order_type = "market".to_string();
        input.limit_price_decimal = Some("50000".to_string());
        let error = evaluate(input).expect_err("market with limit fields should reject");
        assert!(error
            .to_string()
            .contains("limit_price_decimal is not allowed for market orders"));
    }

    #[test]
    fn supports_zone_pending_mid_rule() {
        let mut input = sample_input();
        input.client_order_id = "ord-zone-1".to_string();
        input.entry_mode = Some("zone_pending".to_string());
        input.side = "buy".to_string();
        input.order_type = "limit".to_string();
        input.limit_price_decimal = None;
        input.zone_side = Some("buyside".to_string());
        input.zone_low_decimal = Some("58000".to_string());
        input.zone_high_decimal = Some("60000".to_string());
        input.zone_price_rule = Some("zone_mid".to_string());
        input.trigger_price_decimal = Some("58900".to_string());
        input.stop_loss_decimal = Some("57500".to_string());
        input.take_profit_decimal = Some("62000".to_string());
        input.strategy_tag = Some("zone-demo".to_string());

        let result = evaluate(input).expect("zone pending should pass");
        assert_eq!(result.entry_mode, EntryMode::ZonePending);
        assert_eq!(result.zone_side, Some(ZoneSide::Buyside));
        assert_eq!(result.zone_price_rule, Some(ZonePriceRule::ZoneMid));
        assert_eq!(result.limit_price_decimal.as_deref(), Some("59000"));
        assert_eq!(result.stop_loss_decimal.as_deref(), Some("57500"));
        assert_eq!(result.take_profit_decimal.as_deref(), Some("62000"));
    }

    #[test]
    fn has_stable_golden_hash_vector() {
        let result = evaluate(sample_input()).expect("evaluation should pass");
        assert_eq!(
            result.intent_hash_hex,
            "40ae3cab741eb177ac422091498c7f5ecfc86e4eb9c8170a548c04c4b244b29f"
        );
    }
}
