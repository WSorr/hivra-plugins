use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const PLUGIN_ID: &str = "hivra.contract.temperature-li.tomorrow.v1";
const CONTRACT_KIND: &str = "temperature_tomorrow_liechtenstein";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposerRule {
    Above,
    Below,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractOutcome {
    ProposerWins,
    CounterpartyWins,
    Draw,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemperatureSettlementInput {
    pub schema_version: u32,
    pub plugin_id: String,
    pub peer_hex: String,
    pub location_code: String,
    pub target_date_utc: String,
    pub threshold_deci_celsius: i32,
    pub observed_deci_celsius: i32,
    pub proposer_rule: ProposerRule,
    pub draw_on_equal: bool,
    pub oracle_source_id: String,
    pub oracle_event_id: String,
    pub oracle_recorded_at_utc: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CanonicalSettlement {
    schema_version: u32,
    plugin_id: String,
    contract_kind: String,
    peer_hex: String,
    location_code: String,
    target_date_utc: String,
    threshold_deci_celsius: i32,
    observed_deci_celsius: i32,
    proposer_rule: ProposerRule,
    draw_on_equal: bool,
    outcome: ContractOutcome,
    winner_role: String,
    oracle_source_id: String,
    oracle_event_id: String,
    oracle_recorded_at_utc: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemperatureSettlementOutput {
    pub outcome: ContractOutcome,
    pub winner_role: String,
    pub canonical_json: String,
    pub settlement_hash_hex: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemperatureContractError(pub String);

impl std::fmt::Display for TemperatureContractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for TemperatureContractError {}

#[no_mangle]
pub extern "C" fn hivra_plugin_abi_version() -> u32 {
    1
}

#[no_mangle]
pub extern "C" fn hivra_plugin_contract_id() -> u32 {
    2
}

#[no_mangle]
pub extern "C" fn hivra_temperature_li_tomorrow_eval_outcome(
    threshold_deci_celsius: i32,
    observed_deci_celsius: i32,
    proposer_rule_code: u32,
    draw_on_equal: u32,
) -> u32 {
    let proposer_rule = match proposer_rule_code {
        0 => ProposerRule::Above,
        1 => ProposerRule::Below,
        _ => return 255,
    };
    let outcome = resolve_outcome(
        observed_deci_celsius,
        threshold_deci_celsius,
        proposer_rule,
        draw_on_equal != 0,
    );
    match outcome {
        ContractOutcome::ProposerWins => 0,
        ContractOutcome::CounterpartyWins => 1,
        ContractOutcome::Draw => 2,
    }
}

pub fn evaluate_from_json(raw_json: &str) -> Result<TemperatureSettlementOutput, TemperatureContractError> {
    let input: TemperatureSettlementInput = serde_json::from_str(raw_json).map_err(|error| {
        TemperatureContractError(format!("invalid_json: {error}"))
    })?;
    evaluate(input)
}

pub fn evaluate(input: TemperatureSettlementInput) -> Result<TemperatureSettlementOutput, TemperatureContractError> {
    validate_input(&input)?;

    let normalized_location = input.location_code.trim().to_uppercase();
    let outcome = resolve_outcome(
        input.observed_deci_celsius,
        input.threshold_deci_celsius,
        input.proposer_rule,
        input.draw_on_equal,
    );
    let winner_role = winner_role_for_outcome(outcome).to_string();

    let canonical = CanonicalSettlement {
        schema_version: 1,
        plugin_id: input.plugin_id,
        contract_kind: CONTRACT_KIND.to_string(),
        peer_hex: input.peer_hex,
        location_code: normalized_location,
        target_date_utc: input.target_date_utc,
        threshold_deci_celsius: input.threshold_deci_celsius,
        observed_deci_celsius: input.observed_deci_celsius,
        proposer_rule: input.proposer_rule,
        draw_on_equal: input.draw_on_equal,
        outcome,
        winner_role: winner_role.clone(),
        oracle_source_id: input.oracle_source_id,
        oracle_event_id: input.oracle_event_id,
        oracle_recorded_at_utc: input.oracle_recorded_at_utc,
    };

    let canonical_json = serde_json::to_string(&canonical).map_err(|error| {
        TemperatureContractError(format!("canonical_serialize_failed: {error}"))
    })?;
    let settlement_hash_hex = sha256_hex(canonical_json.as_bytes());

    Ok(TemperatureSettlementOutput {
        outcome,
        winner_role,
        canonical_json,
        settlement_hash_hex,
    })
}

fn validate_input(input: &TemperatureSettlementInput) -> Result<(), TemperatureContractError> {
    if input.schema_version != 1 {
        return Err(TemperatureContractError(
            "invalid_schema_version: expected 1".to_string(),
        ));
    }
    if input.plugin_id.trim() != PLUGIN_ID {
        return Err(TemperatureContractError(
            "invalid_plugin_id: unsupported plugin id".to_string(),
        ));
    }
    if !is_valid_hex64(&input.peer_hex) {
        return Err(TemperatureContractError(
            "invalid_peer_hex: expected 64 lowercase/uppercase hex chars".to_string(),
        ));
    }

    let location = input.location_code.trim().to_uppercase();
    if location != "LI" {
        return Err(TemperatureContractError(
            "invalid_location_code: only LI is supported".to_string(),
        ));
    }
    if !is_valid_date_yyyy_mm_dd(&input.target_date_utc) {
        return Err(TemperatureContractError(
            "invalid_target_date_utc: expected YYYY-MM-DD".to_string(),
        ));
    }
    if input.oracle_source_id.trim().is_empty() || input.oracle_event_id.trim().is_empty() {
        return Err(TemperatureContractError(
            "invalid_oracle_metadata: source_id and event_id are required".to_string(),
        ));
    }
    if !is_valid_utc_instant(&input.oracle_recorded_at_utc) {
        return Err(TemperatureContractError(
            "invalid_oracle_recorded_at_utc: expected YYYY-MM-DDTHH:MM:SSZ".to_string(),
        ));
    }
    Ok(())
}

fn resolve_outcome(
    observed: i32,
    threshold: i32,
    proposer_rule: ProposerRule,
    draw_on_equal: bool,
) -> ContractOutcome {
    if observed == threshold {
        if draw_on_equal {
            return ContractOutcome::Draw;
        }
        return match proposer_rule {
            ProposerRule::Above => ContractOutcome::CounterpartyWins,
            ProposerRule::Below => ContractOutcome::ProposerWins,
        };
    }

    let is_above = observed > threshold;
    let proposer_wins = match proposer_rule {
        ProposerRule::Above => is_above,
        ProposerRule::Below => !is_above,
    };
    if proposer_wins {
        ContractOutcome::ProposerWins
    } else {
        ContractOutcome::CounterpartyWins
    }
}

fn winner_role_for_outcome(outcome: ContractOutcome) -> &'static str {
    match outcome {
        ContractOutcome::ProposerWins => "proposer",
        ContractOutcome::CounterpartyWins => "counterparty",
        ContractOutcome::Draw => "draw",
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

fn is_valid_hex64(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.len() != 64 {
        return false;
    }
    trimmed.bytes().all(|byte| {
        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte) || (b'A'..=b'F').contains(&byte)
    })
}

fn is_valid_date_yyyy_mm_dd(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10 {
        return false;
    }
    for (index, byte) in bytes.iter().enumerate() {
        let must_be_dash = index == 4 || index == 7;
        if must_be_dash {
            if *byte != b'-' {
                return false;
            }
            continue;
        }
        if !byte.is_ascii_digit() {
            return false;
        }
    }
    true
}

fn is_valid_utc_instant(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 20 {
        return false;
    }
    for (index, byte) in bytes.iter().enumerate() {
        let must_be_separator =
            index == 4 || index == 7 || index == 10 || index == 13 || index == 16;
        if must_be_separator {
            let expected = match index {
                4 | 7 => b'-',
                10 => b'T',
                13 | 16 => b':',
                _ => unreachable!(),
            };
            if *byte != expected {
                return false;
            }
            continue;
        }
        if index == 19 {
            return *byte == b'Z';
        }
        if !byte.is_ascii_digit() {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_input() -> TemperatureSettlementInput {
        TemperatureSettlementInput {
            schema_version: 1,
            plugin_id: PLUGIN_ID.to_string(),
            peer_hex: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .to_string(),
            location_code: "LI".to_string(),
            target_date_utc: "2026-04-10".to_string(),
            threshold_deci_celsius: 85,
            observed_deci_celsius: 90,
            proposer_rule: ProposerRule::Above,
            draw_on_equal: true,
            oracle_source_id: "oracle.temperature.li.v1".to_string(),
            oracle_event_id: "evt-42".to_string(),
            oracle_recorded_at_utc: "2026-04-10T12:00:00Z".to_string(),
        }
    }

    #[test]
    fn settles_proposer_win_for_above_rule() {
        let result = evaluate(sample_input()).expect("settlement should succeed");
        assert_eq!(result.outcome, ContractOutcome::ProposerWins);
        assert_eq!(result.winner_role, "proposer");
    }

    #[test]
    fn settles_draw_when_equal_and_draw_on_equal_enabled() {
        let mut input = sample_input();
        input.observed_deci_celsius = input.threshold_deci_celsius;
        let result = evaluate(input).expect("settlement should succeed");
        assert_eq!(result.outcome, ContractOutcome::Draw);
        assert_eq!(result.winner_role, "draw");
    }

    #[test]
    fn settles_proposer_win_for_below_rule_on_equal_without_draw() {
        let mut input = sample_input();
        input.proposer_rule = ProposerRule::Below;
        input.draw_on_equal = false;
        input.observed_deci_celsius = input.threshold_deci_celsius;
        let result = evaluate(input).expect("settlement should succeed");
        assert_eq!(result.outcome, ContractOutcome::ProposerWins);
        assert_eq!(result.winner_role, "proposer");
    }

    #[test]
    fn output_is_deterministic_for_same_input() {
        let input = sample_input();
        let first = evaluate(input.clone()).expect("first result should succeed");
        let second = evaluate(input).expect("second result should succeed");
        assert_eq!(first.canonical_json, second.canonical_json);
        assert_eq!(first.settlement_hash_hex, second.settlement_hash_hex);
    }

    #[test]
    fn rejects_invalid_location() {
        let mut input = sample_input();
        input.location_code = "CH".to_string();
        let error = evaluate(input).expect_err("invalid location should reject");
        assert!(error
            .to_string()
            .contains("invalid_location_code: only LI is supported"));
    }

    #[test]
    fn has_stable_golden_hash_vector() {
        let result = evaluate(sample_input()).expect("settlement should succeed");
        assert_eq!(
            result.settlement_hash_hex,
            "2103cfe0b90790998eac636a2d80594e48668d69a5c7b9aff1a9f5469baa21b5"
        );
    }
}
