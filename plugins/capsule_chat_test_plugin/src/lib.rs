pub const PLUGIN_ID: &str = "hivra.contract.capsule-chat.v1";
pub const CONTRACT_KIND: &str = "capsule_chat";

#[no_mangle]
pub extern "C" fn hivra_plugin_abi_version() -> u32 {
    1
}

#[no_mangle]
pub extern "C" fn hivra_plugin_contract_id() -> u32 {
    3
}

#[no_mangle]
pub extern "C" fn hivra_entry_v1() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_matches_manifest_contract() {
        assert_eq!(PLUGIN_ID, "hivra.contract.capsule-chat.v1");
        assert_eq!(CONTRACT_KIND, "capsule_chat");
    }

    #[test]
    fn abi_exports_are_stable() {
        assert_eq!(hivra_plugin_abi_version(), 1);
        assert_eq!(hivra_plugin_contract_id(), 3);
    }
}
