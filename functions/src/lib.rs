use aws_lambda_events::event::cloudwatch_logs::CloudwatchLogsLogEvent as LogEvent;
use chrono::{DateTime, FixedOffset};
use lazy_static::lazy_static;
use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use std::env;
use std::io::prelude::*;
use std::io::BufWriter;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::str::FromStr;

lazy_static! {
    static ref LOG_STREAM_REGEX: Regex = Regex::new(r"^.+\[(?:([0-9a-zA-Z]+))\].+$").unwrap();
    static ref LOGSTASH_HOST: String =
        env::var("LOGSTASH_HOST").expect("env LOGSTASH_HOST is required");
    static ref LOGSTASH_PORT: String =
        env::var("LOGSTASH_PORT").expect("env LOGSTASH_PORT is required");
    static ref TOKEN: String = env::var("TOKEN").expect("env TOKEN is required");
}

const KEY_REQUEST_ID: &str = "request_id";
const KEY_LEVEL: &str = "level";
const KEY_MESSAGE: &str = "message";

#[derive(Debug, PartialEq, Serialize)]
enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl FromStr for LogLevel {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let level = match s.to_lowercase().as_str() {
            "info" => LogLevel::Info,
            "warn" => LogLevel::Warn,
            "error" => LogLevel::Error,
            _ => LogLevel::Debug,
        };
        Ok(level)
    }
}

#[derive(Debug, PartialEq, Serialize)]
struct LogMessage {
    level: LogLevel,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    fields: Option<Value>,
    #[serde(rename = "@timestamp")]
    timestamp: DateTime<FixedOffset>,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct Log<'a> {
    log_stream: &'a str,
    log_group: &'a str,
    function_name: &'a str,
    lambda_version: &'a str,
    #[serde(rename = "type")]
    kind: &'a str,
    token: &'a str,
    #[serde(flatten)]
    content: LogMessage,
}

pub fn send_log_stream(log_group: &str, log_stream: &str, log_events: Vec<LogEvent>) {
    let addr = format!("{}:{}", *LOGSTASH_HOST, *LOGSTASH_PORT);
    let addrs: Vec<SocketAddr> = addr.to_socket_addrs().unwrap().collect();

    let func_name = function_name(log_group);
    let lambda_ver = lambda_version(log_stream);

    match TcpStream::connect(&addrs[..]) {
        Ok(stream) => {
            log::debug!("Tcp connection succeeded");

            let mut writer = BufWriter::new(&stream);

            for event in log_events {
                if let Some(content) = log_message(event) {
                    let log = Log {
                        log_stream,
                        log_group,
                        function_name: func_name,
                        lambda_version: lambda_ver,
                        kind: "cloudwatch",
                        token: TOKEN.as_str(),
                        content,
                    };

                    let msg = format!("{}\n", serde_json::to_string(&log).unwrap());
                    writer.write_all(msg.as_bytes()).unwrap();
                }
            }

            writer.flush().unwrap();
        }
        Err(err) => {
            log::error!("Tcp connection failed: {}", err);
        }
    }
}

fn function_name(log_group: &str) -> &str {
    let tokens: Vec<&str> = log_group.split('/').collect();
    tokens[tokens.len() - 1]
}

fn lambda_version(log_stream: &str) -> &str {
    let captures = LOG_STREAM_REGEX.captures(log_stream).unwrap();
    captures.get(1).map_or("", |m| m.as_str())
}

fn system_message(message: &str) -> bool {
    message.starts_with("START RequestId")
        || message.starts_with("END RequestId")
        || message.starts_with("REPORT RequestId")
}

fn message_parts(message: &str) -> (&str, &str, &str) {
    let parts: Vec<&str> = message.split('\t').collect();
    (parts[0], parts[1], parts[2])
}

