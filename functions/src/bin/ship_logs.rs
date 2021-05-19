use aws_lambda_events::event::cloudwatch_logs::{CloudwatchLogsData, CloudwatchLogsEvent as Event};
use flate2::read::GzDecoder;
use lambda_runtime::{handler_fn, Context, Error};
use simple_logger::SimpleLogger;
use std::io::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Error> {
    SimpleLogger::from_env().init()?;

    let func = handler_fn(lambda);
    lambda_runtime::run(func).await?;

    Ok(())
}

async fn lambda(event: Event, _: Context) -> Result<(), Error> {
    log::info!("event: {:?}", event);

    if let Some(data) = event.aws_logs.data {
        let bytes = base64::decode(data)?;
        let mut decoder = GzDecoder::new(&bytes[..]);
        let mut json = String::new();
        decoder.read_to_string(&mut json)?;

        let CloudwatchLogsData {
            log_group,
            log_stream,
            log_events,
            ..
        } = serde_json::from_str(&json)?;

        let log_group = log_group.unwrap();
        let log_stream = log_stream.unwrap();
        let events_len = log_events.len();

        functions::send_log_stream(&log_group, &log_stream, log_events);

        log::info!("Successfully processes {} log events.", events_len);
    }

    Ok(())
}
