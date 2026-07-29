use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const PLUGIN_ID: &str = "hivra.contract.moltbook-ambassador.v1";
const DRAFT_CONTRACT_KIND: &str = "moltbook_ambassador_draft";
const HEARTBEAT_CONTRACT_KIND: &str = "moltbook_ambassador_heartbeat_plan";
const ENGAGEMENT_CONTRACT_KIND: &str = "moltbook_ambassador_engagement_plan";
const PREPARE_DRAFT_METHOD: &str = "prepare_moltbook_draft";
const PLAN_HEARTBEAT_METHOD: &str = "plan_moltbook_heartbeat";
const PLAN_ENGAGEMENT_METHOD: &str = "plan_moltbook_engagement";
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
    activity_on_own_posts: Vec<HeartbeatActivityInput>,
    suggested_actions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct HeartbeatActivityInput {
    post_id: String,
    post_title: String,
    submolt_name: String,
    new_notification_count: u32,
    latest_at_utc: String,
    latest_commenters: Vec<String>,
    preview: String,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct EngagementPostInput {
    post_id: String,
    title: String,
    content: String,
    author_name: String,
    submolt_name: String,
    score: i64,
    is_verified: bool,
    is_spam: bool,
    is_locked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct EngagementCommentInput {
    comment_id: String,
    parent_comment_id: Option<String>,
    content: String,
    author_name: String,
    score: i64,
    created_at_utc: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct EngagementInput {
    schema_version: u32,
    plugin_id: String,
    host_method: String,
    observed_at_utc: String,
    selection_kind: String,
    allowed_topics: Vec<String>,
    post: EngagementPostInput,
    comments: Vec<EngagementCommentInput>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CanonicalEngagementPlan {
    schema_version: u32,
    plugin_id: String,
    contract_kind: String,
    observed_at_utc: String,
    action_class: String,
    target_post_id: String,
    target_comment_id: Option<String>,
    reason: String,
    publish_allowed: bool,
    human_review_required: bool,
    safety_flags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct EngagementOutput {
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
        PLAN_ENGAGEMENT_METHOD => {
            let input = serde_json::from_value::<EngagementInput>(value)
                .map_err(|error| format!("invalid_engagement_input: {error}"))?;
            serde_json::to_value(evaluate_engagement(input)?).map_err(|error| error.to_string())?
        }
        _ => return Err("unsupported_method: unknown host_method".to_string()),
    };
    Ok(output)
}

fn evaluate_engagement(input: EngagementInput) -> Result<EngagementOutput, String> {
    validate_identity(input.schema_version, &input.plugin_id)?;
    if input.host_method != PLAN_ENGAGEMENT_METHOD {
        return Err("invalid_engagement_method".to_string());
    }
    validate_utc(&input.observed_at_utc, "observed_at_utc")?;
    if !matches!(
        input.selection_kind.as_str(),
        "own_activity" | "feed_candidate"
    ) {
        return Err("selection_kind is invalid".to_string());
    }
    if input.allowed_topics.is_empty() || input.allowed_topics.len() > 16 {
        return Err("allowed_topics must contain 1..16 items".to_string());
    }
    if input.allowed_topics.iter().any(|topic| {
        topic.is_empty()
            || topic.len() > 64
            || !topic
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    }) {
        return Err("allowed_topics contains an invalid item".to_string());
    }
    let post = &input.post;
    if post.post_id.is_empty()
        || post.post_id.len() > 256
        || post.title.is_empty()
        || post.title.len() > 300
        || post.content.len() > 40_000
        || post.author_name.is_empty()
        || post.author_name.len() > 128
        || post.submolt_name.is_empty()
        || post.submolt_name.len() > 128
        || post.score < -1_000_000_000
        || post.score > 1_000_000_000
    {
        return Err("engagement post is invalid".to_string());
    }
    if input.comments.len() > 20 {
        return Err("engagement comments exceed their limit".to_string());
    }
    let mut comment_ids = Vec::new();
    for comment in &input.comments {
        if comment.comment_id.is_empty()
            || comment.comment_id.len() > 256
            || comment
                .parent_comment_id
                .as_ref()
                .is_some_and(|id| id.is_empty() || id.len() > 256)
            || comment.content.is_empty()
            || comment.content.len() > 12_000
            || comment.author_name.is_empty()
            || comment.author_name.len() > 128
            || comment.score < -1_000_000_000
            || comment.score > 1_000_000_000
            || comment_ids.contains(&comment.comment_id)
        {
            return Err("engagement comments contain an invalid item".to_string());
        }
        validate_utc(&comment.created_at_utc, "comment.created_at_utc")?;
        comment_ids.push(comment.comment_id.clone());
    }

    let lowered =
        format!("{} {} {}", post.title, post.content, post.submolt_name).to_ascii_lowercase();
    let topic_match = input
        .allowed_topics
        .iter()
        .map(|topic| topic.to_ascii_lowercase())
        .any(|topic| lowered.contains(&topic) || lowered.contains(&topic.replace('-', " ")));
    let newest_comment = input
        .comments
        .iter()
        .max_by(|left, right| left.created_at_utc.cmp(&right.created_at_utc));

    let (action_class, target_comment_id, reason) = if post.is_spam || !post.is_verified {
        (
            "no_action",
            None,
            "Unverified or spam-marked remote content is not eligible.",
        )
    } else if post.is_locked {
        ("no_action", None, "The selected Moltbook post is locked.")
    } else if input.selection_kind == "own_activity" {
        match newest_comment {
            Some(comment) => (
                "reply_draft",
                Some(comment.comment_id.clone()),
                "New activity on an owned post is eligible for a reviewed reply draft.",
            ),
            None => (
                "no_action",
                None,
                "No bounded comment is available to answer.",
            ),
        }
    } else if topic_match {
        (
            "comment_draft",
            None,
            "Verified feed content matches the local allowed-topic policy.",
        )
    } else if post.score >= 5 {
        (
            "upvote_candidate",
            None,
            "Verified non-spam content has positive community evidence but no topic match.",
        )
    } else {
        (
            "no_action",
            None,
            "The selected post has insufficient bounded evidence for engagement.",
        )
    };

    let canonical = CanonicalEngagementPlan {
        schema_version: ABI_SCHEMA_VERSION,
        plugin_id: PLUGIN_ID.to_string(),
        contract_kind: ENGAGEMENT_CONTRACT_KIND.to_string(),
        observed_at_utc: input.observed_at_utc,
        action_class: action_class.to_string(),
        target_post_id: post.post_id.clone(),
        target_comment_id,
        reason: reason.to_string(),
        publish_allowed: false,
        human_review_required: true,
        safety_flags: vec![
            "remote_content_untrusted".to_string(),
            "no_external_effect".to_string(),
            "ai_text_not_generated".to_string(),
            "follow_requires_longitudinal_evidence".to_string(),
        ],
    };
    let canonical_json = serde_json::to_string(&canonical).map_err(|error| error.to_string())?;
    Ok(EngagementOutput {
        plan_hash_hex: sha256_hex(canonical_json.as_bytes()),
        canonical_json,
    })
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
    if input.home.activity_on_own_posts.len() > 32 {
        return Err("activity_on_own_posts exceeds its limit".to_string());
    }
    if input.feed.len() > 25 {
        return Err("feed exceeds its page limit".to_string());
    }

    let mut activity_candidates = Vec::new();
    for activity in &input.home.activity_on_own_posts {
        if activity.post_id.is_empty()
            || activity.post_id.len() > 256
            || activity.post_title.is_empty()
            || activity.post_title.len() > 300
            || activity.submolt_name.is_empty()
            || activity.submolt_name.len() > 128
            || activity.new_notification_count == 0
            || activity.new_notification_count > 1_000_000_000
            || activity.latest_commenters.len() > 32
            || activity
                .latest_commenters
                .iter()
                .any(|name| name.is_empty() || name.len() > 128)
            || activity.preview.len() > 2_000
        {
            return Err("activity_on_own_posts contains an invalid item".to_string());
        }
        validate_utc(&activity.latest_at_utc, "activity.latest_at_utc")?;
        if !activity_candidates.contains(&activity.post_id) && activity_candidates.len() < 5 {
            activity_candidates.push(activity.post_id.clone());
        }
    }

    let mut feed_candidates = Vec::new();
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
        if post.is_verified && !post.is_spam && feed_candidates.len() < 5 {
            feed_candidates.push(post.post_id.clone());
        }
    }

    let (priority, reason, candidates) = if input.home.unread_notification_count > 0 {
        (
            "review_activity",
            "Unread activity on the connected Moltbook account has priority.",
            activity_candidates,
        )
    } else if !feed_candidates.is_empty() {
        (
            "inspect_feed",
            "Verified non-spam feed candidates are available for review.",
            feed_candidates,
        )
    } else {
        (
            "idle",
            "No unread activity or eligible feed candidate requires attention.",
            Vec::new(),
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
                activity_on_own_posts: vec![HeartbeatActivityInput {
                    post_id: "own-post-1".to_string(),
                    post_title: "Hivra update".to_string(),
                    submolt_name: "general".to_string(),
                    new_notification_count: 2,
                    latest_at_utc: "2026-07-29T09:58:00.000Z".to_string(),
                    latest_commenters: vec!["Reader".to_string()],
                    preview: "Reader replied".to_string(),
                }],
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
        assert!(output
            .canonical_json
            .contains("\"candidate_post_ids\":[\"own-post-1\"]"));
        assert!(output.canonical_json.contains("\"publish_allowed\":false"));
    }

    #[test]
    fn engagement_proposes_reply_without_external_effect() {
        let input = EngagementInput {
            schema_version: 1,
            plugin_id: PLUGIN_ID.to_string(),
            host_method: PLAN_ENGAGEMENT_METHOD.to_string(),
            observed_at_utc: "2026-07-29T10:00:00.000Z".to_string(),
            selection_kind: "own_activity".to_string(),
            allowed_topics: vec!["hivra-development".to_string()],
            post: EngagementPostInput {
                post_id: "own-post-1".to_string(),
                title: "Hivra development".to_string(),
                content: "A bounded runtime update.".to_string(),
                author_name: "HivraAmbassador".to_string(),
                submolt_name: "general".to_string(),
                score: 2,
                is_verified: true,
                is_spam: false,
                is_locked: false,
            },
            comments: vec![EngagementCommentInput {
                comment_id: "comment-1".to_string(),
                parent_comment_id: None,
                content: "How does the runtime stay local-first?".to_string(),
                author_name: "Reader".to_string(),
                score: 1,
                created_at_utc: "2026-07-29T09:59:00.000Z".to_string(),
            }],
        };
        let raw = serde_json::to_string(&input).expect("input serializes");
        let first = evaluate_abi_json(&raw);
        let second = evaluate_abi_json(&raw);
        assert_eq!(first, second);
        let envelope: AbiEnvelope = serde_json::from_slice(&first).expect("envelope parses");
        let output: EngagementOutput =
            serde_json::from_value(envelope.result.expect("plan output")).expect("plan parses");
        assert!(output
            .canonical_json
            .contains("\"action_class\":\"reply_draft\""));
        assert!(output
            .canonical_json
            .contains("\"target_comment_id\":\"comment-1\""));
        assert!(output.canonical_json.contains("\"publish_allowed\":false"));
    }
}
