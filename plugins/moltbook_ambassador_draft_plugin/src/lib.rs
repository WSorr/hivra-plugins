use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const PLUGIN_ID: &str = "hivra.contract.moltbook-ambassador.v1";
const DRAFT_CONTRACT_KIND: &str = "moltbook_ambassador_draft";
const HEARTBEAT_CONTRACT_KIND: &str = "moltbook_ambassador_heartbeat_plan";
const PREPARE_DRAFT_METHOD: &str = "prepare_moltbook_draft";
const PLAN_HEARTBEAT_METHOD: &str = "plan_moltbook_heartbeat";
const ABI_SCHEMA_VERSION: u32 = 1;
#[cfg(target_arch = "wasm32")]
const MAX_ABI_INPUT_BYTES: usize = 64 * 1024;
#[cfg(target_arch = "wasm32")]
const MAX_ABI_OUTPUT_BYTES: usize = 128 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct DraftInput {
    schema_version: u32,
    plugin_id: String,
    bulletin_id: String,
    release_tag: String,
    category: String,
    facts: Vec<String>,
    title_hint: String,
    audience: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct HeartbeatHomeInput {
    unread_notification_count: u32,
    suggested_actions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct HeartbeatFeedPostInput {
    post_id: String,
    title: String,
    author_name: String,
    submolt_name: String,
    score: i64,
    comment_count: u32,
    is_verified: bool,
    is_spam: bool,
    created_at_utc: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct HeartbeatInput {
    schema_version: u32,
    plugin_id: String,
    host_method: String,
    observed_at_utc: String,
    allowed_topics: Vec<String>,
    home: HeartbeatHomeInput,
    feed: Vec<HeartbeatFeedPostInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CanonicalDraft {
    schema_version: u32,
    plugin_id: String,
    contract_kind: String,
    bulletin_id: String,
    release_tag: String,
    category: String,
    title: String,
    body: String,
    audience: String,
    approval_required: bool,
    safety_flags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct DraftOutput {
    canonical_json: String,
    draft_hash_hex: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CanonicalHeartbeatPlan {
    schema_version: u32,
    plugin_id: String,
    contract_kind: String,
    observed_at_utc: String,
    priority: String,
    reason: String,
    candidate_post_ids: Vec<String>,
    publish_allowed: bool,
    human_review_required: bool,
    safety_flags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct HeartbeatOutput {
    canonical_json: String,
    plan_hash_hex: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AbiEnvelope {
    schema_version: u32,
    status: String,
    result: Option<Value>,
    error_code: Option<String>,
    error_message: Option<String>,
}

#[no_mangle]
pub extern "C" fn hivra_plugin_abi_version() -> u32 {
    2
}

#[no_mangle]
pub extern "C" fn hivra_plugin_contract_id() -> u32 {
    4
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn hivra_alloc_v1(len: u32) -> u32 {
    if len == 0 || len as usize > MAX_ABI_INPUT_BYTES {
        return 0;
    }
    let mut bytes = Vec::<u8>::with_capacity(len as usize);
    let ptr = bytes.as_mut_ptr();
    std::mem::forget(bytes);
    ptr as u32
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub unsafe extern "C" fn hivra_dealloc_v1(ptr: u32, len: u32) {
    if ptr != 0 && len != 0 {
        let _ = Vec::from_raw_parts(ptr as *mut u8, 0, len as usize);
    }
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub unsafe extern "C" fn hivra_evaluate_v1(ptr: u32, len: u32) -> u64 {
    if ptr == 0 || len == 0 || len as usize > MAX_ABI_INPUT_BYTES {
        return write_output(rejected(
            "invalid_abi_input",
            "ABI input must be non-empty and within the size limit",
        ));
    }
    let input = std::slice::from_raw_parts(ptr as *const u8, len as usize);
    let output = match std::str::from_utf8(input) {
        Ok(raw) => evaluate_abi_json(raw),
        Err(_) => rejected("invalid_utf8", "ABI input must be UTF-8 JSON"),
    };
    write_output(output)
}

#[cfg(target_arch = "wasm32")]
unsafe fn write_output(output: Vec<u8>) -> u64 {
    if output.is_empty() || output.len() > MAX_ABI_OUTPUT_BYTES {
        return 0;
    }
    let mut output = output.into_boxed_slice();
    let ptr = output.as_mut_ptr() as u32;
    let len = output.len() as u32;
    std::mem::forget(output);
    ((ptr as u64) << 32) | len as u64
}

fn evaluate_abi_json(raw: &str) -> Vec<u8> {
    let result = evaluate_request(raw);
    let envelope = match result {
        Ok(result) => AbiEnvelope {
            schema_version: ABI_SCHEMA_VERSION,
            status: "executed".to_string(),
            result: Some(result),
            error_code: None,
            error_message: None,
        },
        Err(error) => AbiEnvelope {
            schema_version: ABI_SCHEMA_VERSION,
            status: "rejected".to_string(),
            result: None,
            error_code: Some("invalid_args".to_string()),
            error_message: Some(error),
        },
    };
    serde_json::to_vec(&envelope).unwrap_or_default()
}

fn evaluate_request(raw: &str) -> Result<Value, String> {
    let value =
        serde_json::from_str::<Value>(raw).map_err(|error| format!("invalid_json: {error}"))?;
    let method = value
        .get("host_method")
        .and_then(Value::as_str)
        .unwrap_or(PREPARE_DRAFT_METHOD);
    let output = match method {
        PREPARE_DRAFT_METHOD => {
            let input = serde_json::from_value::<DraftInput>(value)
                .map_err(|error| format!("invalid_draft_input: {error}"))?;
            serde_json::to_value(evaluate_draft(input)?).map_err(|error| error.to_string())?
        }
        PLAN_HEARTBEAT_METHOD => {
            let input = serde_json::from_value::<HeartbeatInput>(value)
                .map_err(|error| format!("invalid_heartbeat_input: {error}"))?;
            serde_json::to_value(evaluate_heartbeat(input)?).map_err(|error| error.to_string())?
        }
        _ => return Err("unsupported_method: unknown host_method".to_string()),
    };
    Ok(output)
}

fn evaluate_draft(input: DraftInput) -> Result<DraftOutput, String> {
    if input.schema_version != ABI_SCHEMA_VERSION {
        return Err("invalid_schema_version: expected 1".to_string());
    }
    if input.plugin_id.trim() != PLUGIN_ID {
        return Err("invalid_plugin_id: unsupported plugin id".to_string());
    }
    let bulletin_id = input.bulletin_id.trim();
    let release_tag = input.release_tag.trim();
    let category = input.category.trim();
    let title = input.title_hint.trim();
    let audience = input.audience.trim();
    if bulletin_id.is_empty() || bulletin_id.len() > 160 {
        return Err("bulletin_id must contain 1..160 UTF-8 bytes".to_string());
    }
    if release_tag.is_empty() || release_tag.len() > 80 {
        return Err("release_tag must contain 1..80 UTF-8 bytes".to_string());
    }
    if category.is_empty() || category.len() > 80 {
        return Err("category must contain 1..80 UTF-8 bytes".to_string());
    }
    if input.facts.is_empty() || input.facts.len() > 32 {
        return Err("facts must contain 1..32 items".to_string());
    }
    let facts = input
        .facts
        .iter()
        .map(|fact| fact.trim())
        .collect::<Vec<_>>();
    if facts
        .iter()
        .any(|fact| fact.is_empty() || fact.len() > 2048)
    {
        return Err("each fact must contain 1..2048 UTF-8 bytes".to_string());
    }
    if title.is_empty() || title.len() > 180 {
        return Err("title_hint must contain 1..180 UTF-8 bytes".to_string());
    }
    if audience.is_empty() || audience.len() > 80 {
        return Err("audience must contain 1..80 UTF-8 bytes".to_string());
    }
    let body = facts.join("\n");
    let lowered = format!("{category} {body} {title}").to_ascii_lowercase();
    let forbidden: [(&str, &[&str]); 4] = [
        ("crypto_promotion", &["bitcoin", "crypto", "token", "coin"]),
        ("financial_advice", &["buy", "sell", "profit", "investment"]),
        (
            "credential_or_secret",
            &["api_key", "password", "seed phrase", "private key"],
        ),
        (
            "medical_or_legal_advice",
            &["diagnose", "medical advice", "legal advice"],
        ),
    ];
    let mut flags = Vec::new();
    for (flag, terms) in forbidden {
        if terms.iter().any(|term| lowered.contains(term)) {
            flags.push(flag.to_string());
        }
    }
    if !flags.is_empty() {
        return Err(format!("unsafe_public_content: {}", flags.join(",")));
    }
    let canonical = CanonicalDraft {
        schema_version: ABI_SCHEMA_VERSION,
        plugin_id: PLUGIN_ID.to_string(),
        contract_kind: DRAFT_CONTRACT_KIND.to_string(),
        bulletin_id: bulletin_id.to_string(),
        release_tag: release_tag.to_string(),
        category: category.to_string(),
        title: title.to_string(),
        body,
        audience: audience.to_string(),
        approval_required: true,
        safety_flags: flags,
    };
    let canonical_json = serde_json::to_string(&canonical).map_err(|error| error.to_string())?;
    Ok(DraftOutput {
        draft_hash_hex: sha256_hex(canonical_json.as_bytes()),
        canonical_json,
    })
}

fn evaluate_heartbeat(input: HeartbeatInput) -> Result<HeartbeatOutput, String> {
    validate_identity(input.schema_version, &input.plugin_id)?;
    if input.host_method != PLAN_HEARTBEAT_METHOD {
        return Err("invalid_heartbeat_method".to_string());
    }
    validate_utc(&input.observed_at_utc, "observed_at_utc")?;
    if input.allowed_topics.is_empty() || input.allowed_topics.len() > 16 {
        return Err("allowed_topics must contain 1..16 items".to_string());
    }
    if input
        .allowed_topics
        .iter()
        .any(|topic| topic.is_empty() || topic.len() > 64)
    {
        return Err("allowed_topics contains an invalid item".to_string());
    }
    if input.home.suggested_actions.len() > 32 {
        return Err("suggested_actions exceeds its limit".to_string());
    }
    if input.feed.len() > 25 {
        return Err("feed exceeds its page limit".to_string());
    }

    let mut candidates = Vec::new();
    for post in &input.feed {
        if post.post_id.is_empty()
            || post.post_id.len() > 256
            || post.title.is_empty()
            || post.title.len() > 300
            || post.author_name.is_empty()
            || post.author_name.len() > 128
            || post.submolt_name.is_empty()
            || post.submolt_name.len() > 128
            || post.comment_count > 1_000_000_000
            || post.score < -1_000_000_000
            || post.score > 1_000_000_000
        {
            return Err("feed contains an invalid post".to_string());
        }
        validate_utc(&post.created_at_utc, "feed.created_at_utc")?;
        if post.is_verified && !post.is_spam && candidates.len() < 5 {
            candidates.push(post.post_id.clone());
        }
    }

    let (priority, reason) = if input.home.unread_notification_count > 0 {
        (
            "review_activity",
            "Unread activity on the connected Moltbook account has priority.",
        )
    } else if !candidates.is_empty() {
        (
            "inspect_feed",
            "Verified non-spam feed candidates are available for review.",
        )
    } else {
        (
            "idle",
            "No unread activity or eligible feed candidate requires attention.",
        )
    };
    let canonical = CanonicalHeartbeatPlan {
        schema_version: ABI_SCHEMA_VERSION,
        plugin_id: PLUGIN_ID.to_string(),
        contract_kind: HEARTBEAT_CONTRACT_KIND.to_string(),
        observed_at_utc: input.observed_at_utc,
        priority: priority.to_string(),
        reason: reason.to_string(),
        candidate_post_ids: candidates,
        publish_allowed: false,
        human_review_required: true,
        safety_flags: vec![
            "remote_content_untrusted".to_string(),
            "no_external_effect".to_string(),
        ],
    };
    let canonical_json = serde_json::to_string(&canonical).map_err(|error| error.to_string())?;
    Ok(HeartbeatOutput {
        plan_hash_hex: sha256_hex(canonical_json.as_bytes()),
        canonical_json,
    })
}

fn validate_identity(schema_version: u32, plugin_id: &str) -> Result<(), String> {
    if schema_version != ABI_SCHEMA_VERSION {
        return Err("invalid_schema_version: expected 1".to_string());
    }
    if plugin_id.trim() != PLUGIN_ID {
        return Err("invalid_plugin_id: unsupported plugin id".to_string());
    }
    Ok(())
}

fn validate_utc(value: &str, field: &str) -> Result<(), String> {
    if value.len() < 20 || value.len() > 40 || !value.ends_with('Z') || !value.contains('T') {
        return Err(format!("{field} must be canonical UTC"));
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn rejected(code: &str, message: &str) -> Vec<u8> {
    serde_json::to_vec(&AbiEnvelope {
        schema_version: ABI_SCHEMA_VERSION,
        status: "rejected".to_string(),
        result: None,
        error_code: Some(code.to_string()),
        error_message: Some(message.to_string()),
    })
    .unwrap_or_default()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> DraftInput {
        DraftInput {
            schema_version: 1,
            plugin_id: PLUGIN_ID.to_string(),
            bulletin_id: "release-v1.0.3-test14".to_string(),
            release_tag: "v1.0.3-test14".to_string(),
            category: "release".to_string(),
            facts: vec![
                "Hivra is a local-first runtime for user-owned Capsules.".to_string(),
                "The release includes bounded WASM plugin execution.".to_string(),
            ],
            title_hint: "A local-first runtime for user-owned Capsules".to_string(),
            audience: "agent-developers".to_string(),
        }
    }

    #[test]
    fn draft_is_deterministic_and_requires_approval() {
        let raw = serde_json::to_string(&input()).expect("input serializes");
        let first = evaluate_abi_json(&raw);
        let second = evaluate_abi_json(&raw);
        assert_eq!(first, second);
        let envelope: AbiEnvelope = serde_json::from_slice(&first).expect("envelope parses");
        let output: DraftOutput =
            serde_json::from_value(envelope.result.expect("draft output")).expect("draft parses");
        assert_eq!(envelope.status, "executed");
        assert_eq!(output.draft_hash_hex.len(), 64);
        assert!(output.canonical_json.contains("approval_required"));
    }

    #[test]
    fn rejects_public_crypto_promotion() {
        let mut value = input();
        value.facts = vec!["Buy this crypto token for profit.".to_string()];
        assert!(evaluate_draft(value)
            .expect_err("unsafe draft must reject")
            .contains("unsafe_public_content"));
    }

    #[test]
    fn heartbeat_prioritizes_activity_without_external_effects() {
        let input = HeartbeatInput {
            schema_version: 1,
            plugin_id: PLUGIN_ID.to_string(),
            host_method: PLAN_HEARTBEAT_METHOD.to_string(),
            observed_at_utc: "2026-07-29T10:00:00.000Z".to_string(),
            allowed_topics: vec!["hivra-development".to_string()],
            home: HeartbeatHomeInput {
                unread_notification_count: 2,
                suggested_actions: vec!["Read replies".to_string()],
            },
            feed: vec![HeartbeatFeedPostInput {
                post_id: "post-1".to_string(),
                title: "Reliable effects".to_string(),
                author_name: "Agent".to_string(),
                submolt_name: "general".to_string(),
                score: 3,
                comment_count: 1,
                is_verified: true,
                is_spam: false,
                created_at_utc: "2026-07-29T09:59:00.000Z".to_string(),
            }],
        };
        let raw = serde_json::to_string(&input).expect("input serializes");
        let first = evaluate_abi_json(&raw);
        let second = evaluate_abi_json(&raw);
        assert_eq!(first, second);
        let envelope: AbiEnvelope = serde_json::from_slice(&first).expect("envelope parses");
        let output: HeartbeatOutput =
            serde_json::from_value(envelope.result.expect("plan output")).expect("plan parses");
        assert_eq!(output.plan_hash_hex.len(), 64);
        assert!(output
            .canonical_json
            .contains("\"priority\":\"review_activity\""));
        assert!(output.canonical_json.contains("\"publish_allowed\":false"));
    }
}