fn log_message(event: LogEvent) -> Option<LogMessage> {
    let message = event.message.unwrap();

    if system_message(&message) {
        return None;
    }

    let (timestamp, request_id, event) = message_parts(&message);
    let timestamp = DateTime::parse_from_rfc3339(timestamp).unwrap();

    let log_message = match serde_json::from_str::<Value>(event) {
        Ok(Value::Object(mut fields)) => {
            fields.insert(KEY_REQUEST_ID.into(), Value::String(request_id.into()));

            let level = match fields.get(KEY_LEVEL) {
                Some(Value::String(lvl)) => LogLevel::from_str(&lvl).unwrap(),
                _ => LogLevel::Debug,
            };
            let message = match fields.get(KEY_MESSAGE) {
                Some(Value::String(msg)) => msg.to_string(),
                _ => String::default(),
            };

            fields.remove(KEY_LEVEL);
            fields.remove(KEY_MESSAGE);

            LogMessage {
                level,
                message,
                fields: Some(Value::Object(fields)),
                timestamp,
            }
        }
        _ => LogMessage {
            level: LogLevel::Debug,
            message: event.into(),
            fields: None,
            timestamp,
        },
    };

    Some(log_message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_getting_function_name() {
        let log_group = "/aws/lambda/service-env-funcName";
        assert_eq!(function_name(log_group), "service-env-funcName");
    }

    #[test]
    fn test_lambda_version() {
        let log_stream = "2016/08/17/[76]afe5c000d5344c33b5d88be7a4c55816";
        assert_eq!(lambda_version(log_stream), "76");

        let log_stream = "2016/08/17/[LATEST]afe5c000d5344c33b5d88be7a4c55816";
        assert_eq!(lambda_version(log_stream), "LATEST");
    }

    #[test]
    fn test_system_message() {
        let message = "START RequestId: 67c005bb-641f-11e6-b35d-6b6c651a2f01 Version: 31\n";
        assert!(system_message(message));

        let message = "END RequestId: 5e665f81-641f-11e6-ab0f-b1affae60d28\n";
        assert!(system_message(message));

        let message = "REPORT RequestId: 5e665f81-641f-11e6-ab0f-b1affae60d28\tDuration: 1095.52 ms\tBilled Duration: 1100 ms \tMemory Size: 128 MB\tMax Memory Used: 32 MB\t\n";
        assert!(system_message(message));

        let message =
            "2017-04-26T10:41:09.023Z	db95c6da-2a6c-11e7-9550-c91b65931beb\tloading index.html...\n";
        assert!(!system_message(message));
    }

    #[test]
    fn test_message_parts() {
        let message =
            "2017-04-26T10:41:09.023Z	db95c6da-2a6c-11e7-9550-c91b65931beb\tloading index.html...\n";

        let (timestamp, request_id, event) = message_parts(message);
        assert_eq!(timestamp, "2017-04-26T10:41:09.023Z");
        assert_eq!(request_id, "db95c6da-2a6c-11e7-9550-c91b65931beb");
        assert_eq!(event, "loading index.html...\n");
    }

    #[test]
    fn test_log_message() {
        let event =
            event_factory("START RequestId: 67c005bb-641f-11e6-b35d-6b6c651a2f01 Version: 31\n");
        assert!(log_message(event).is_none());

        let event = event_factory("END RequestId: 5e665f81-641f-11e6-ab0f-b1affae60d28\n");
        assert!(log_message(event).is_none());

        let event = event_factory("REPORT RequestId: 5e665f81-641f-11e6-ab0f-b1affae60d28\tDuration: 1095.52 ms\tBilled Duration: 1100 ms \tMemory Size: 128 MB\tMax Memory Used: 32 MB\t\n");
        assert!(log_message(event).is_none());

        let event = event_factory(
            "2017-04-26T10:41:09.023Z	db95c6da-2a6c-11e7-9550-c91b65931beb\tloading index.html...\n",
        );
        assert_eq!(
            log_message(event),
            Some(LogMessage {
                level: LogLevel::Debug,
                message: "loading index.html...\n".into(),
                fields: None,
                timestamp: DateTime::parse_from_rfc3339("2017-04-26T10:41:09.023Z").unwrap()
            })
        );

        let data = r#"
            {
                "level": "error",
                "message": "This is a test",
                "other": "other value"
            }
        "#;
        let event = event_factory(&format!(
            "2017-04-26T10:41:09.023Z	db95c6da-2a6c-11e7-9550-c91b65931beb\t{}\n",
            data
        ));
        let expected_fields = serde_json::json!({
            "request_id": "db95c6da-2a6c-11e7-9550-c91b65931beb",
            "other": "other value"
        });
        assert_eq!(
            log_message(event),
            Some(LogMessage {
                level: LogLevel::Error,
                message: "This is a test".into(),
                fields: Some(expected_fields),
                timestamp: DateTime::parse_from_rfc3339("2017-04-26T10:41:09.023Z").unwrap()
            })
        );
    }

    fn event_factory(message: &str) -> LogEvent {
        LogEvent {
            id: None,
            timestamp: 1,
            message: Some(message.into()),
        }
    }
}
