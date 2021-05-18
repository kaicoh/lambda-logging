use lambda_runtime::{handler_fn, Context, Error};
use serde_json::Value;
use simple_logger::SimpleLogger;

#[tokio::main]
async fn main() -> Result<(), Error> {
    SimpleLogger::from_env().init()?;

    let func = handler_fn(lambda);
    lambda_runtime::run(func).await?;

    Ok(())
}

async fn lambda(event: Value, _: Context) -> Result<(), Error> {
    log::info!("event: {}", event);
    Ok(())
}
