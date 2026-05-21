use std::{env, path::PathBuf, process};

use crate::brokered_access::{
    active_grants, auth_status_for_config, broker_search, broker_show_text, broker_timeline,
    default_app_config_dir, default_save_directory_from_config, load_grants, record_audit_event,
    scope_class, BrokerErrorResponse, BrokerGrant, BrokerSearchRequest, BrokerSearchResponse,
    BrokerShowTextResponse, BrokerTimelineRequest, BrokerTimelineResponse,
};

pub fn run_and_exit(program_name: &'static str) {
    if let Err(error) = run(program_name) {
        eprintln!("{error}");
        process::exit(1);
    }
}

pub fn run(program_name: &'static str) -> Result<(), String> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let config_dir = default_app_config_dir()
        .ok_or_else(|| "failed to resolve Mnema app config dir".to_string())?;

    match args.as_slice() {
        [command, subcommand] if command == "auth" && subcommand == "status" => {
            print_json(&auth_status_for_config(&config_dir).map_err(|error| error.to_string())?)
        }
        [command, rest @ ..] if command == "search" => {
            let query = option_value(rest, "--query")
                .ok_or_else(|| "search requires --query <text>".to_string())?;
            let request = BrokerSearchRequest {
                query,
                from: option_value(rest, "--from"),
                to: option_value(rest, "--to"),
                limit: option_value(rest, "--limit").and_then(|value| value.parse().ok()),
            };
            let (response, grants) = with_infra(&config_dir, |infra, grants| async move {
                broker_search(&infra, &grants, request).await
            })?;
            audit_result(
                &config_dir,
                &grants,
                program_name,
                "search",
                response
                    .as_ref()
                    .ok()
                    .map(|response: &BrokerSearchResponse| response.results.len() as u32)
                    .unwrap_or(0),
            )?;
            print_json_result(response)
        }
        [command, opaque_id] if command == "show-text" => {
            let opaque_id = opaque_id.clone();
            let (response, grants) = with_infra(&config_dir, |infra, grants| async move {
                broker_show_text(&infra, &grants, &opaque_id).await
            })?;
            audit_result(
                &config_dir,
                &grants,
                program_name,
                "show_text",
                response
                    .as_ref()
                    .ok()
                    .map(|_: &BrokerShowTextResponse| 1)
                    .unwrap_or(0),
            )?;
            print_json_result(response)
        }
        [command, rest @ ..] if command == "timeline" => {
            let from = option_value(rest, "--from")
                .ok_or_else(|| "timeline requires --from <ts>".to_string())?;
            let to = option_value(rest, "--to")
                .ok_or_else(|| "timeline requires --to <ts>".to_string())?;
            let request = BrokerTimelineRequest {
                from,
                to,
                limit: option_value(rest, "--limit").and_then(|value| value.parse().ok()),
            };
            let (response, grants) = with_infra(&config_dir, |infra, grants| async move {
                broker_timeline(&infra, &grants, request).await
            })?;
            audit_result(
                &config_dir,
                &grants,
                program_name,
                "timeline",
                response
                    .as_ref()
                    .ok()
                    .map(|response: &BrokerTimelineResponse| response.intervals.len() as u32)
                    .unwrap_or(0),
            )?;
            print_json_result(response)
        }
        [command, opaque_id] if command == "open-in-mnema" => {
            let opaque_id = opaque_id.clone();
            let (response, grants) = with_infra(&config_dir, |_infra, grants| async move {
                if grants.is_empty() {
                    return Ok(Err(BrokerErrorResponse::authorization_required()));
                }
                if crate::brokered_access::decode_opaque_id(&opaque_id).is_none() {
                    return Ok(Err(BrokerErrorResponse {
                        error: crate::brokered_access::BrokerAuthStatusKind::AuthorizationRequired,
                        message: "invalid opaque result id".to_string(),
                    }));
                }
                open_mnema_deep_link(&opaque_id)?;
                Ok(Ok(serde_json::json!({
                    "opened": true,
                    "opaqueId": opaque_id
                })))
            })?;
            audit_result(
                &config_dir,
                &grants,
                program_name,
                "open_in_mnema",
                response.as_ref().ok().map(|_| 1).unwrap_or(0),
            )?;
            print_json_result(response)
        }
        _ => {
            eprintln!(
                "usage: {program_name} auth status | search --query <text> [--limit n] | show-text <opaque-result-id> | timeline --from <ts> --to <ts> | open-in-mnema <opaque-result-id>"
            );
            process::exit(2);
        }
    }
}

fn option_value(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|window| window.first().is_some_and(|value| value == name))
        .and_then(|window| window.get(1))
        .cloned()
}

fn with_infra<T, Fut>(
    config_dir: &PathBuf,
    f: impl FnOnce(crate::AppInfra, Vec<crate::brokered_access::BrokerGrant>) -> Fut,
) -> Result<(Result<T, BrokerErrorResponse>, Vec<BrokerGrant>), String>
where
    T: serde::Serialize,
    Fut: std::future::Future<Output = crate::Result<Result<T, BrokerErrorResponse>>>,
{
    let grants = load_grants(config_dir).map_err(|error| error.to_string())?;
    let grants = active_grants(&grants, crate::brokered_access::now_unix_ms());
    if grants.is_empty() {
        return Ok((Err(BrokerErrorResponse::authorization_required()), grants));
    }
    let save_directory = default_save_directory_from_config(config_dir)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| {
            "failed to resolve Mnema saveDirectory from recording settings".to_string()
        })?;
    let runtime = tokio::runtime::Runtime::new().map_err(|error| error.to_string())?;
    runtime.block_on(async {
        let infra = crate::AppInfra::initialize(save_directory)
            .await
            .map_err(|error| error.to_string())?;
        let grants_for_response = grants.clone();
        f(infra, grants)
            .await
            .map(|result| (result, grants_for_response))
            .map_err(|error| error.to_string())
    })
}

fn audit_result(
    config_dir: &PathBuf,
    grants: &[BrokerGrant],
    tool_identity: &str,
    command_type: &str,
    result_count: u32,
) -> Result<(), String> {
    if grants.is_empty() {
        return Ok(());
    }
    record_audit_event(
        config_dir,
        tool_identity,
        command_type,
        result_count,
        scope_class(grants),
    )
    .map_err(|error| error.to_string())
}

fn print_json_result<T: serde::Serialize>(
    result: Result<T, BrokerErrorResponse>,
) -> Result<(), String> {
    match result {
        Ok(value) => print_json(&value),
        Err(error) => print_json(&error),
    }
}

fn print_json(value: &impl serde::Serialize) -> Result<(), String> {
    let raw = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    println!("{raw}");
    Ok(())
}

fn open_mnema_deep_link(opaque_id: &str) -> crate::Result<()> {
    let url = format!("mnema://open/{opaque_id}");
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(&url).status()?;
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &url])
            .status()?;
        Ok(())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(&url).status()?;
        Ok(())
    }
}
