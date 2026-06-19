use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const PLUGIN_ID: &str = "hivra.contract.capsule-chat.v1";
pub const CONTRACT_KIND: &str = "capsule_chat";
const ABI_SCHEMA_VERSION: u32 = 1;
const MAX_ABI_INPUT_BYTES: usize = 64 * 1024;
#[cfg(target_arch = "wasm32")]
const MAX_ABI_OUTPUT_BYTES: usize = 128 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ChatInput {
    schema_version: u32,
    plugin_id: String,
    peer_hex: String,
    client_message_id: String,
    message_text: String,
    created_at_utc: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CanonicalChat {
    schema_version: u32,
    plugin_id: String,
    contract_kind: String,
    peer_hex: String,
    client_message_id: String,
    message_text: String,
    created_at_utc: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ChatOutput {
    canonical_json: String,
    envelope_hash_hex: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AbiEnvelope {
    schema_version: u32,
    status: String,
    result: Option<ChatOutput>,
    error_code: Option<String>,
    error_message: Option<String>,
}

#[no_mangle]
pub extern "C" fn hivra_plugin_abi_version() -> u32 {
    2
}

#[no_mangle]
pub extern "C" fn hivra_plugin_contract_id() -> u32 {
    3
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
    if raw.len() > MAX_ABI_INPUT_BYTES {
        return rejected("input_too_large", "ABI input exceeds the size limit");
    }
    let result = serde_json::from_str::<ChatInput>(raw)
        .map_err(|error| format!("invalid_json: {error}"))
        .and_then(evaluate);
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

fn evaluate(input: ChatInput) -> Result<ChatOutput, String> {
    if input.schema_version != 1 {
        return Err("invalid_schema_version: expected 1".to_string());
    }
    if input.plugin_id.trim() != PLUGIN_ID {
        return Err("invalid_plugin_id: unsupported plugin id".to_string());
    }
    let peer = input.peer_hex.trim().to_lowercase();
    if peer.len() != 64 || !peer.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err("peer_hex must be a 64-char lowercase hex".to_string());
    }
    let message_id = input.client_message_id.trim();
    if message_id.is_empty() || message_id.len() > 128 {
        return Err("client_message_id must contain 1..128 chars".to_string());
    }
    let text = input.message_text.trim();
    if text.is_empty() || text.len() > 1024 {
        return Err("message_text must contain 1..1024 UTF-8 bytes".to_string());
    }
    let created = input.created_at_utc.trim();
    if created.len() < 20 || !created.ends_with('Z') || !created.contains('T') {
        return Err("created_at_utc must be ISO-8601 UTC instant".to_string());
    }
    let canonical = CanonicalChat {
        schema_version: 1,
        plugin_id: PLUGIN_ID.to_string(),
        contract_kind: "capsule_chat_direct".to_string(),
        peer_hex: peer,
        client_message_id: message_id.to_string(),
        message_text: text.to_string(),
        created_at_utc: created.to_string(),
    };
    let canonical_json =
        serde_json::to_string(&canonical).map_err(|error| error.to_string())?;
    Ok(ChatOutput {
        envelope_hash_hex: sha256_hex(canonical_json.as_bytes()),
        canonical_json,
    })
}

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

    fn input() -> ChatInput {
        ChatInput {
            schema_version: 1,
            plugin_id: PLUGIN_ID.to_string(),
            peer_hex: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .to_string(),
            client_message_id: "msg-1".to_string(),
            message_text: "hello".to_string(),
            created_at_utc: "2026-04-04T10:00:00Z".to_string(),
        }
    }

    #[test]
    fn produces_deterministic_semantic_envelope() {
        let raw = serde_json::to_string(&input()).expect("input serializes");
        let first = evaluate_abi_json(&raw);
        let second = evaluate_abi_json(&raw);
        assert_eq!(first, second);
        let envelope: AbiEnvelope = serde_json::from_slice(&first).expect("envelope parses");
        assert_eq!(envelope.status, "executed");
        assert_eq!(envelope.result.unwrap().envelope_hash_hex.len(), 64);
    }

    #[test]
    fn rejects_empty_message() {
        let mut value = input();
        value.message_text = " ".to_string();
        assert!(evaluate(value).is_err());
    }
}
