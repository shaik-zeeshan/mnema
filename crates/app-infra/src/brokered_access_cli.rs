use std::{env, process};

use crate::brokered_access::{
    execute_default_broker_request, BrokerSearchRequest, BrokerTimelineRequest,
    BrokeredCaptureRequest,
};

pub fn run_and_exit(program_name: &'static str) {
    if let Err(error) = run(program_name) {
        eprintln!("{error}");
        process::exit(1);
    }
}

pub fn run(program_name: &'static str) -> Result<(), String> {
    let args = env::args().skip(1).collect::<Vec<_>>();

    match args.as_slice() {
        [command, subcommand] if command == "auth" && subcommand == "status" => {
            print_broker_request(program_name, BrokeredCaptureRequest::AuthStatus)
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
            print_broker_request(program_name, BrokeredCaptureRequest::Search(request))
        }
        [command, opaque_id] if command == "show-text" => print_broker_request(
            program_name,
            BrokeredCaptureRequest::ShowText {
                opaque_id: opaque_id.clone(),
            },
        ),
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
            print_broker_request(program_name, BrokeredCaptureRequest::Timeline(request))
        }
        [command, opaque_id] if command == "open-in-mnema" => print_broker_request(
            program_name,
            BrokeredCaptureRequest::OpenInMnema {
                opaque_id: opaque_id.clone(),
            },
        ),
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

fn print_broker_request(
    program_name: &'static str,
    request: BrokeredCaptureRequest,
) -> Result<(), String> {
    let response =
        execute_default_broker_request(program_name, request).map_err(|error| error.to_string())?;
    print_json(&response)
}

fn print_json(value: &impl serde::Serialize) -> Result<(), String> {
    let raw = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    println!("{raw}");
    Ok(())
}
