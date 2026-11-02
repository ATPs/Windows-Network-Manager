//! Quick fixes: flush the DNS cache, renew a DHCP lease and restart an
//! adapter. Each runs a fixed PowerShell template after preview and
//! confirmation. Adapters owned by tunnel/proxy software (EasyTier, v2rayN,
//! ...) and system interfaces are refused here, not only in the UI.

use crate::{
    arrays, blocking, coexistence, consume_confirmation, err, err_with, field_bool, field_i64, field_string,
    get_network_overview, issue_confirmation, network_insights::{notice, notice_texts}, require_admin,
    run_powershell_json, valid_index, AppResult, AppState,
};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

const FLUSH_DNS_SCRIPT: &str =
    "Clear-DnsClientCache -ErrorAction Stop;[PSCustomObject]@{flushed=$true}|ConvertTo-Json -Compress";

const RENEW_DHCP_SCRIPT: &str = r#"$c=@(Get-CimInstance Win32_NetworkAdapterConfiguration -Filter ("InterfaceIndex="+[int]$InputData.interfaceIndex) -ErrorAction Stop)|Select-Object -First 1
if(-not $c){throw 'The adapter no longer exists.'}
if(-not $c.DHCPEnabled){throw 'DHCP is not enabled on this adapter.'}
$r=Invoke-CimMethod -InputObject $c -MethodName RenewDHCPLease -ErrorAction Stop
if([int]$r.ReturnValue -ne 0){throw "Windows could not renew the DHCP lease (code $($r.ReturnValue))."}
[PSCustomObject]@{renewed=$true}|ConvertTo-Json -Compress"#;

const RESTART_ADAPTER_SCRIPT: &str = r#"Get-NetAdapter -IncludeHidden -InterfaceIndex ([int]$InputData.interfaceIndex) -ErrorAction Stop|Restart-NetAdapter -Confirm:$false -ErrorAction Stop
$a=Get-NetAdapter -IncludeHidden -InterfaceIndex ([int]$InputData.interfaceIndex)
[PSCustomObject]@{restarted=$true;status=[string]$a.Status}|ConvertTo-Json -Compress"#;

const ACTIONS: &[&str] = &["flushDns", "renewDhcp", "restartAdapter"];

fn quick_fix_payload(input: &Value) -> AppResult<Value> {
    let action = input
        .get("action")
        .and_then(Value::as_str)
        .filter(|action| ACTIONS.contains(action))
        .ok_or_else(|| err("VALIDATION_ERROR", "Unsupported quick fix."))?;
    let index = if action == "flushDns" {
        Value::Null
    } else {
        json!(valid_index(input.get("interfaceIndex").unwrap_or(&Value::Null))?)
    };
    Ok(json!({"action":action,"interfaceIndex":index}))
}

/// Checks an adapter action against the live adapter and returns its notices.
pub fn check_adapter_fix(adapter: &Value, action: &str) -> AppResult<Vec<Value>> {
    let alias = field_string(adapter, "alias");
    if let Some(owner) = coexistence::owner_of(adapter) {
        return Err(err_with(
            "PROTECTED_ADAPTER",
            format!("{alias} is managed by {owner}. Restart or renew it from {owner} instead."),
            json!({"alias":alias,"owner":owner}),
        ));
    }
    if field_bool(adapter, "systemInterface") {
        return Err(err("VALIDATION_ERROR", "System interfaces cannot be changed."));
    }
    if field_string(adapter, "adminStatus").eq_ignore_ascii_case("down") {
        return Err(err("VALIDATION_ERROR", "The adapter is disabled. Enable it instead."));
    }
    if action == "renewDhcp" && adapter.get("dhcpEnabled").and_then(Value::as_bool) != Some(true) {
        return Err(err("VALIDATION_ERROR", "DHCP is not enabled on this adapter."));
    }
    let mut notices = vec![if action == "renewDhcp" {
        notice(
            "fix.renewDhcp.interrupt",
            json!({"alias":alias}),
            format!("{alias} briefly releases and renews its IPv4 address. Connections on it may drop."),
        )
    } else {
        notice(
            "fix.restart.interrupt",
            json!({"alias":alias}),
            format!("{alias} is disabled and enabled again. Connections on it drop for a few seconds."),
        )
    }];
    if field_bool(adapter, "defaultRoute") {
        notices.push(notice(
            "fix.defaultGateway",
            json!({"alias":alias}),
            "This adapter currently carries a default gateway; Internet access may pause.",
        ));
    }
    Ok(notices)
}

fn live_adapter(index: i64) -> AppResult<Value> {
    arrays(get_network_overview()?.get("interfaces"))
        .into_iter()
        .find(|item| field_i64(item, "interfaceIndex") == index)
        .ok_or_else(|| err("NOT_FOUND", "Interface no longer exists."))
}

/// Validate the payload against live state and return (adapter, notices).
fn prepare(payload: &Value) -> AppResult<(Value, Vec<Value>)> {
    let action = field_string(payload, "action");
    if action == "flushDns" {
        let notices = vec![notice(
            "fix.flushDns",
            json!({}),
            "Clears cached DNS answers on this PC. Names are looked up again on next use; no adapter or route changes.",
        )];
        return Ok((Value::Null, notices));
    }
    let adapter = live_adapter(field_i64(payload, "interfaceIndex"))?;
    let notices = check_adapter_fix(&adapter, &action)?;
    Ok((adapter, notices))
}

#[tauri::command]
pub async fn preview_quick_fix(app: AppHandle, input: Value) -> AppResult<Value> {
    blocking(move || {
        let state = app.state::<AppState>();
        let payload = quick_fix_payload(&input)?;
        let (adapter, notices) = prepare(&payload)?;
        let requires_admin = field_string(&payload, "action") != "flushDns";
        Ok(json!({
            "fix":payload,
            "adapter":adapter,
            "warnings":notice_texts(&notices),
            "notices":notices,
            "requiresAdmin":requires_admin,
            "confirmationToken":issue_confirmation(&state, "quick-fix", &payload)?
        }))
    })
    .await
}

#[tauri::command]
pub async fn apply_quick_fix(app: AppHandle, input: Value) -> AppResult<Value> {
    blocking(move || {
        let state = app.state::<AppState>();
        let payload = quick_fix_payload(input.get("fix").unwrap_or(&input))?;
        let token = input.get("confirmationToken").and_then(Value::as_str).unwrap_or("");
        consume_confirmation(&state, token, "quick-fix", &payload)?;
        let action = field_string(&payload, "action");
        if action != "flushDns" {
            require_admin()?;
        }
        // Re-check protection against the live adapter right before running.
        let (adapter, _) = prepare(&payload)?;
        let alias = field_string(&adapter, "alias");
        let (script, message) = match action.as_str() {
            "flushDns" => (FLUSH_DNS_SCRIPT, notice("fix.flushDns.done", json!({}), "DNS cache cleared.")),
            "renewDhcp" => (
                RENEW_DHCP_SCRIPT,
                notice("fix.renewDhcp.done", json!({"alias":alias}), format!("DHCP lease renewed on {alias}.")),
            ),
            _ => (
                RESTART_ADAPTER_SCRIPT,
                notice("fix.restart.done", json!({"alias":alias}), format!("{alias} was restarted.")),
            ),
        };
        let result = run_powershell_json(script, Some(&payload))?;
        Ok(json!({"result":result,"message":message["text"],"messageCode":message}))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter(extra: Value) -> Value {
        let mut base = json!({"interfaceIndex":5,"alias":"WLAN","adminStatus":"Up","dhcpEnabled":true,"defaultRoute":false,"systemInterface":false,"managedBy":null});
        for (key, value) in extra.as_object().unwrap() {
            base[key] = value.clone();
        }
        base
    }

    #[test]
    fn payloads_accept_only_known_actions() {
        assert_eq!(quick_fix_payload(&json!({"action":"flushDns","interfaceIndex":5})).unwrap()["interfaceIndex"], Value::Null);
        assert_eq!(quick_fix_payload(&json!({"action":"renewDhcp","interfaceIndex":"5"})).unwrap()["interfaceIndex"], 5);
        assert!(quick_fix_payload(&json!({"action":"renewDhcp"})).is_err());
        assert!(quick_fix_payload(&json!({"action":"resetWinsock"})).is_err());
        assert!(quick_fix_payload(&json!({"action":"restartAdapter","interfaceIndex":-1})).is_err());
    }

    #[test]
    fn protected_system_and_ineligible_adapters_are_refused() {
        let protected = check_adapter_fix(&adapter(json!({"alias":"et_6_0qmn","managedBy":"EasyTier"})), "restartAdapter");
        assert_eq!(protected.unwrap_err().code, "PROTECTED_ADAPTER");
        assert_eq!(
            check_adapter_fix(&adapter(json!({"managedBy":"v2rayN"})), "renewDhcp").unwrap_err().code,
            "PROTECTED_ADAPTER"
        );
        assert!(check_adapter_fix(&adapter(json!({"systemInterface":true})), "restartAdapter").is_err());
        assert!(check_adapter_fix(&adapter(json!({"adminStatus":"Down"})), "restartAdapter").is_err());
        assert!(check_adapter_fix(&adapter(json!({"dhcpEnabled":false})), "renewDhcp").is_err());
        assert!(check_adapter_fix(&adapter(json!({"dhcpEnabled":false})), "restartAdapter").is_ok());
        let notices = check_adapter_fix(&adapter(json!({"defaultRoute":true})), "renewDhcp").unwrap();
        assert_eq!(notices.len(), 2);
        assert_eq!(notices[1]["code"], "fix.defaultGateway");
    }
}
